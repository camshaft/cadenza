//! The ML printer: [`Arenas`] -> pretty-printed ML text.
//!
//! It walks the arena emitting layout tokens into a [`Doc`] (the Oppen engine), so each construct
//! breaks across lines only when it exceeds the target width. Two concerns are kept separate:
//!
//! - **Parens** are precedence-driven and width-independent: a subexpression parenthesizes itself
//!   when its operator precedence is lower than the surrounding context (`parent_prec`). This is the
//!   same minimal-paren logic the single-line surface used; it never depends on the line width.
//! - **Line breaks** are the `Doc`'s job: within a fixed paren structure, boxes decide flat-vs-broken
//!   by width.
//!
//! Names print bare when they re-lex to exactly themselves and are not reserved; otherwise they are
//! backtick-quoted (`emit_name`), the lossless escape that lets a name like `let` or `+` round-trip
//! as a name rather than a keyword/operator.
//!
//! Layout is a pure function of (arena, width): no input whitespace is consulted, so the printer is
//! idempotent — `print(x) == print(read_ml(print(x)))`.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::doc::Doc;
use crate::lexer::Lexer;
use crate::literal;
use crate::token::{self, Kind, PREC_ARROW, PREC_AS, PREC_MEMBER, infix_glyph, infix_prec};

/// Indentation per box level (spaces). A layout choice, not a contract.
const INDENT: isize = 2;

/// The parent-precedence to print a subexpression at when a block-form there (`if`/`let`/`match`)
/// must parenthesize but an infix operator must NOT. Block forms parenthesize when `parent_prec > 0`;
/// the lowest infix precedence is 1 and infixes parenthesize only when `prec < parent_prec`, so a
/// value of 1 forces `(if …)`/`(let …)`/`(match …)` while leaving every infix chain bare. Used for an
/// `if` condition, so a nested conditional condition reads as `if (if …) then …`.
const PREC_KEYWORD: u8 = 1;

/// The parent-precedence that forces a bitwise-or `|` INFIX subexpression to parenthesize: one above
/// `|`'s infix binding power (`token::infix_prec("|")` = 7), so `prec(7) < parent_prec(8)` triggers the
/// paren. Used ONLY for a match/handle arm body headed by `|` — a bare `|` at the arm's own level
/// re-parses as the next arm's separator (`Parser::arm_bar_terminates`), so `=> (x | 8)` is mandatory.
/// (`PREC_KEYWORD` cannot do this — it wraps block forms but never an infix, whose lowest prec is 1.)
const PREC_PIPE_PAREN: u8 = 8;

/// Pretty-print `arenas` to ML text targeting `width` columns. This is the CANONICAL, RE-READABLE
/// surface: a name that would re-lex as something else is backtick-escaped, a `Rational` value leaf
/// (`1/4`) is quoted, a unit is spelled as its full `Unit.base(#name)` construction — everything the
/// reader needs to reconstruct the exact same tree. For rendering a value to a human (a calculator
/// result), use [`print_display`], which drops that ceremony.
pub fn print(arenas: &Arenas, width: usize) -> String {
    print_mode(arenas, width, false)
}

/// Pretty-print `arenas` for human DISPLAY (the spec's "typed-result-to-text" surface,
/// self-hosting-surface.md §Rendering A Result Is A Compiler-Exposed Display Conversion). Unlike
/// [`print`], the output is NOT required to re-read to the same tree, so it drops the round-trip
/// ceremony that makes a value ugly to read: a `Rational` prints bare (`1/4`, not `` `1/4` ``; `8/1`
/// as `8`), a quantity prints in its concise `<value> <unit>` surface (`1/4 meter/second`, not
/// `Qty.of(`1/4`, Unit.base(#meter) / Unit.base(#second))`), and the outer `(: value type)` type
/// annotation on a whole result is stripped (a calculator shows the value, not its type). Everything
/// else renders exactly as [`print`] does — same layout, same precedence, same width behavior.
pub fn print_display(arenas: &Arenas, width: usize) -> String {
    print_mode(arenas, width, true)
}

fn print_mode(arenas: &Arenas, width: usize, display: bool) -> String {
    let mut p = Printer {
        a: arenas,
        doc: Doc::new(),
        shadowed_ctors: shadowed_ctors(arenas),
        delimit_body: false,
        display,
        depth: 0,
        suppress_leading_docs: false,
        flush_match_arms: false,
        width,
    };
    // In display mode, an outer `(: value type)` result annotation is stripped — a rendered value
    // shows the value, not its type. Only at the ROOT (a nested ascription is a real program form).
    let root = match (display, p.a.as_form(arenas.root, ":")) {
        (true, Some(ann)) if ann.len() == 2 => ann[0],
        _ => arenas.root,
    };
    // A `do` at the ROOT is the program's top-level form sequence — print its forms BARE (blank-line
    // separated), not wrapped in `do { … }`. A nested `do` (reached via `expr`) keeps the block form.
    if let Some(forms) = p.a.as_form(root, "do")
        && !forms.is_empty()
    {
        let forms = forms.to_vec();
        p.print_root_forms(&forms);
    } else {
        p.expr(root, 0);
    }
    p.doc.render(width)
}

/// The default target width (100 columns) — moved to `cadenza-syntax-core`, re-exported so
/// `printer::DEFAULT_WIDTH` stays a stable path.
pub use cadenza_syntax_core::DEFAULT_WIDTH;

/// Pretty-print at the default width.
pub fn print_ml(arenas: &Arenas) -> String {
    print(arenas, DEFAULT_WIDTH)
}

struct Printer<'a> {
    a: &'a Arenas,
    doc: Doc,
    /// Compound-ctor names (`list`/`tuple`/`record`/`map`) that some binder in this tree shadows, so
    /// a NAME-headed occurrence of one is a user application to render as a call — never sugared to a
    /// literal. Computed once per print (a whole-tree scan); see [`shadowed_ctors`].
    shadowed_ctors: CtorSet,
    /// Set while printing a top-level form that is immediately FOLLOWED by another form whose first
    /// surface token is "open" (a non-keyword — a name/number/`(`/…). A function/`fn` body that is a
    /// PLAIN expression (parsed greedily at `expr(0)`) must then be PARENTHESIZED: without a delimiter
    /// its trailing token would fuse with the next form (`def f(n) = n + 1` then `f(9)` re-lexes `1 f`
    /// as the quantity literal `1 f`), and a `;` cannot help — the greedy body would just swallow it.
    /// The parens are the "genuine ambiguity" delimiter. Cleared otherwise, so the common keyword-led
    /// case (`… def …`, `… export …`) keeps the clean bare `= n + 1` body. See `body_after_eq`.
    delimit_body: bool,
    /// DISPLAY mode: render values for a human rather than for re-reading. Set by [`print_display`].
    /// When true a `Rational` value leaf prints bare (`1/4`), a quantity prints in its concise
    /// `<value> <unit>` surface, and a base unit prints as its bare name — none of which round-trips,
    /// but all of which reads better as a result. `false` is the canonical, re-readable printer.
    display: bool,
    /// Current nesting depth of the `expr` recursion — incremented on entry, decremented on exit. The
    /// ML printer is a MUTUALLY-RECURSIVE machine (`expr`→`list`→the shape helpers→`expr`), one native
    /// frame per level, and `print` runs on arenas from ANY source — a decoded binary AST in particular,
    /// which `codec::decode` accepts at ARBITRARY depth (no cap, unlike the reader's `MAX_NESTING_DEPTH`).
    /// A recursive walk overflowed the native stack (SIGABRT) on a deep tree. Rather than rewrite the
    /// whole mutually-recursive printer to an explicit stack (large + output-risky), guard the ONE
    /// recursion hub (`expr`) with [`MAX_PRINT_DEPTH`] and elide past it. See `expr`.
    depth: u32,
    /// Set by the `@`-annotation arm when it has ALREADY printed a documented def's leading `(doc …)`
    /// forms ABOVE the annotation (where the user wrote a `/// header` before `@test`). Consumed by
    /// [`print_def_docs`] so the def does NOT re-print them below the `@`. Without this, a doc carried
    /// INSIDE an annotated def (the reader's `carry_docs`) prints BETWEEN the annotation and the def
    /// (`@test` / `/// header` / `def …`), moving a section header below its annotation — the
    /// annotation-comment adjacency the frontend is touchy about (v-cad/v-cdz-tooling report). A
    /// one-shot flag (like `delimit_body`): set, print the form, taken+cleared at the def's doc site.
    suppress_leading_docs: bool,
    /// One-shot flag (operator seq-96/97): the NEXT `match`/`handle` printed is in STATEMENT/TAIL position
    /// — it starts its OWN line (a def/let/if/arm body-tail, a `do`/top-level statement, a `handle`/`host`
    /// `in`-body) rather than being BOUND inline to a preceding token (`def f = match …`, a call arg, an
    /// operand). A statement-position match aligns its `|` arms FLUSH with the `match` keyword's column
    /// (`cbox(0)`); a bound match keeps them INDENTED one level (`cbox(INDENT)`, the default). Set by each
    /// statement emitter right before it prints the block (guarded on the body actually being a
    /// `match`/`handle`), and taken+cleared by `print_match`/`print_handle` so it never leaks to a nested
    /// value-position match. Mirrors the `delimit_body` one-shot discipline.
    flush_match_arms: bool,
    /// The target column width the whole print targets — threaded to the sub-grammar printer for an
    /// embedded region (`json{ … }` / `toml{ … }`), which renders its own body at the same width.
    width: usize,
}

/// The `expr`-recursion depth ceiling for the ML printer. Set FAR above the reader's
/// [`crate::sexpr::MAX_NESTING_DEPTH`] (1024) so NO reader-parseable program is ever affected (every
/// such program nests ≤ 1024), while still bounding native-stack use below the ~tens-of-thousands of
/// frames that overflow a default worker stack. Past this depth the printer emits an elision (`…`)
/// instead of recursing — keeping the printer TOTAL on a pathological decode-only arena (which cannot
/// round-trip through the depth-capped reader anyway) rather than aborting the process.
const MAX_PRINT_DEPTH: u32 = 4096;

/// The four compound-value constructors that have a `{…}`/`(…)`/`[…]`/`#{…}` surface literal AND a
/// shadowable prelude-alias name. A NAME-headed form of one sugars to its literal only when the name
/// is unshadowed; the string-headed primitive (`("record" …)`) always sugars.
const CTOR_NAMES: [&str; 4] = ["list", "tuple", "record", "map"];

/// A tiny set of "which of the four ctor names are shadowed", by index into [`CTOR_NAMES`].
type CtorSet = [bool; 4];

fn ctor_index(name: &str) -> Option<usize> {
    CTOR_NAMES.iter().position(|&c| c == name)
}

/// Scan the whole tree for any binder that binds one of the four compound-ctor names, returning the
/// set of shadowed ctors. Over-approximate on purpose: a ctor name bound ANYWHERE in the tree
/// disables literal sugar for EVERY name-headed occurrence of it, tree-wide. That is coarser than
/// true lexical scope, but always SOUND — the only cost of a false "shadowed" verdict is printing the
/// (still round-tripping) call form `list(…)` instead of the `[…]` literal. Precise per-occurrence
/// scope tracking is unnecessary: shadowing a compound-ctor name is rare, and when it happens the
/// call form is the honest rendering everywhere in that tree.
///
/// Binders considered (mirroring the resolver's binding forms): a `let`'s binding names, a `fn`'s
/// parameters, a `def`'s name and parameters, a `module`'s name, and a `match` arm's pattern binders.
fn shadowed_ctors(a: &Arenas) -> CtorSet {
    let mut set = [false; 4];
    for id in (0..a.structure.len() as u32).map(StructId) {
        collect_binders_at(a, id, &mut set);
        if set == [true; 4] {
            break; // every ctor already shadowed — nothing more to learn
        }
    }
    set
}

/// Record any compound-ctor name bound by the binder form (if any) headed at `id`.
fn collect_binders_at(a: &Arenas, id: StructId, set: &mut CtorSet) {
    let Struct::List(items) = a.get(id) else {
        return;
    };
    let Some(&head) = items.first() else { return };
    let mark = |a: &Arenas, name_id: StructId, set: &mut CtorSet| {
        if let Some(n) = a.as_name(name_id)
            && let Some(i) = ctor_index(n)
        {
            set[i] = true;
        }
    };
    match a.as_name(head) {
        // (let ((name value) …) body): each binding's name is a binder.
        Some("let") if items.len() >= 2 => {
            if let Struct::List(binds) = a.get(items[1]) {
                for &b in binds {
                    if let Struct::List(pair) = a.get(b)
                        && let Some(&n) = pair.first()
                    {
                        mark(a, n, set);
                    }
                }
            }
        }
        // (fn (param…) body): each param is a binder (a plain name or a `(: name Type)`).
        Some("fn") if items.len() >= 2 => {
            if let Struct::List(params) = a.get(items[1]) {
                for &p in params {
                    mark(a, param_name(a, p), set);
                }
            }
        }
        // (def (name param…) … body) or (def name … value): the def name plus any params.
        Some("def") if items.len() >= 2 => match a.get(items[1]) {
            Struct::List(sig) => {
                for &s in sig {
                    mark(a, param_name(a, s), set);
                }
            }
            Struct::Atom(_) => mark(a, items[1], set),
        },
        // (module name …): the module name is a binder.
        Some("module") if items.len() >= 2 => mark(a, items[1], set),
        // (match scrut (pat body)…): each arm's pattern contributes its binders.
        Some("match") if items.len() >= 2 => {
            for &arm in &items[2..] {
                if let Struct::List(pair) = a.get(arm)
                    && let Some(&pat) = pair.first()
                {
                    collect_pattern_binders(a, pat, set);
                }
            }
        }
        _ => {}
    }
}

/// The bound name of a parameter binder: `(: name Type)` binds `name`, a bare atom binds itself.
fn param_name(a: &Arenas, p: StructId) -> StructId {
    if let Some(t) = a.as_form(p, ":")
        && t.len() == 2
    {
        t[0]
    } else {
        p
    }
}

/// Collect the variable binders a pattern introduces (a bare name binds; a constructor/tuple pattern
/// recurses into its sub-patterns; a guard's pattern is its first element). Literals bind nothing.
fn collect_pattern_binders(a: &Arenas, pat: StructId, set: &mut CtorSet) {
    match a.get(pat) {
        Struct::Atom(_) => {
            if let Some(n) = a.as_name(pat)
                && let Some(i) = ctor_index(n)
            {
                set[i] = true;
            }
        }
        Struct::List(items) => {
            // `(guard pat expr)` → the pattern is the first arg; `(Ctor sub…)`/`(tuple sub…)` → the
            // sub-patterns are the tail (the head is a ctor/dotted-ctor, not a binder).
            if a.as_name(items.first().copied().unwrap_or(pat)) == Some("guard") && items.len() == 3
            {
                collect_pattern_binders(a, items[1], set);
            } else {
                for &sub in items.iter().skip(1) {
                    collect_pattern_binders(a, sub, set);
                }
            }
        }
    }
}

impl<'a> Printer<'a> {
    /// Print occurrence `id` in a context whose surrounding precedence is `parent_prec`.
    fn expr(&mut self, id: StructId, parent_prec: u8) {
        match self.a.get(id) {
            Struct::Atom(l) => {
                let leaf = self.a.leaf(*l).clone();
                self.leaf(&leaf);
            }
            Struct::List(items) => {
                // A native RATIONAL value `(RationalTag <num-int> <den-int>)` (seq-204) renders as the
                // mathematical `num/den` (slash, NO space) — the operator's seq-204 ruling ("stick with 3/2
                // with no space", dropped the `r` glyph, which collided with the unit-suffix). DISPLAY mode
                // drops an integral `/1` (`8/1` → `8`, the REPL/notebook value surface); CANONICAL keeps it.
                // The sign stays on the numerator. There is NO ML rational LITERAL (unspaced `3/2` is Int64
                // division), so `3/2` is a value-render form only, not a source round-trip — a rational
                // reaches ML source via `(/ n d)`-style construction. NO `Rational.of` resugar, NO `(: …
                // Rational)` ascription (operator's "native value, no sugar/desugar"). Rendered here at the
                // list level because the tag is a payloadless head (FieldPair/Member); children are Int atoms.
                if let Some((num, den)) = self.a.rational_parts(id) {
                    if self.display && self.a.as_int_usize(den) == Some(1) {
                        self.expr(num, 0);
                        return;
                    }
                    self.expr(num, 0);
                    self.doc.word("/");
                    self.expr(den, 0);
                    return;
                }
                // Depth guard: `expr` is the printer's single recursion hub (every mutually-recursive
                // shape helper reaches a child through it). Only a `List` descends, so guard HERE. Past
                // MAX_PRINT_DEPTH (far above any reader-parseable nesting) emit an elision instead of
                // recursing — keeps the printer total on a pathological decode-only arena rather than
                // overflowing the native stack. Atoms never recurse, so they are always rendered.
                if self.depth >= MAX_PRINT_DEPTH {
                    self.doc.word("…");
                    return;
                }
                self.depth += 1;
                let items = items.clone();
                self.list(&items, parent_prec);
                self.depth -= 1;
            }
        }
    }

    fn leaf(&mut self, leaf: &Leaf) {
        match leaf {
            Leaf::Int { value, radix } => self.doc.word(literal::render_int(value, *radix)),
            Leaf::Float(d) => self.doc.word(literal::render_decimal(d)),
            // Non-finite float VALUES render `nan`/`inf`/`-inf` (value display). Produced only by
            // `Ast.encode` of a computed float, never by the reader, so — like a `Rational` value's
            // display form — a round-tripping source literal is deferred to a separate surface slice.
            Leaf::FloatNan => self.doc.word("nan"),
            Leaf::FloatInf { negative } => self.doc.word(if *negative { "-inf" } else { "inf" }),
            Leaf::Bool(b) => self.doc.word(if *b { "true" } else { "false" }),
            Leaf::Str(s) => self.doc.word(format!("\"{}\"", literal::escape_string(s))),
            Leaf::Bytes(b) => self.doc.word(format!("b\"{}\"", literal::escape_bytes(b))),
            // A symbol renders `#name` (the unquoted sugar) when its content is a bare identifier, else
            // `#"…"` (reusing the string escape set). Both re-read via the ML lexer's `#` paths to the same
            // `Leaf::Sym`, so the round-trip is preserved either way.
            Leaf::Sym(s) if sym_is_bare_safe(s) => self.doc.word(format!("#{s}")),
            Leaf::Sym(s) => self.doc.word(format!("#\"{}\"", literal::escape_string(s))),
            Leaf::Name(n) => self.doc.word(emit_name(n)),
            // A bad-escape MARKER round-trips back to `"\<c>"` so the printed form re-reads to the same
            // marker (the defect survives the round-trip rather than being silently lost).
            Leaf::BadEscape(c) => self.doc.word(format!("\"\\{c}\"")),
            // A char renders `#\…`; a bad-char MARKER round-trips to `#\<text>`. Both re-read (via the
            // ML lexer's `#\` path) to the same leaf.
            Leaf::Char(c) => self.doc.word(literal::render_char(*c)),
            Leaf::BadChar(s) => self.doc.word(format!("#\\{s}")),
            // A TYPE-SUFFIXED literal renders `<body><suffix>` (`100N`, `0.5R`) — re-reads (via the ML
            // lexer's glued-suffix scan) to the same leaf.
            Leaf::Suffixed { value, kind } => self.doc.word(literal::render_suffixed(value, *kind)),
            // Native compound HEAD leaves (M2) are LIST heads, resugared to their ML surface at the list
            // level; a bare atom occurrence (not expected in a well-formed tree) renders a best-effort
            // marker so the printer stays total.
            Leaf::Ctor(c) => self.doc.word(crate::sexpr::compound_ctor_word(*c)),
            Leaf::FieldPair => self.doc.word("="),
            Leaf::Member => self.doc.word("."),
            // A BARE native-rational TAG leaf (not the head of a well-formed `(RationalTag num den)` node —
            // that list form renders `num/den` in `expr`). A stray tag has no operands, so it falls back to
            // the marker word `#rational`.
            Leaf::Rational => self.doc.word("#rational"),
        }
    }

    fn list(&mut self, items: &[StructId], parent_prec: u8) {
        if items.is_empty() {
            // The reader never produces an empty list; render defensively as the raw-list escape.
            self.doc.word("#[]");
            return;
        }
        // A compound-value literal renders back to its surface form (`(… "list" …)` → `[…]`, `tuple`
        // → `(a, b)`, `record` → `{…}`, `map` → `#{…}`), the round-trip inverse of the reader's literal
        // desugar. The head is the STRING primitive (`("record" …)`, always a literal) OR the NAME alias
        // (`(record …)`) when it is NOT shadowed by a binder in this tree — a shadowed name is a user
        // application, rendered as a call. Checked before the name-head keyword/operator dispatch below.
        if let Some(ctor) = self.literal_ctor(items[0]) {
            let args = &items[1..];
            // A NON-last `(comment-after …)` element (only from a decoded/synthetic AST) has no faithful
            // inline rendering — decline the sugared surface so it falls to the generic call form, which
            // round-trips. `print_record`/`print_map` also self-guard via `is_record_shape`/`is_map_shape`.
            let inline_ok = !self.has_nonlast_comment_after(args);
            match ctor.as_str() {
                "list" if inline_ok => return self.print_list_literal(args),
                "tuple" if !args.is_empty() && inline_ok => return self.print_tuple(args),
                "record" if self.is_record_shape(args) => return self.print_record(args),
                "map" if self.is_map_shape(args) => return self.print_map(args),
                // Native `Leaf::Ctor(Set)` — elements are its direct children (the M2 native set ctor,
                // uniform with the others); renders back to `#(…)`. The legacy `((. Set of) (list …))`
                // member form is recognized separately below (dual-support during the corpus migration).
                // A set CONSTRUCTION spread (`#(..a, x)`) carries a flat `Name("..")` marker — render through
                // the rest-aware path (`.. a`), the twin of the list/map/record spread; else the plain path.
                "set" if inline_ok && self.has_rest_marker(args) => {
                    return self.bracketed_rest(
                        "#(",
                        ")",
                        false,
                        args,
                        |p, e| p.expr(e, 0),
                        |p, e| p.expr(e, 0),
                    );
                }
                "set" if inline_ok => return self.bracketed_comment_aware("#(", ")", false, args),
                _ => {}
            }
        }
        // DISPLAY mode: a quantity VALUE `(Qty.of <value> <unit>)` renders in its concise
        // `<value> <unit>` surface (`1/4 meter/second`), with the unit spelled bare (`Unit.base(#meter)`
        // → `meter`, a composite as its infix `meter/second`), and a DIMENSIONLESS quantity (`Unit.one`)
        // as just its value. This is the value-form analogue of `quantity_literal` — it accepts the
        // `Unit.base`/`Unit.one`/composite shapes a RESULT carries (not the `Unit.of` a source literal
        // uses), and never has to round-trip, so it needs no bare-safe/non-negative guards.
        if self.display
            && let Some((value, unit)) = self.display_quantity(items)
        {
            self.doc.ibox(0);
            self.expr(value, PREC_MEMBER);
            if let Some(unit) = unit {
                self.doc.word(" ");
                // The unit is a space-separated trailing token (no operand can fuse to its right in a
                // value form — a quantity only sits in comma/brace-delimited slots), so it needs no
                // OUTER parens: render at precedence 0 and let its own composition parenthesize
                // internally only where a looser operator sits under a tighter one (`meter/second^2`).
                self.display_unit(unit, 0);
            }
            self.doc.end();
            return;
        }
        // A quantity literal `(Qty.of <numlit> (Unit.of #"name"))` renders back to its concise surface
        // `<num> name` — the inverse of the parser's `maybe_quantity_literal`. Binds tightest (like a
        // literal), so a following `.member`/`(args)`/infix operand needs no parens. Checked before the
        // name-head dispatch since the head is the member-access LIST `(. Qty of)`, not a name.
        if let Some((num, name)) = self.quantity_literal(items) {
            self.doc.ibox(0);
            self.expr(num, PREC_MEMBER);
            self.doc.word(" ");
            self.doc.word(name);
            self.doc.end();
            return;
        }
        // A unit conversion `(Unit.in (Unit.of #"name") value)` renders back to `value as name` — the
        // inverse of the parser's `as_conversion` — when the target is a bare-name family unit. It binds
        // at `PREC_AS`: the whole expression parenthesizes when the surrounding context binds tighter,
        // and the value (a left operand) is printed at `PREC_AS` so a looser operator inside it (a
        // pipeline/ascription/arrow) parenthesizes while tighter arithmetic does not. A COMPOUND or
        // computed target has no bare-name surface, so it falls through to the `Unit.in(target, value)`
        // call form — a faithful round-trip either way, exactly as `quantity_literal` falls back.
        if let Some((value, name)) = self.unit_conversion(items) {
            let paren = PREC_AS < parent_prec;
            self.doc.ibox(INDENT);
            if paren {
                self.doc.word("(");
            }
            self.expr(value, PREC_AS);
            // A NON-breaking space before `as` (not `space()`): the `as` operator's parser declines to
            // consume a leading `as` across a NEWLINE (the sequencing guard — a new-line `as` must not
            // reach back to the previous statement), so a break here emits `… )⏎  as unit`, which then
            // FAILS to re-parse ("keyword used outside its form"). Keeping ` as` glued to the value's last
            // line makes `<value> as <unit>` round-trip at every width (the value itself still wraps
            // internally). Fixes the chained-conversion ML round-trip failure (18-units-of-measure).
            self.doc.word(" as ");
            self.doc.word(name);
            if paren {
                self.doc.word(")");
            }
            self.doc.end();
            return;
        }
        // A set literal `((. Set of) (list …))` renders back to `#(…)` — the inverse of the parser's
        // `set_literal`. Checked before the name-head dispatch since the head is the member-access LIST
        // `(. Set of)`, not a name. Like the quantity/unit sugars, `Set` needs no shadow guard (the
        // member access re-reads identically); the inner list IS shadow-gated via `literal_ctor`.
        if let Some(elems) = self.set_literal(items) {
            return self.bracketed_comment_aware("#(", ")", false, &elems);
        }
        // A head that is an Atom(Name) may name a construct or an operator; otherwise it is a
        // computed-callee application.
        let head = self.head_name(items[0]);
        let args = &items[1..];

        if let Some(head) = head {
            // ---- type-suffix resugar: `(: <suffixed> BigInt|Rational)` -> the bare `100N`/`0.5R` ----
            // The reader desugars a type suffix to this annotation; print just the suffixed atom (the
            // suffix carries the type). A bare `(: 100 BigInt)` value-output (plain `Int` child) is NOT
            // matched and still prints as an explicit annotation.
            if head == ":"
                && args.len() == 2
                && let Struct::Atom(l) = self.a.get(args[0])
                && matches!(self.a.leaf(*l), Leaf::Suffixed { .. })
            {
                return self.expr(args[0], parent_prec);
            }
            // NOTE (seq-204): NO legacy `(: <n/d> Rational)` → `Rational.of(n, d)` resugar. A rational is
            // now the native `(RationalTag num den)` node, rendered `num/den` at the list level (see `expr`);
            // there is no `Name("n/d")` value form to resugar. (The operator's "native value, no
            // sugar/desugar" — the `Rational.of` resugar was the pre-native workaround for a Name-leaf.)
            // ---- function type `(-> A B)` -> `A -> B` (right-associative) ----
            if head == "->" && args.len() == 2 {
                return self.arrow(args[0], args[1], parent_prec);
            }
            // ---- prefix unary minus `(- e)` -> `-e` ----
            // The arity-1 subtraction is negation (the parser's prefix `-<expr>` desugar; `lower` reads
            // it as a type-directed negation). Render the operand at PREC_MEMBER so it stays tight — a
            // compound operand (`-(x + 1)`, `-(a - b)`) re-wraps itself, while a name / call / member
            // chain prints bare (`-x`, `-(f x)` → `-f(x)`, `-x.field`). A `-` glued to a numeric literal
            // is a SIGNED LITERAL, never this form, so `-e` never abuts a digit ambiguously. Checked
            // before the binary-infix arm (which requires exactly 2 args, so it never matched arity-1).
            if head == "-" && args.len() == 1 {
                self.doc.ibox(0);
                self.doc.word("-");
                self.expr(args[0], PREC_MEMBER);
                self.doc.end();
                return;
            }
            // ---- infix binary operator ----
            if let Some(prec) = infix_prec(&head)
                && args.len() == 2
            {
                return self.infix(&head, prec, args[0], args[1], parent_prec);
            }
            // ---- member access `(. obj key)` -> obj.key ----
            // The key is a plain field NAME (`obj.field`) or a non-negative INTEGER index
            // (`obj.0` — positional tuple access). A numeric key must not abut an operand that ends in
            // a decimal digit, or the concatenation re-lexes as a float (`5.0`, `x.0.1`); such an
            // operand is wrapped in explicit parens (`(5).0`, `(x.0).1`) so it re-reads correctly.
            if head == "."
                && args.len() == 2
                && let Some(key) = self.member_key(args[1])
            {
                let numeric = key.bytes().next().is_some_and(|b| b.is_ascii_digit());
                let wrap = numeric && self.ends_in_decimal_digit(args[0]);
                self.doc.ibox(0);
                if wrap {
                    self.doc.word("(");
                    self.expr(args[0], 0);
                    self.doc.word(")");
                } else {
                    self.expr(args[0], PREC_MEMBER);
                }
                self.doc.word(".");
                self.doc.word(key);
                self.doc.end();
                return;
            }
            // ---- quasiquote / unquote sigils ----
            // ---- first-class embedded syntax `(embedded #grammar <subtree>)` -> `grammar{ <text> }` ----
            // The reader parses a `json{ … }` / `toml{ … }` region by handing the body to the sub-grammar's
            // OWN reader and grafting its arena under `(embedded #<grammar> <subtree>)`. Re-emit that
            // SURFACE by dispatching to the sub-grammar's OWN printer on the grafted subtree, so a
            // `json{ … }` round-trips (and `cdz fmt`) as `json{ … }` — NOT as the generic application
            // `embedded(#json, json-object(…))` a fall-through render would produce, which is not the
            // readable surface (a print-fidelity bug the structural round-trip misses, since the generic
            // form re-parses to the same tree). Only the reserved grammars have a printer; an unknown tag
            // (should not occur — the reader only grafts reserved tags) falls through to the generic render.
            if head == "embedded"
                && args.len() == 2
                && let Some(grammar) = self.a.as_sym(args[0])
                && self.print_embedded(grammar, args[1])
            {
                return;
            }
            if head == "quasiquote" && args.len() == 1 {
                self.doc.word("`{ ");
                self.expr(args[0], 0);
                self.doc.word(" }");
                return;
            }
            if (head == "unquote" || head == "unquote-splicing") && args.len() == 1 {
                let sigil = if head == "unquote" { "," } else { ",@" };
                return self.unquote(sigil, args[0], parent_prec);
            }
            // ---- annotation `(@ name form)` -> `@name` on its OWN line, above the form ----
            // The general-purpose annotation sigil (inline-never/inline-always today, and whatever the
            // language grows). `args = [name, form]`. The annotation prints on the line ABOVE its target
            // (the Rust `#[attr]\nfn …` convention — an attribute reads as a modifier of the item below,
            // not part of its first line), via a `hardbreak` inside a consistent box, exactly like a
            // `// comment` above a node (`print_comment`). A nested `(@ …)` recurses here, so stacked
            // `@a @b def …` prints one annotation per line.
            //
            // The name slot is EITHER a bare name (`@test`) OR a call-style application `(tag "slow")`
            // for a parameterized annotation (`@tag("slow")` — the argument surface). The bare case
            // prints `@name` via `emit_name`; the application case renders the application itself
            // (`tag("slow")`) after the `@`, so `(@ (tag "slow") form)` round-trips to `@tag("slow")`.
            if head == "@" && args.len() == 2 {
                // An annotation `@name` prints on its OWN line above the form. That is only safe at a
                // STATEMENT / body position (`parent_prec == PREC_SEQ`), where the following surface token
                // is a fresh statement. In any OPERAND position (`parent_prec > PREC_SEQ` — an infix/
                // ascription operand, a `match` scrutinee, …) the trailing operator would bind to the
                // annotated form's LAST line rather than the whole `(@ …)`: `(: (@ test (if a b c)) T)`
                // printed `@test\n if … c : T`, which re-reads as `(@ test (if a b (: c T)))` (the `: T`
                // swallowed by the `if`'s else-branch) — a round-trip BREAK. Parenthesize the whole
                // annotation in operand position so `(@test\n form)` is one self-delimiting unit and the
                // enclosing operator binds to it. (A call ARG already wraps in `(`/`)`, so it round-trips
                // at prec 0 there; only the bare-operand positions need this.)
                let paren = parent_prec > crate::token::PREC_SEQ;
                if let Some(name) = self.a.as_name(args[0]) {
                    self.doc.cbox(0);
                    if paren {
                        self.doc.word("(");
                    }
                    // A `/// header` the user wrote ABOVE the annotation was carried INSIDE the def by
                    // the reader (`carry_docs`) — print it back ABOVE the `@name`, not between the
                    // annotation and the def. Sets `suppress_leading_docs` so the def skips re-printing.
                    self.hoist_annotated_docs(args[1]);
                    self.doc.word("@");
                    self.doc.word(emit_name(name));
                    self.doc.hardbreak();
                    self.annotated_form(args[1]);
                    if paren {
                        self.doc.word(")");
                    }
                    self.doc.end();
                    return;
                }
                // A parameterized annotation: the name slot is an application `(tag "slow")`. Render
                // `@` glued to that application (which prints as `tag("slow")`), so the reader re-reads
                // the glued `(` as the annotation argument. `PREC_MEMBER` keeps the application tight (a
                // bare call/member, never wrapped) so the `@`-glued form round-trips.
                if matches!(self.a.get(args[0]), Struct::List(_)) {
                    self.doc.cbox(0);
                    if paren {
                        self.doc.word("(");
                    }
                    self.hoist_annotated_docs(args[1]);
                    self.doc.word("@");
                    self.expr(args[0], PREC_MEMBER);
                    self.doc.hardbreak();
                    self.annotated_form(args[1]);
                    if paren {
                        self.doc.word(")");
                    }
                    self.doc.end();
                    return;
                }
            }
            // ---- pragma `(pragma key arg)` -> `@!key arg` (the inner-attribute sugar) ----
            // The pragma-directive sugar, the `@` twin: `@!default-float Float32` reads as a modifier of
            // the enclosing module (Rust's `#![…]`). Prints only for the CANONICAL two-argument shape (a
            // NAME key + one argument), so a malformed `(pragma …)` (no key / wrong arity) falls through to
            // the ordinary call rendering and its structure stays visible. The arg prints in the SAME line
            // as an argument (a type name / parenthesized type), NOT wrapped as a call — the `@!` sits at
            // the head of a module member, so no leading break.
            // The `@!param` module directive: `(pragma param (param <kv>…) (: name Type))` ->
            // `@!param(k: v, …) name : Type` (the operator's module-level `@param`). The config sub-node is
            // headed `param` (byte-similar to the `@param` annotation's inner `(param <kv>…)` app, the head
            // v-metaprogramming's scan reads); its kvs render as the glued `(...)` argument surface (each
            // `(: k v)` -> `k: v`), and the `(: name Type)` binder renders as `name : Type`. An EMPTY config
            // prints `@!param name : Type` (no `()`), the inverse of the parser accepting a missing config.
            // Guarded on the exact shape so a malformed `(pragma param …)` falls through to the generic call
            // render (structure stays visible).
            if head == "pragma"
                && args.len() == 3
                && self.a.as_name(args[0]) == Some("param")
                && self.a.as_form(args[1], "param").is_some()
                && self.a.as_form(args[2], ":").map(<[_]>::len) == Some(2)
            {
                // Delegate to a NON-INLINE helper: `expr` is the printer's recursive hub, so keeping its
                // per-frame locals minimal is what lets a MAX_NESTING_DEPTH-deep arena walk fit the test
                // thread's stack. Inlining this arm's locals (the config vec + kv loop) into `expr` grew the
                // frame enough to overflow the deep-flat-chain guard test (pr-sync reject). `#[inline(never)]`
                // moves them off `expr`'s frame.
                self.print_param_pragma(args[1], args[2]);
                return;
            }
            if head == "pragma"
                && args.len() == 2
                && let Some(key) = self.a.as_name(args[0])
            {
                self.doc.cbox(0);
                self.doc.word("@!");
                self.doc.word(emit_name(key));
                self.doc.word(" ");
                self.expr(args[1], parent_prec);
                self.doc.end();
                return;
            }
            // ---- keyword forms ----
            match head.as_str() {
                "let" if self.is_let_shape(args) => return self.print_let(args, parent_prec),
                "if" if args.len() == 3 => return self.print_if(args, parent_prec),
                "fn" if args.len() == 2 => return self.print_fn(args, parent_prec),
                "match" if self.is_match_shape(args) => return self.print_match(args, parent_prec),
                "def" if self.is_def_shape(args) => return self.print_def(args),
                "def" if self.is_value_def_shape(args) => return self.print_value_def(args),
                "do" if !args.is_empty() => return self.print_do(args),
                "type" if self.is_type_shape(args) => return self.print_type(args),
                "effect" if self.is_effect_shape(args) => return self.print_effect(args),
                "world" if self.is_world_shape(args) => return self.print_world(args),
                "handle" if self.is_handle_shape(args) => {
                    return self.print_handle(args, parent_prec);
                }
                "host" if self.is_host_shape(args) => return self.print_host(args, parent_prec),
                "module" if self.is_module_shape(args) => return self.print_module(args),
                "export" if self.is_export_shape(args) => return self.print_export(args),
                "import" if self.is_import_shape(args) || self.is_import_alias_shape(args) => {
                    return self.print_import(args);
                }
                // The compound-value literals (`list`/`tuple`/`record`/`map`) are STRING-headed now and
                // handled by the `head_ctor` dispatch above — a NAME head of the same spelling is an
                // ordinary application of the shadowable alias (or a user binding), rendered as a call.
                // A `(comment "text" node)` wraps a node in ANY position, so render it as `// text`
                // above the node wherever it appears. A `(doc …)`, by contrast, is only a `///`
                // line in a def/module BODY position (handled by print_def/print_module); a stray
                // `(doc …)` elsewhere falls through to the generic call form.
                "comment" if args.len() == 2 && self.is_string(args[0]) => {
                    return self.print_comment(args[0], args[1], parent_prec);
                }
                // A `(module-doc "text")` — a FILE/MODULE-level doc-comment (a leading `///` on a
                // non-documentable form, e.g. a file header before the first `import`). Unlike a
                // `(doc …)` (which lives INSIDE a def/module body), a module-doc is a STANDALONE
                // top-level/member form; it re-prints as its own `///` line so a file header round-trips
                // as documentation rather than being downgraded to `//`. Distinct 2-elem `(module-doc
                // "str")` shape; anything else falls through to the generic call form.
                "module-doc" if args.len() == 1 && self.is_string(args[0]) => {
                    self.print_doc(args[0]);
                    return;
                }
                // A binary literal `(bin <segment> …)` renders as `b[<segment>, …]` — the inverse of the
                // parser's `bin_literal`/`bin_pattern`. `bin` is a reserved grammar form (structurally
                // dispatched like `match`, never a shadowable value), so this always sugars, in both
                // expression and pattern position; the empty form `(bin)` prints as `b[]`.
                "bin" => return self.print_bin(args),
                "tagged-template" if self.is_tagged_template_shape(args) => {
                    return self.print_tagged_template(args);
                }
                // `(forall (a b) TYPE)` -> `forall a b. TYPE` — the explicit generic binder in type
                // position (the inverse of the parser's `forall_type`). The binder list is a nested list
                // of `Name` atoms; TYPE is any type. Only sugar when the shape matches (a binder LIST +
                // a body); otherwise a user's `forall(...)` application falls through to the call form.
                "forall" if self.is_forall_shape(args) => {
                    return self.print_forall(args, parent_prec);
                }
                _ => {}
            }
            // ---- generic call form: head(a, b, c) ----
            self.doc.word(emit_name(&head));
            self.call_args(args);
        } else {
            // computed-callee application: `(expr arg…)` -> expr(arg…)
            self.doc.ibox(0);
            self.expr(items[0], PREC_MEMBER);
            self.doc.end();
            self.call_args(args);
        }
    }

    /// A call's argument list. If the LAST argument is a block-like construct (a lambda or a
    /// `match`), it HUGS: the head args stay inline and only the trailing block breaks internally
    /// (`map(items, fn(x) => …)`), the highest-value readability case for higher-order calls.
    /// Otherwise it is all-or-nothing: inline if it fits, else one arg per line, block-indented,
    /// closing `)` on its own dedented line.
    fn call_args(&mut self, args: &[StructId]) {
        // A same-line trailing `//` on the LAST argument (`(comment-after "text" arg)`) needs the closing
        // `)` forced onto its own line (else `arg // text)` swallows the `)` into the comment) — the plain
        // comment-aware path handles that. A `(comment-after …)` arg is never huggable (its head is
        // `comment-after`, not `fn`/`match`), so `hug_call` never sees one. A NON-last `comment-after`
        // (only from a decoded AST) has no faithful `arg // text, …` rendering, so it is NOT routed here —
        // it falls through to the ordinary render, where it prints as a `comment-after(...)` call that
        // round-trips faithfully (same total-printer discipline as the collection literals, PR#763).
        if args.last().is_some_and(|&a| self.is_comment_after(a))
            && !self.has_nonlast_comment_after(args)
        {
            return self.plain_call_comment_aware(args);
        }
        if !args.is_empty() && self.is_huggable_arg(args[args.len() - 1]) {
            self.hug_call(args);
        } else {
            self.plain_call(args);
        }
    }

    /// A `plain_call` variant for when the LAST argument carries a same-line trailing `(comment-after
    /// "text" arg)`: render each arg (unwrapping the last's `comment-after` to `arg // text` same-line),
    /// then force a hard newline before `)` so the trailing comment ends its line and the `)` is not
    /// swallowed into it. Mirrors `bracketed_comment_aware` for the collection literals.
    fn plain_call_comment_aware(&mut self, args: &[StructId]) {
        self.doc.cbox(INDENT);
        self.doc.word("(");
        self.doc.zerobreak();
        for (i, &arg) in args.iter().enumerate() {
            if i > 0 {
                self.doc.word(",");
                self.doc.space();
            }
            self.print_elem_maybe_commented(arg);
        }
        // Hard newline before `)` so the last arg's trailing `// …` ends its line.
        self.doc.hardbreak_with(-INDENT);
        self.doc.word(")");
        self.doc.end();
    }

    /// All-or-nothing argument layout.
    fn plain_call(&mut self, args: &[StructId]) {
        self.doc.cbox(INDENT);
        self.doc.word("(");
        if !args.is_empty() {
            self.doc.zerobreak();
            for (i, &arg) in args.iter().enumerate() {
                if i > 0 {
                    self.doc.word(",");
                    self.doc.space();
                }
                self.expr(arg, 0);
            }
            // a zero-width break before `)` that, when the box breaks, dedents the `)` to the
            // call's own column (nothing when flat)
            self.doc.break_with(0, -INDENT);
        }
        self.doc.word(")");
        self.doc.end();
    }

    /// A `def`/`fn` PARAMETER list `(p, …)` with the SAME all-or-nothing layout as `plain_call`: inline
    /// when it fits, else the open `(` stays on the header line, EACH param drops to its own line indented
    /// one level, and the close `)` sits on its own line dedented back to the construct's column (operator
    /// seq-92 — no partial mid-param wrap). The caller has already printed the head (`def name` / `fn`).
    fn print_param_list(&mut self, params: &[StructId]) {
        self.doc.cbox(INDENT);
        self.doc.word("(");
        if !params.is_empty() {
            self.doc.zerobreak();
            for (i, &p) in params.iter().enumerate() {
                if i > 0 {
                    self.doc.word(",");
                    self.doc.space();
                }
                self.print_param(p);
            }
            self.doc.break_with(0, -INDENT);
        }
        self.doc.word(")");
        self.doc.end();
    }

    /// Last-arg-hugging layout: `head(a, b, <block>)` where the head args `a, b` stay on the line
    /// and `<block>` breaks internally. The head args live in their OWN box that does not contain
    /// the block, so the block's internal hardbreaks cannot force the head args apart; they are
    /// joined to the block with a literal `, ` that never breaks.
    fn hug_call(&mut self, args: &[StructId]) {
        let (head, last) = args.split_at(args.len() - 1);
        self.doc.cbox(0);
        self.doc.word("(");
        // head args, comma-space separated, never broken (a small ibox keeps them flat)
        self.doc.ibox(0);
        for (i, &arg) in head.iter().enumerate() {
            if i > 0 {
                self.doc.word(", ");
            }
            self.expr(arg, 0);
        }
        self.doc.end();
        if !head.is_empty() {
            self.doc.word(", ");
        }
        // the hugged block breaks internally
        self.expr(last[0], 0);
        self.doc.word(")");
        self.doc.end();
    }

    /// True if `id` is an argument worth hugging as the last argument of a call: a lambda or a
    /// `match` — a construct that lays itself out across lines and reads well trailing a call.
    fn is_huggable_arg(&self, id: StructId) -> bool {
        let head = match self.a.get(id) {
            Struct::List(items) => items.first().and_then(|&h| self.head_name(h)),
            _ => None,
        };
        match head.as_deref() {
            Some("fn") => matches!(self.a.get(id), Struct::List(i) if i.len() == 3),
            Some("match") => self.is_match_shape_form(id),
            _ => false,
        }
    }

    /// True if `id` is a well-formed `(match scrut arm…)` the match surface handles.
    fn is_match_shape_form(&self, id: StructId) -> bool {
        match self.a.get(id) {
            Struct::List(items) if items.len() >= 2 => self.is_match_shape(&items[1..]),
            _ => false,
        }
    }

    /// A left-associative infix chain at precedence `prec`. A run of same-precedence operators
    /// (`a + b - c`) is FLATTENED into one box so, if it overflows, the operators break at ONE
    /// consistent indent rather than compounding a level per nesting. The break sits BEFORE each
    /// operator (R10) so a wrapped operand lands with its operator leading the line.
    /// Whether `id` is a type-suffix desugar `(: <Suffixed> …)` — the node the resugar at `expr` collapses
    /// to the bare `100N`/`0.5R` literal. Mirrors that resugar's guard so the infix left-spine flatten
    /// leaves it whole (see the call site).
    fn is_suffix_desugar(&self, id: StructId) -> bool {
        if let Some(t) = self.a.as_form(id, ":")
            && t.len() == 2
            && let Struct::Atom(l) = self.a.get(t[0])
        {
            matches!(self.a.leaf(*l), Leaf::Suffixed { .. })
        } else {
            false
        }
    }

    fn infix(&mut self, op: &str, prec: u8, l: StructId, r: StructId, parent_prec: u8) {
        let paren = prec < parent_prec;
        // Collect the flat chain: descend the left spine while the operator has the SAME precedence.
        // Result is operands `[o0, o1, …]` and the operators `[op1, …]` between them.
        let mut operands = vec![r];
        let mut ops = vec![op.to_string()];
        let mut left = l;
        loop {
            // A type-suffix desugar `(: <Suffixed> BigInt|Rational)` is NOT an annotation to flatten — it
            // resugars to the bare literal (`100N`). If the left spine is this node (e.g. the inner form of
            // `(: 100N Int64)`, which the reader nests as `(: (: 100N BigInt) Int64)`), keep it WHOLE so
            // `expr` reaches the suffix resugar; flattening it would spuriously expose the internal
            // `: BigInt` (`100N : BigInt : Int64`).
            if self.is_suffix_desugar(left) {
                break;
            }
            match self.a.get(left) {
                Struct::List(items) if items.len() == 3 => {
                    if let Some(h) = self.head_name(items[0])
                        && infix_prec(&h) == Some(prec)
                    {
                        operands.push(items[2]);
                        ops.push(h);
                        left = items[1];
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }
        operands.push(left);
        operands.reverse(); // now left-to-right
        ops.reverse();

        self.doc.ibox(INDENT);
        if paren {
            self.doc.word("(");
        }
        // first operand (its left child, if any, already bound at this prec). `force_break` is set when
        // that operand carried a trailing `//` comment, so the break before the NEXT operator is HARD.
        let mut force_break = self.infix_operand(operands[0], prec);
        for (i, o) in ops.iter().enumerate() {
            // Peel LEADING `(comment …)` off the next operand — such a comment PRECEDED the operator in
            // source (`a\n  // note\n  op b`, an own-line comment/block between operands of a multi-line
            // chain), so it must print OWN-LINE BEFORE the operator (the reader re-drains an own-line
            // comment at the operator slot as the right operand's leading, so this keeps the round-trip
            // IDEMPOTENT; emitting it after the op — `op // note` — re-reads to a DROP). seq-277/C3.
            let mut operand = operands[i + 1];
            let mut op_leads: Vec<StructId> = Vec::new();
            while let Some(a) = self.a.as_form(operand, "comment")
                && a.len() == 2
                && self.is_string(a[0])
            {
                op_leads.push(a[0]);
                operand = a[1];
            }
            // A trailing `//` on the previous operand (force_break), or a leading comment on this operand,
            // forces a HARD break before the operator (a `//` runs to end-of-line, so a soft space could
            // keep the chain flat and swallow the ` op right` / the comment can't sit inline).
            if force_break || !op_leads.is_empty() {
                self.doc.hardbreak();
            } else {
                self.doc.space(); // break BEFORE the operator
            }
            for &text in &op_leads {
                self.doc.word(format!("//{}", self.doc_line_text(text)));
                self.doc.hardbreak();
            }
            // In infix position the operator prints as its SURFACE GLYPH (the arena head `=` for
            // equality prints as `==`; every other op is identity). The backtick escape is only for
            // an operator glyph used as an ordinary NAME.
            self.doc.word(infix_glyph(o).to_string());
            self.doc.word(" ");
            force_break = self.infix_operand(operand, prec + 1); // right binds one tighter (leads peeled)
        }
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// Print one operand of an infix chain, re-emitting a trailing `(comment-after "text" inner)` wrapper
    /// as `inner // text` (the seq-277/C3 mid-infix-chain trailing comment the reader attaches). Returns
    /// `true` when a trailing comment was emitted, so [`Self::infix`] forces a HARD break before the next
    /// operator (a `//` runs to end-of-line, so a soft space could keep the chain flat and swallow the
    /// following ` op right` into the comment). A plain operand prints via `expr` and returns `false`.
    fn infix_operand(&mut self, operand: StructId, prec: u8) -> bool {
        if let Some(a) = self.a.as_form(operand, "comment-after")
            && a.len() == 2
            && self.is_string(a[0])
        {
            self.expr(a[1], prec);
            self.doc.word(format!(" //{}", self.doc_line_text(a[0])));
            return true;
        }
        self.expr(operand, prec);
        false
    }

    /// The function type `(-> A B)` -> `A -> B`. RIGHT-associative: a chain `(-> A (-> B C))` prints as
    /// `A -> B -> C` by descending the RIGHT spine (the inverse of `infix`, which descends the left).
    /// The left operand of each arrow binds one tighter (`PREC_ARROW + 1`) so a nested arrow THERE
    /// parenthesizes (`(A -> B) -> C`), while the right operand stays at `PREC_ARROW` so the natural
    /// right nesting prints without parens. The whole type parenthesizes when the surrounding context
    /// binds tighter than the arrow (e.g. an arrow type used as an application argument).
    fn arrow(&mut self, l: StructId, r: StructId, parent_prec: u8) {
        let paren = PREC_ARROW < parent_prec;
        // Collect the flat right spine: `A -> B -> C` is operands `[A, B, C]`.
        let mut operands = vec![l];
        let mut right = r;
        loop {
            if let Struct::List(items) = self.a.get(right)
                && items.len() == 3
                && self.head_name(items[0]).as_deref() == Some("->")
            {
                operands.push(items[1]);
                right = items[2];
                continue;
            }
            break;
        }
        operands.push(right);

        self.doc.ibox(INDENT);
        if paren {
            self.doc.word("(");
        }
        for (i, &operand) in operands.iter().enumerate() {
            if i > 0 {
                self.doc.space(); // break BEFORE the arrow
                self.doc.word("-> ");
            }
            // Every operand but the last is a left operand of an arrow: bind one tighter so a nested
            // arrow there parenthesizes. The last is the final result, printed at the arrow's own prec.
            let operand_prec = if i + 1 < operands.len() {
                PREC_ARROW + 1
            } else {
                PREC_ARROW
            };
            self.expr(operand, operand_prec);
        }
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// `let n = e, … in body` — the binding(s), `in`, then the body on the next line. `in`
    /// self-delimits the `let`, so a `let` chain reads as flat `let x = e in` lines then the body.
    /// A `let` in a value position (`parent_prec > 0`) parenthesizes.
    fn print_let(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        self.doc.cbox(0);
        if paren {
            self.doc.word("(");
        }
        self.doc.word("let ");
        // The bindings arg may be wrapped in a TRAILING `(comment-after "text" binds)` — a `//` that
        // followed `in` on its source line (`let x = a in // note`). Peel it: the binds print normally,
        // and the captured text re-emits after ` in` below (so it round-trips; the body's own hardbreak
        // already drops the body to the next line, so the `//` can't swallow it).
        let in_trailing = self
            .a
            .as_form(args[0], "comment-after")
            .and_then(|a| (a.len() == 2 && self.is_string(a[0])).then_some(a[0]));
        let binds_arg = self.strip_comment_after(args[0]);
        // The bindings box is CONSISTENT: a multi-binding `let` that does not fit on one line drops
        // EVERY binding to its own line, indented under `let` — not a greedy fill that packs two
        // bindings on the first line and wraps the overflow (which reads as an accidental line break
        // mid-list). A single-binding `let` has no inter-binding break, so this is a no-op for it (the
        // common case); only a multi-binding `let` that overflows changes, and it changes for the
        // better. The value of each binding still breaks within its own nested boxes independently.
        self.doc.cbox(INDENT);
        if let Struct::List(binds) = self.a.get(binds_arg) {
            let binds = binds.clone();
            for (i, &raw) in binds.iter().enumerate() {
                if i > 0 {
                    self.doc.word(",");
                    self.doc.space();
                }
                // Peel LEADING own-line `(comment …)` wrapper(s): each prints as a `// …` line ABOVE the
                // binding (a hardbreak forces the bindings box to break, so the comment ends its line
                // before the binder). A LOOP handles multiple (decoded ASTs may nest). The remaining
                // TRAILING `(comment-after …)`, if any, prints after the value, same-line.
                let mut b = raw;
                while let Some(a) = self.a.as_form(b, "comment")
                    && a.len() == 2
                    && self.is_string(a[0])
                {
                    self.doc.word(format!("//{}", self.doc_line_text(a[0])));
                    self.doc.hardbreak();
                    b = a[1];
                }
                let trailing = self
                    .a
                    .as_form(b, "comment-after")
                    .and_then(|a| (a.len() == 2 && self.is_string(a[0])).then_some(a[0]));
                let b = self.strip_comment_after(b);
                if let Struct::List(pair) = self.a.get(b) {
                    let (n, e) = (pair[0], pair[1]);
                    // A binder is a plain NAME (`Atom`) or a destructuring PATTERN (`List` —
                    // `(tuple a b)` / `(list x .. rest)` / …). A pattern binder renders through the
                    // pattern surface (`(a, b)`, `[x, .. rest]`), the inverse of `let_expr` routing a
                    // pattern-opening binder to `pattern`; a plain name renders as an ordinary expr.
                    if matches!(self.a.get(n), Struct::List(_)) {
                        self.pattern(n);
                    } else {
                        self.expr(n, 0);
                    }
                    self.doc.word(" = ");
                    self.value(e);
                    if let Some(text) = trailing {
                        self.doc.word(format!(" //{}", self.doc_line_text(text)));
                    }
                }
            }
        }
        self.doc.end();
        self.doc.word(" in");
        // A captured trailing `//` after `in` re-emits here, same-line (`… in // note`). The body's
        // hardbreak below then drops the body to the next line, so the comment can't swallow it.
        if let Some(text) = in_trailing {
            self.doc.word(format!(" //{}", self.doc_line_text(text)));
        }
        // Body layout (operator seq68 → seq-86): a `let … in` chain is FLAT — every chained `let` AND
        // the final body drop to the SAME column (the chain's indent, offset 0 in this box), the ML idiom
        // for a pervasive `let … in`. The operator (seq-86) flagged the earlier per-`let` DEEPENING of the
        // final body ("why is the last statement indented") as weird — same class as the else-if ladder
        // flatten (seq69/70). So NO extra indent on the final body: `hardbreak()` uniformly, whether the
        // body is another `let` (chain continues) or the terminal expression.
        self.doc.hardbreak();
        // The `in`-body is on its own line, so a `match`/`handle` body flushes its arms (seq-96/97).
        self.flush_match_arms = self.head_is_match_form(self.a.peel_comments(args[1]));
        self.expr(args[1], 0);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// Print an expression in STATEMENT position — the trailing body of a `let` (or the root). Here a
    /// `(do …)` sequence prints BARE (`a;⏎b;⏎c`, no parens), because the enclosing construct's
    /// delimiter (or end-of-program) scopes it. This is the ONLY place a `do` is unparenthesized;
    /// everywhere a `do` is reached through `expr` (a value, a call arg, an `if` branch) it
    /// parenthesizes. A non-`do` statement is a plain expression.
    fn print_stmt(&mut self, id: StructId) {
        let seq = matches!(self.a.get(id), Struct::List(items)
            if items.len() > 1 && self.head_name(items[0]).as_deref() == Some("do"));
        if seq {
            let items: Vec<StructId> = match self.a.get(id) {
                Struct::List(items) => items[1..].to_vec(),
                _ => unreachable!(),
            };
            self.print_do_stmts(&items);
        } else {
            // A statement-position `match`/`handle` starts its own line → its arms flush (seq-96/97).
            self.flush_match_arms = self.head_is_match_form(self.a.peel_comments(id));
            self.expr(id, 0);
        }
    }

    /// `if c then t else e`. Inline when it fits; otherwise the condition stays on the `if` line and
    /// the branches break to indented lines under their `then`/`else` keywords:
    /// ```text
    /// if c then
    ///   t
    /// else
    ///   e
    /// ```
    /// The condition is printed at `PREC_KEYWORD` so a nested block-form condition (`if`/`let`/`match`)
    /// parenthesizes — otherwise `if if a then b else c then …` is ambiguous to read.
    fn print_if(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        self.doc.cbox(INDENT);
        if paren {
            self.doc.word("(");
        }
        // FLATTEN the else-if LADDER (operator seq69/seq70): render every `if`/`else if`/`else` HEADER at
        // the SAME outer indent inside ONE box, rather than recursing into `expr` for an `if`-shaped else
        // branch — which opened a NESTED `cbox(INDENT)` per rung, indenting a chained-if DEEPER on every
        // `else` (the operator's "20 levels deep" compiler-ml pain). Loop the ladder here: the else branch,
        // while it is a bare `(if cond then else)` (3-arg), continues the same box as `else if …`; the
        // final non-`if` else prints once. Each branch BODY stays indented one level under its header.
        self.doc.word("if ");
        let mut arms = args;
        loop {
            // Condition at PREC_KEYWORD so a nested block-form condition (`if`/`let`/`match`) parenthesizes
            // — `if (if a then b else c) then …` rather than the unreadable `if if a …`.
            self.expr(arms[0], PREC_KEYWORD);
            self.doc.word(" then");
            // Breakable space keeps `then t` on the line when it fits, else drops `t` to an indented line.
            self.doc.space();
            // A `match`/`handle` then-branch on its own line flushes its arms (seq-96/97).
            self.flush_match_arms = self.head_is_match_form(self.a.peel_comments(arms[1]));
            let then_had_trailing = self.expr_with_trailing_comment(arms[1], 0);
            // A same-line `//` trailing the then-branch runs to end-of-line, so `else` MUST drop to the next
            // line — a breakable space would collapse to ` else` INSIDE the comment (`then 1 // note else 2`
            // swallows the `else`). Force a hardbreak (dedented to the `if` column) when the then-branch
            // carried a trailing comment; otherwise the ordinary breakable space (dedent back to the column).
            if then_had_trailing {
                self.doc.hardbreak_with(-INDENT);
            } else {
                self.doc.break_with(1, -INDENT);
            }
            // A BARE `if`-shaped else branch (no comment/annotation wrapper) flattens: `else if …` in the
            // SAME box at this indent. Any other else (or a wrapped `if`) prints once and ends the ladder.
            match self.a.as_form(arms[2], "if") {
                Some(inner) if inner.len() == 3 => {
                    self.doc.word("else if ");
                    arms = inner;
                }
                _ => {
                    self.doc.word("else");
                    self.doc.space();
                    // A `match`/`handle` else-branch on its own line flushes its arms (seq-96/97).
                    self.flush_match_arms = self.head_is_match_form(self.a.peel_comments(arms[2]));
                    self.expr_with_trailing_comment(arms[2], 0);
                    break;
                }
            }
        }
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// Print an expression at `parent_prec`, re-emitting a same-line TRAILING `(comment-after "text"
    /// inner)` wrapper as `inner // text` (the wrapper is peeled, `inner` printed, then the comment
    /// appended same-line). Used where a sub-expression can carry a captured trailing comment that is NOT
    /// the last token of the whole form — an `if`'s then/else branch (`if a then 1 // note` before
    /// `else`), so the comment doesn't fall through to the generic `comment-after(...)` call render (which
    /// would break round-trip / trip the comment-drop guard). A node with no `comment-after` wrapper
    /// prints exactly as `expr` (no-op peel). Returns `true` if a trailing comment WAS emitted — the
    /// caller must then force a hardbreak before the next token (a `//` runs to end-of-line, so anything
    /// after it on the same line would be swallowed into the comment).
    fn expr_with_trailing_comment(&mut self, id: StructId, parent_prec: u8) -> bool {
        let trailing = self
            .a
            .as_form(id, "comment-after")
            .and_then(|a| (a.len() == 2 && self.is_string(a[0])).then_some(a[0]));
        let inner = self.strip_comment_after(id);
        self.expr(inner, parent_prec);
        if let Some(text) = trailing {
            self.doc.word(format!(" //{}", self.doc_line_text(text)));
            true
        } else {
            false
        }
    }

    /// A function BODY that is a top-level type ascription `(: inner R)` denotes a RETURN TYPE: it is
    /// the shape `def f(x) -> R = inner` and `fn(x) -> R => inner` desugar to. Returns `(inner, R)` so
    /// the printer can put the `-> R` back in signature position (round-tripping the surface form),
    /// leaving `inner` as the printed body. Any other body has no return type.
    fn return_type(&self, body: StructId) -> Option<(StructId, StructId)> {
        let t = self.a.as_form(body, ":")?;
        if t.len() == 2 {
            // A `(: <suffixed> BigInt|Rational)` is a SELF-TYPED literal — the `N`/`R` suffix already
            // carries the type, and the value-position resugar prints it bare (`100N`/`0.5R`), which
            // re-reads to the SAME `(: <suffixed> …)` ascription. Do NOT hoist it to a `-> R` return type:
            // hoisting drops the ascription and emits `-> R = 100N`, whose bare `100N` body RE-desugars to
            // `(: <suffixed> R)` on read — so the def/fn gains a return-type ascription that was NOT in the
            // source (a round-trip mismatch: `(def (f) 255N)` printed `def f() -> BigInt = 255N`, re-read as
            // a return-typed def). Leave it as the body; the suffix-resugar renders it. Mirrors the resugar
            // guard (`(: <Suffixed> …)`).
            if let Struct::Atom(l) = self.a.get(t[0])
                && matches!(self.a.leaf(*l), Leaf::Suffixed { .. })
            {
                return None;
            }
            Some((t[0], t[1]))
        } else {
            None
        }
    }

    /// The return-type arrow `-> R` in signature position (before `=`/`=>`), when the body carries one.
    fn print_return_type(&mut self, ret_ty: Option<StructId>) {
        if let Some(ty) = ret_ty {
            self.doc.word(" -> ");
            // Bind one tighter than the arrow so a function-typed return still reads as one arrow chain
            // (`-> Int64 -> Int64` is the curried result), not a parenthesized inner arrow.
            self.expr(ty, PREC_ARROW);
        }
    }

    /// `fn(p, …) => body`, or `fn(p, …) -> R => inner` when the body is a return-type ascription.
    fn print_fn(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        // A body that is a top-level ascription is a return type — hoist it to signature position.
        let (body, ret_ty) = match self.return_type(args[1]) {
            Some((inner, ty)) => (inner, Some(ty)),
            None => (args[1], None),
        };
        self.doc.cbox(0);
        if paren {
            self.doc.word("(");
        }
        self.doc.word("fn");
        if let Struct::List(params) = self.a.get(args[0]) {
            let params = params.clone();
            self.print_param_list(&params);
        } else {
            self.print_param_list(&[]);
        }
        self.print_return_type(ret_ty);
        self.doc.word(" =>");
        // A block-like body hugs the `=>` (breaks internally); a plain body drops to an indented
        // line if it overflows — same discipline as a def's `=` body. A `fn` body is a sequence-tail
        // position, so a `(do …)` body prints bare.
        self.body_after_eq(body, true);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// `def name(p, …) = body` — a named function definition (a hoisting declaration). `args` is
    /// `signature doc… body`: the signature list `(name p …)`, zero or more `(doc "…")` forms
    /// (printed as `/// …` lines above the `def`), and the single body. Never parenthesized.
    fn print_def(&mut self, args: &[StructId]) {
        let signature = args[0];
        let docs = &args[1..args.len() - 1];
        let raw_body = args[args.len() - 1];
        // A body that is a top-level ascription is a RETURN TYPE — hoist it to signature position as
        // `-> R`, leaving `inner` as the printed body (`def f(x) -> R = inner`).
        let (body, ret_ty) = match self.return_type(raw_body) {
            Some((inner, ty)) => (inner, Some(ty)),
            None => (raw_body, None),
        };
        // Offset 0: a hugged block body (match/let/…) carries its own indentation relative to its
        // own start column, so the def box must not add another level on top. A plain-expression
        // body that overflows is indented explicitly by `body_after_eq`.
        self.doc.cbox(0);
        self.print_def_docs(docs);
        self.doc.word("def ");
        if let Struct::List(sig) = self.a.get(signature) {
            let sig = sig.clone();
            // name
            self.expr(sig[0], 0);
            self.print_param_list(&sig[1..]);
            self.print_return_type(ret_ty);
            self.doc.word(" =");
        }
        // A function body is a sequence-tail position — a `(do …)` body prints bare.
        self.body_after_eq(body, true);
        self.doc.end();
    }

    /// `def name = value` — a value definition (a hoisting declaration, like the function form).
    /// `args` is `name doc… value`. Uses `def` (not `let`) because it hoists — `let` is sequential.
    fn print_value_def(&mut self, args: &[StructId]) {
        let name = args[0];
        let docs = &args[1..args.len() - 1];
        let value = args[args.len() - 1];
        self.doc.cbox(0);
        self.print_def_docs(docs);
        self.doc.word("def ");
        self.expr(name, 0);
        self.doc.word(" =");
        // A value def binds a single expression (a VALUE position) — a `(do …)` value parenthesizes.
        self.body_after_eq(value, false);
        self.doc.end();
    }

    /// Emit a def's leading `(doc "…")` forms as `/// …` lines, each followed by a hardbreak. Shared
    /// by the function and value def printers.
    fn print_def_docs(&mut self, docs: &[StructId]) {
        // The `@`-annotation arm may have ALREADY printed these leading docs ABOVE the annotation
        // (a `/// header` the user wrote before `@test`). One-shot flag: skip them here so they are
        // not re-printed between the annotation and the def. Cleared as it's consumed.
        if std::mem::take(&mut self.suppress_leading_docs) {
            return;
        }
        for &d in docs {
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
    }

    /// Print a parameter binder. A type-annotated binder `(: name Type)` prints as `name: Type`;
    /// a plain binder prints as itself. (Any other shape in parameter position — it shouldn't
    /// occur — falls back to the ordinary expression printer.)
    fn print_param(&mut self, p: StructId) {
        // A `(const BINDER)` wrapper — an EXPLICIT compile-time parameter. Emit the `const ` keyword prefix
        // then the inner binder (so `(const (: d T))` → `const d: T`, `(const d)` → `const d`). The ML
        // reader's `param` accepts a leading `const`; s-expr keeps the `(const …)` form.
        if let Some(t) = self.a.as_form(p, "const")
            && t.len() == 1
        {
            self.doc.word("const ");
            self.print_param(t[0]);
            return;
        }
        if let Some(t) = self.a.as_form(p, ":")
            && t.len() == 2
        {
            let (name, ty) = (t[0], t[1]);
            self.expr(name, 0);
            self.doc.word(": ");
            self.expr(ty, 0);
        } else {
            self.expr(p, 0);
        }
    }

    /// `(doc "text")` -> `/// text` (a documentation line). Verbatim text after the `///`.
    fn print_doc(&mut self, text: StructId) {
        self.doc.word(format!("///{}", self.doc_line_text(text)));
    }

    /// `(comment "text" node)` -> `// text` on its own line, then the annotated node beneath it. The
    /// comment wrapper is TRANSPARENT to precedence, so `node` is printed at the SAME `parent_prec` the
    /// wrapper was asked for — otherwise a comment-wrapped body that needed parenthesizing (a non-last
    /// match-arm body whose tail is an open `match`/`handle`, forced with `PREC_KEYWORD`) would print the
    /// inner form BARE, and the following `| pat` would be absorbed into it (a structural round-trip bug).
    fn print_comment(&mut self, text: StructId, node: StructId, parent_prec: u8) {
        self.doc.cbox(0);
        self.doc.word(format!("//{}", self.doc_line_text(text)));
        self.doc.hardbreak();
        self.expr(node, parent_prec);
        self.doc.end();
    }

    /// The text of a doc/comment string leaf, prefixed with a space when non-empty so it renders as
    /// `/// text` (and re-reads with the space stripped). An empty comment renders as bare `///`.
    fn doc_line_text(&self, text: StructId) -> String {
        match self.a.get(text) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Str(s) if s.is_empty() => String::new(),
                Leaf::Str(s) => format!(" {s}"),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    /// Emit ` body` after a `=`/`in`-style keyword. `seq_ok` is true in a SEQUENCE-TAIL position — a
    /// function/`fn` body, whose `;`-run is delimited by the next top-level form — where a `(do …)`
    /// body prints BARE (`= a;⏎b;⏎c`, no parens), the exact surface the parser folds back into that
    /// `(do …)`. It is false in a VALUE position (a value def's RHS), where a `(do …)` must PARENTHESIZE
    /// (`= (a; b)`) or the `;` would escape into the enclosing sequence.
    ///
    /// A block-like body (a `match`, `let`, `if`, … that manages its own multi-line layout) HUGS the
    /// `=` — a plain space keeps it on the line so it breaks internally (`fn f(x) = match … {` … ). A
    /// plain-expression body uses a breakable space so a long flat expression instead drops to an
    /// indented line (`fn f(x) =\n  a + b + …`).
    fn body_after_eq(&mut self, body: StructId, seq_ok: bool) {
        // DELIMIT: the next top-level form is "open" (would fuse onto this body's tail — see
        // `print_root_forms`), so close the body off in parens: ` = (body)`. Parens work for EVERY body
        // shape (a plain expr `(n + 1)`, a sequence `(a; b)`, a `match`/`let`/`if`), and a `)` cannot
        // fuse with the following name/number/`(`. Consume the flag so it does not leak into the body.
        if std::mem::take(&mut self.delimit_body) {
            self.doc.ibox(INDENT);
            self.doc.space();
            self.doc.word("(");
            // The body is a full sequence position inside the parens (`seq_ok` there is implicit — the
            // parens delimit it), so a `(do …)` renders as a `;`-run; any other body prints plainly.
            if let Some(stmts) = self.as_do_seq(body) {
                self.print_do_stmts(&stmts);
            } else {
                self.expr(body, 0);
            }
            self.doc.word(")");
            self.doc.end();
            return;
        }
        if seq_ok && let Some(stmts) = self.as_do_seq(body) {
            // A bare sequence body: ` =` then each statement on its own indented line, `;`-separated.
            // A consistent box so the interior hardbreaks (between statements) force the leading break
            // to fire too — the body drops under the `=` rather than sitting flat after it.
            self.doc.cbox(INDENT);
            self.doc.space();
            self.print_do_stmts(&stmts);
            self.doc.end();
        } else if self.is_block_body(body) {
            // Hug: a plain space keeps the block on the `=` line; it breaks internally at its own
            // indentation (the def box is at offset 0, so no extra level is added). This is a BOUND
            // position (the `match`/`handle` sits inline after `=`), so its arms stay INDENTED — clear
            // any stray `flush_match_arms` so it never inherits a caller's statement-position flush.
            self.doc.word(" ");
            self.flush_match_arms = false;
            self.expr(body, 0);
        } else {
            // Plain expression: a breakable space, and its own indented box so a long flat body
            // drops to an indented continuation line.
            self.doc.ibox(INDENT);
            self.doc.space();
            self.expr(body, 0);
            self.doc.end();
        }
    }

    /// If `id` is a `(do e1 e2 …)` sequence of at LEAST TWO elements, return the element list — the
    /// shape that prints as a bare `;`-separated statement run. A one-element `(do e)` is NOT returned
    /// (it has no faithful bare ML spelling — the surface never builds one — so it falls through to the
    /// parenthesizing `expr` path, matching the pre-sequencing behavior).
    fn as_do_seq(&self, id: StructId) -> Option<Vec<StructId>> {
        match self.a.get(id) {
            Struct::List(items)
                if items.len() >= 3 && self.head_name(items[0]).as_deref() == Some("do") =>
            {
                Some(items[1..].to_vec())
            }
            _ => None,
        }
    }

    /// True if `id` is a BRACKET-DELIMITED construct that manages its own self-contained
    /// indentation, so a `= <body>` hugs it — the opening delimiter stays on the `=` line and the
    /// contents break inside. That is `match`/`module` and the literal forms `record`/`list`/
    /// `tuple`/`map` (each opens its own indented box). Only the WELL-FORMED shapes qualify: a
    /// malformed literal falls back to the (non-self-indenting) call form, which must NOT be hugged
    /// or it would double-indent. Non-bracket forms (`let`, `if`, `fn`) are NOT hugged: they take
    /// the plain-expression path — inline when they fit, else a flat-laid-out indented continuation.
    fn is_block_body(&self, id: StructId) -> bool {
        let (head, args) = match self.a.get(id) {
            Struct::List(items) if !items.is_empty() => (items[0], &items[1..]),
            _ => return false,
        };
        // A body that renders as a `{…}`/`[…]`/`(a,b)`/`#{…}` literal hugs the `=`. Recognized through
        // the same gate as the render itself (`literal_ctor`), so a string primitive OR an unshadowed
        // name alias both hug — and a shadowed name (rendered as a plain call) does NOT.
        if let Some(ctor) = self.literal_ctor(head) {
            return match ctor.as_str() {
                "list" => true,
                "tuple" => args.len() >= 2,
                "record" => self.is_record_shape(args),
                "map" => self.is_map_shape(args),
                _ => false,
            };
        }
        match self.a.as_name(head) {
            Some("match") => self.is_match_shape(args),
            Some("module") => self.is_module_shape(args),
            _ => false,
        }
    }

    /// A `(do a b c)` reached through `expr` — i.e. a sequence used as a VALUE, a call argument, or an
    /// `if` branch. Such a sequence can't sit bare (a bare `a; b` would escape into the enclosing
    /// context), so it PARENTHESIZES: `(a; b; c)`, like OCaml's `let x = (f (); 42)`. The only bare
    /// (unparenthesized) sequences are a `let` body and the program root — printed via
    /// `print_do_stmts` from `print_stmt`/`print_root_forms`.
    fn print_do(&mut self, stmts: &[StructId]) {
        self.doc.cbox(INDENT);
        self.doc.word("(");
        self.doc.zerobreak();
        self.print_do_stmts(stmts);
        self.doc.break_with(0, -INDENT);
        self.doc.word(")");
        self.doc.end();
    }

    /// The statements of a sequence, bare: `a;⏎b;⏎c` — each on its own line, `;` after every statement
    /// except the last, at the current column. Shared by the parenthesized `print_do`, the bare `let`
    /// body (`print_stmt`), and the program root.
    ///
    /// A non-final statement whose rendering ends in a GREEDY open tail — a `match`/`fn`/`let`/`handle`/
    /// `host` body, all parsed at `expr(0)` — must be PARENTHESIZED: without a closer, that body would
    /// swallow the following `; rest` on re-parse (`match … | _ => 99` then `; next` becomes the arm body
    /// `(99; next)`). `if` is NOT greedy-tailed (its branches parse at `PREC_SEQ + 1`), so it never needs
    /// this. The LAST statement takes no `;` and so needs no wrapping.
    ///
    /// A non-final `def` is the same hazard: a function/value def body is a greedy `expr(0)` sequence
    /// position, so a NESTED `def fac(n) = <e>` followed by `; fac(x)` in an enclosing body would read
    /// `fac`'s body as `(do <e> (fac x))`, swallowing the enclosing block's next statement. Unlike a
    /// top-level def (delimited by wrapping just its `= body`, since a following `def` re-parses as a
    /// sibling), a body-local def followed by an EXPRESSION must wrap the WHOLE form — `(def fac(n) = <e>)`
    /// — so the `)` bounds the def body and the `; fac(x)` sequences at the enclosing level. (Wrapping only
    /// the `= body`, `def fac(n) = (<e>)`, does NOT help: the def body's `expr(0)` continues its `;`-run
    /// past the group's `)`.)
    fn print_do_stmts(&mut self, stmts: &[StructId]) {
        for (i, &s) in stmts.iter().enumerate() {
            if i > 0 {
                self.doc.hardbreak();
            }
            let last = i + 1 == stmts.len();
            if self.as_do_seq(s).is_some() {
                // A statement that is ITSELF a nested multi-statement `(do …)` must render as its
                // OWN parenthesized block `( a; b )` — inlining it (the `print_stmt` do path) would
                // splice its statements into THIS sequence and DROP the nested-`do` node, so the
                // reparse yields a flat `(do … a b …)` (one fewer AST node), failing the surface
                // round-trip. `expr` routes a `do` through `print_do`, which parenthesizes. Applies
                // in EVERY slot (final or not): the root path already parenthesizes a nested-`do`
                // via `expr`, so this makes the bare-body path (`let`/`fn`/`handle` body) consistent.
                self.expr(s, 0);
            } else if !last && (self.has_greedy_tail(s) || self.form_routes_delimit(s)) {
                self.doc.word("(");
                self.expr(s, 0);
                self.doc.word(")");
            } else {
                self.print_stmt(s);
            }
            if !last {
                self.doc.word(";");
            }
        }
    }

    /// True if `id` renders with a GREEDY trailing body — a `match`/`fn`/`let`/`handle`/`host` — whose
    /// last sub-expression is parsed at `expr(0)` and so would absorb a following `;` in a bare sequence
    /// (see [`Self::print_do_stmts`]). `if` is excluded: its branches parse at `PREC_SEQ + 1`, so a `;`
    /// after an `if` belongs to the enclosing sequence, not the `else` branch.
    fn has_greedy_tail(&self, id: StructId) -> bool {
        // A `(comment "text" inner)` wrapper is transparent — the tail is `inner`'s tail.
        if let Some(a) = self.a.as_form(id, "comment")
            && a.len() == 2
            && self.is_string(a[0])
        {
            return self.has_greedy_tail(a[1]);
        }
        let head = match self.a.get(id) {
            Struct::List(items) if !items.is_empty() => items[0],
            _ => return false,
        };
        matches!(
            self.head_name(head).as_deref(),
            Some("match" | "fn" | "let" | "handle" | "host")
        )
    }

    /// Whether a NON-LAST match-arm body, printed bare, would let the following `| pat` be ABSORBED —
    /// i.e. its TRAILING rendered sub-expression is an open `|`-arm list (`match`/`handle`). Only then
    /// must the arm body be parenthesized (see [`Self::print_match`]); otherwise the arm-terminating
    /// `|` delimits it cleanly and parens are the redundant-paren defect (the pervasive `(if …)`/`(let
    /// …)` match-arm parens the operator flagged in hm-collect.cdz).
    ///
    /// It follows the TAIL through the forms whose last rendered token IS their trailing sub-expression:
    /// `if`→its last branch (`else`, or `then` when no `else`), `let`/`fn`/`host`/`do`→their body/last
    /// statement, `@`-annotation→the annotated inner, and `comment`/`comment-after` wrappers→the inner.
    /// `match`/`handle` are TERMINAL-true (their own arm list is open). Anything else (an infix/call/
    /// literal/record/tuple/ascription — which ends in a closing token, not a greedy arm list) is false.
    /// This is the `|`-analog of [`Self::has_greedy_tail`]'s `;`-analysis, but tail-RECURSIVE (a greedy
    /// arm form can lurk under `if a then 1 else (match …)` / `@tag (match …)` / `let p=x in (match …)`)
    /// and only `match`/`handle` are terminal (an `fn`/`let`/`if` whose OWN tail isn't an arm form is
    /// `|`-safe). Depth-guarded by the same `MAX_PRINT_DEPTH` budget as `expr` (a decoded-only deep
    /// arena can't overflow — a straight-line follow, but guard anyway).
    fn arm_body_tail_is_open_arm_form(&self, id: StructId, depth: u32) -> bool {
        if depth > MAX_PRINT_DEPTH {
            return false;
        }
        let items = match self.a.get(id) {
            Struct::List(items) if !items.is_empty() => items,
            _ => return false,
        };
        // A `(comment "text" inner)` / `(comment-after inner "text")` wrapper is transparent.
        if let Some(a) = self.a.as_form(id, "comment")
            && a.len() == 2
            && self.is_string(a[0])
        {
            return self.arm_body_tail_is_open_arm_form(a[1], depth + 1);
        }
        if let Some(a) = self.a.as_form(id, "comment-after")
            && a.len() == 2
            && self.is_string(a[1])
        {
            return self.arm_body_tail_is_open_arm_form(a[0], depth + 1);
        }
        match self.head_name(items[0]).as_deref() {
            // Open `|`-arm lists — a following `| pat` extends them. Terminal-true.
            Some("match" | "handle") => true,
            // Tail-transparent forms: their LAST rendered arg is the trailing sub-expression. Follow it.
            // (`if cond then [else]` → last branch; `let binds body` / `fn params body` / `host … body`
            // / `do … last` → last; `@ ann inner` → inner.) A 1-arg (headless-tail) form can't run on.
            Some("if" | "let" | "fn" | "host" | "do" | "@") if items.len() >= 2 => {
                self.arm_body_tail_is_open_arm_form(items[items.len() - 1], depth + 1)
            }
            _ => false,
        }
    }

    /// Whether a match/handle arm BODY is a top-level bitwise-or `(| a b)` infix — which the printer
    /// renders with a bare `|` glyph (`a | b`). At the arm's own bracket level a `|` TERMINATES the arm
    /// (`Parser::arm_bar_terminates`), so a bare-`|` body re-parses as the start of the next arm and the
    /// right operand dangles (breaker's pipe-in-arm round-trip bug: `| tag(x,s) => x | 8` lost `8`).
    /// So an arm body headed by `|` MUST parenthesize — `=> (x | 8)` — for EVERY arm (the last/only arm
    /// too, since the bare `|` starts a phantom arm regardless of what follows). The `|` inside a call's
    /// args (`resume(x | 8, …)`) is already safe — `arg_exprs` clears the flag — so only a body whose
    /// OWN head is `|` needs this; a `|` nested inside a sub-call/bracket does not.
    fn arm_body_is_bare_pipe_infix(&self, id: StructId) -> bool {
        // Peel transparent comment wrappers, then check the head is the `|` bitwise-or operator.
        let mut node = id;
        loop {
            if let Some(a) = self.a.as_form(node, "comment")
                && a.len() == 2
                && self.is_string(a[0])
            {
                node = a[1];
                continue;
            }
            if let Some(a) = self.a.as_form(node, "comment-after")
                && a.len() == 2
                && self.is_string(a[1])
            {
                node = a[0];
                continue;
            }
            break;
        }
        match self.a.get(node) {
            Struct::List(items) if !items.is_empty() => {
                self.head_name(items[0]).as_deref() == Some("|")
            }
            _ => false,
        }
    }

    /// Print an expression in a VALUE position (a let-binding value). A construct that "eats forward"
    /// to the end of the enclosing sequence — a `let` (its body scopes to end) or a `(do …)` sequence
    /// — must PARENTHESIZE here, or it would swallow the statements that follow the binding:
    /// `let x = (let y = 3; …)`, `let x = (a; b)`. A `match`/`if`/`fn`/infix is self-delimited and
    /// prints bare.
    fn value(&mut self, id: StructId) {
        let head = match self.a.get(id) {
            Struct::List(items) if items.len() > 1 => self.head_name(items[0]),
            _ => None,
        };
        match head.as_deref() {
            Some("let") if self.is_let_shape_form(id) => {
                self.doc.word("(");
                self.expr(id, 0);
                self.doc.word(")");
            }
            // a `(do …)` reached via `expr` already parenthesizes itself (see `print_do`).
            _ => self.expr(id, 0),
        }
    }

    /// True if `id` is a well-formed `(let (binds…) body)` the `let` surface prints.
    fn is_let_shape_form(&self, id: StructId) -> bool {
        matches!(self.a.get(id), Struct::List(items) if items.len() == 3
            && self.head_name(items[0]).as_deref() == Some("let")
            && self.is_let_shape(&items[1..]))
    }

    /// The program root's top-level form sequence, printed bare (no wrapper). Top-level forms are
    /// JUXTAPOSED — blank-line separated (like the members of a `module` block) — because `;` is the
    /// WITHIN-body sequencing operator, not a top-level separator; a body's `;`-run stays inside that
    /// body's own `(do …)`.
    ///
    /// The ONE hazard is that the previous form's tail could absorb the next form. It arises only when
    /// the next form is NOT keyword-led (a keyword / `///` can never be absorbed into a preceding
    /// expression). Then, by what THIS form is:
    ///   • a `def` — parenthesize its `= body` (`def f(n) = (n + 1)` before `f(9)`); a def's greedy body
    ///     would SWALLOW a trailing `;`, so the body parens are the delimiter. (via `delimit_body`)
    ///   • a greedy-tailed form (`match`/`fn`/`let`/`handle`/`host`, whose last body parses at `expr(0)`)
    ///     — wrap the WHOLE form in parens; likewise a `;` would be swallowed by that trailing body.
    ///   • a plain bare expression — a trailing `;`. It re-parses as a stmt-level `(do prev next)` that
    ///     `push_root_form` splices back flat, preserving the tree. (Parens would be WRONG: `(5)` then
    ///     `(x)` re-lexes `)(` as application.)
    /// No `;` is emitted between keyword-led forms at all.
    fn print_root_forms(&mut self, forms: &[StructId]) {
        self.doc.cbox(0);
        for (i, &form) in forms.iter().enumerate() {
            if i > 0 {
                // A BLANK line between top-level forms — definitions read as a crammed wall packed
                // one-per-line. A `///` doc line hugs the def it documents (no blank between a doc
                // and its form), so only break blank BEFORE a non-doc form, and never right after a
                // doc line for the form it annotates.
                if self.leads_doc_block(forms, i) {
                    self.doc.hardbreak();
                } else {
                    self.blank_line();
                }
            }
            if let Some(a) = self.a.as_form(form, "doc")
                && a.len() == 1
                && self.is_string(a[0])
            {
                self.print_doc(a[0]);
                continue;
            }
            // A `(module-doc "text")` is a standalone top-level doc line (a file/module header) — print
            // it as its own `///` line, NO trailing `;` (a `///` runs to end-of-line, so a `;` after it
            // would be swallowed INTO the doc text on re-read — breaking idempotence). Same handling as
            // a top-level `(doc …)`.
            if let Some(a) = self.a.as_form(form, "module-doc")
                && a.len() == 1
                && self.is_string(a[0])
            {
                self.print_doc(a[0]);
                continue;
            }
            // Separate this form from the next when the next is non-keyword-led — see the doc comment
            // for the three mechanisms (def-body parens / whole-form parens / trailing `;`).
            let need = i + 1 < forms.len() && !self.form_starts_with_keyword(forms[i + 1]);
            if need && self.form_routes_delimit(form) {
                self.delimit_body = true;
                self.expr(form, 0);
                self.delimit_body = false;
            } else if need && self.has_greedy_tail(form) {
                // A greedy-tailed form (`match`/`fn`/`let`/`handle`/`host`) can't take a `;` (its body
                // would swallow it) — wrap the whole form so its tail is closed.
                self.doc.word("(");
                self.expr(form, 0);
                self.doc.word(")");
            } else {
                self.expr(form, 0);
                if need {
                    self.doc.word(";");
                }
            }
        }
        self.doc.end();
    }

    /// True if `form` is a DECLARATION-keyword form (`def`/`module`/`type`/`effect`) — one the ML reader's
    /// `;`-sequence loop BREAKS before (`at_declaration_keyword`, from `539f7712`), because at the top level
    /// a bare declaration after `;` begins the next juxtaposed form. So a NON-FINAL such form inside a `do`
    /// body must be WRAPPED — `(module Inc { … })` — by [`Self::print_do_stmts`]: the leading `(` makes it an
    /// EXPRESSION, not a bare declaration keyword, so the reader collects it into the body's sequence
    /// instead of ending the body and leaking the rest as top-level siblings. Without this a body of two
    /// adjacent `module`s (`def main() = module A { … }; module B { … }; expr`) truncates after the first
    /// module — the exact round-trip the printer must not produce (it emits ML the reader then rejects).
    /// `def` additionally has a greedy `= body`; a wrapping `(def f() = e)` bounds that body too. `import`
    /// and `export` are top-level-only, never a body statement, so they need not be listed.
    fn form_routes_delimit(&self, form: StructId) -> bool {
        matches!(self.a.get(form), Struct::List(items) if !items.is_empty()
        && matches!(
            self.head_name(items[0]).as_deref(),
            Some("def" | "module" | "type" | "effect")
        ))
    }

    /// True when `id` prints with a leading RESERVED word (a keyword form) or a `///`/`//` comment lead —
    /// a form whose first surface token cannot be absorbed into a preceding expression's tail, so no `;`
    /// separator is needed before it at the top level (see [`Self::print_root_forms`]). A `(comment …)`
    /// wrapper is transparent (its inner form's first token is what a preceding parse would meet). A
    /// `(doc …)` prints `///`, itself a lead — treated as safe.
    fn form_starts_with_keyword(&self, id: StructId) -> bool {
        // A comment wrapper `(comment "text" inner)` — the boundary token is `inner`'s first token.
        if let Some(a) = self.a.as_form(id, "comment")
            && a.len() == 2
            && self.is_string(a[0])
        {
            return self.form_starts_with_keyword(a[1]);
        }
        // An annotation `(@ name inner)` prints as `@name inner`; its LEADING surface token is the `@`
        // sigil, which — like a keyword — is a self-delimiting form boundary the next juxtaposed form
        // cannot fuse onto (an `@` can only begin a fresh annotation). So an annotation counts as
        // keyword-starting: without this, a top-level `@test def …` FOLLOWED by another form was treated as
        // "next form is open" and got a spurious trailing `;` (`@test def a() = unit;`), which then failed
        // to re-parse (`a do block must end in a value form`). Recurse to the annotated inner too, so the
        // stacked/nested cases agree, matching the `comment`-wrapper recursion above.
        if let Some(a) = self.a.as_form(id, "@")
            && a.len() == 2
        {
            return true;
        }
        let head = match self.a.get(id) {
            Struct::List(items) if !items.is_empty() => items[0],
            _ => return false,
        };
        matches!(
            self.head_name(head).as_deref(),
            Some(
                "def"
                    | "type"
                    | "effect"
                    | "module"
                    | "import"
                    | "export"
                    | "let"
                    | "if"
                    | "fn"
                    | "match"
                    | "handle"
                    | "host"
                    | "doc"
                    | "module-doc",
            )
        )
    }

    /// A blank line: two hard breaks, so a consistent box emits an empty line between the two
    /// items. Layout-only (idempotent whitespace) — a re-parse yields the same arena.
    fn blank_line(&mut self) {
        self.doc.hardbreak();
        self.doc.hardbreak();
    }

    /// True if `forms[i]` continues a doc block that began at `forms[i-1]` — i.e. the previous form
    /// is a `(doc …)` line, so this form (its documented target, or the next doc line) must hug it
    /// with a single break rather than a blank line. Keeps a `/// …` comment glued to what it
    /// documents while still blank-separating distinct top-level definitions.
    fn leads_doc_block(&self, forms: &[StructId], i: usize) -> bool {
        i > 0 && (self.is_doc(forms[i - 1]) || self.is_module_doc(forms[i - 1]))
    }

    /// True if `id` is a well-formed `(module-doc "text")` node — a file/module-level doc line. Like a
    /// `(doc …)`, it hugs the following form (a further header line or the documented decl) with a
    /// single break rather than a blank, so a multi-line `///` file header prints as a contiguous block.
    fn is_module_doc(&self, id: StructId) -> bool {
        matches!(self.a.as_form(id, "module-doc"), Some(a) if a.len() == 1 && self.is_string(a[0]))
    }

    /// `module name { form… }` — one member per line (consistent box) when broken, blank-separated
    /// so definitions don't cram together. The first member breaks straight off the `{` (no leading
    /// blank inside the braces); a `///` doc line hugs the member it documents.
    fn print_module(&mut self, args: &[StructId]) {
        // A `///` doc that precedes the `module` keyword documents the MODULE itself — the reader attaches
        // it as a LEADING `(doc …)` MEMBER (args after the name, before the first real member). That is a
        // DISTINCT tree from a doc INSIDE the braces, which attaches to the def it precedes
        // (`(module M (def (x) (doc …) …))`). So the leading module-doc run must print ABOVE the `module`
        // line, at the module's own column — printing it as an in-body `///` line instead re-reads as a
        // doc on the first body member, silently migrating the module-doc onto that member (a round-trip
        // break). Mirrors `print_type`/`print_effect`: leading decl docs print flush, outside the box.
        let docs_end = 1 + args[1..].iter().take_while(|&&a| self.is_doc(a)).count();
        let docs = &args[1..docs_end];
        let members = &args[docs_end..];
        for &d in docs {
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
        self.doc.cbox(INDENT);
        self.doc.word("module ");
        self.expr(args[0], 0); // name
        self.doc.word(" {");
        for (i, &form) in members.iter().enumerate() {
            // First member hugs the `{` with a single break; later members are blank-separated,
            // except a member that continues a doc block (its documented form or a further doc
            // line), which hugs the doc with a single break.
            if i == 0 || self.leads_doc_block(members, i) {
                self.doc.hardbreak();
            } else {
                self.blank_line();
            }
            // a `(doc …)` module member renders as a `///` line (body position); anything else as
            // its ordinary form.
            if let Some(a) = self.a.as_form(form, "doc")
                && a.len() == 1
                && self.is_string(a[0])
            {
                self.print_doc(a[0]);
                continue;
            }
            self.expr(form, 0);
        }
        self.doc.break_with(1, -INDENT);
        self.doc.word("}");
        self.doc.end();
    }

    /// `type Name = A(T, …) | B | …` — a sum-type declaration. `args` is `name doc… variant…`; each
    /// variant is a nullary constructor (a `Name` atom -> `Ctor`) or a payload constructor (a list
    /// `(Ctor T …)` -> `Ctor(T, …)`), joined by ` | `. Inline when it fits; else each variant on its
    /// own line with a leading `| ` at the `type` column. Never parenthesized (a declaration).
    fn print_type(&mut self, args: &[StructId]) {
        // args = name, then optional `(doc …)` forms, then the variants.
        let docs_end = 1 + args[1..].iter().take_while(|&&a| self.is_doc(a)).count();
        let name = args[0];
        let docs = &args[1..docs_end];
        let mut variants = &args[docs_end..];
        // OPEN SUM: a trailing `.. r` row-variable marker is the flat two-sibling pair `Name("..")` then
        // a lowercase `Name` at the END of the variant list (open-sums OS1). Peel it off so it prints as
        // a trailing `.. r` (the ML surface for an open sum), NOT as two spurious `| ..` / `| r` variants
        // (which is what the uniform variant loop would emit — the garbage-render that would not
        // round-trip). Mirrors the reader's trailing-`.. name` parse in `type_expr`.
        let mut row_var: Option<StructId> = None;
        if variants.len() >= 2 && self.a.as_name(variants[variants.len() - 2]) == Some("..") {
            row_var = Some(variants[variants.len() - 1]);
            variants = &variants[..variants.len() - 2];
        }
        // Leading `///` docs print at the type declaration's OWN column, OUTSIDE the `cbox(INDENT)` that
        // indents the `|`-led variants — otherwise the doc block's continuation lines (every line after the
        // first `hardbreak`) reflow to the box's INDENT while line 1 stays at column 0, an inconsistent
        // per-line indent within one doc header (PR-flagged: a multi-line `type` doc-header printed line 1
        // flush but lines 2+ indented 2 spaces). Mirrors `print_def`, which prints its docs flush.
        for &d in docs {
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
        self.doc.cbox(INDENT);
        self.doc.word("type ");
        self.expr(name, 0);
        self.doc.word(" =");
        // Each variant on its own line, led by `| ` (always, including the first) — symmetric with a
        // `match`'s `|`-led arms. The `|` is the surface separator between the structural variant
        // entries, never a node in the tree.
        for &raw in variants {
            // Peel BOTH comment wrappers in EITHER nesting order (a variant can carry a LEADING
            // `(comment …)` own-line comment AND a TRAILING `(comment-after …)` same-line one — e.g. a
            // multi-line trailing comment on the PRIOR variant leaves its own-line continuation lines as
            // this variant's leading, nested OUTSIDE or INSIDE its own trailing). A leading text prints as
            // a `// …` line ABOVE the `| `; a trailing text ` // …` after the variant. Peeling only the
            // leading `comment` (with `print_variant` doing the trailing) failed when the outer wrapper was
            // the `comment-after` — the inner leading `(comment …)` then rendered as a garbage
            // `comment(text, …)` variant (seq-277/C3: ty.cdz's multi-line variant trailing comments).
            // `is_type_shape` accepts all of this via `strip_field_comments`, so the printer must be total.
            let mut v = raw;
            let mut lead_texts: Vec<StructId> = Vec::new();
            let mut trail_texts: Vec<StructId> = Vec::new();
            loop {
                if let Some(a) = self.a.as_form(v, "comment")
                    && a.len() == 2
                    && self.is_string(a[0])
                {
                    lead_texts.push(a[0]);
                    v = a[1];
                    continue;
                }
                if let Some(a) = self.a.as_form(v, "comment-after")
                    && a.len() == 2
                    && self.is_string(a[0])
                {
                    trail_texts.push(a[0]);
                    v = a[1];
                    continue;
                }
                break;
            }
            for &text in &lead_texts {
                self.doc.hardbreak();
                self.doc.word(format!("//{}", self.doc_line_text(text)));
            }
            self.doc.hardbreak();
            self.doc.word("| ");
            self.print_variant(v);
            for &text in trail_texts.iter().rev() {
                self.doc.word(format!(" //{}", self.doc_line_text(text)));
            }
        }
        // The open-sum tail prints after the last variant, on its own line, as `.. r` — re-read by
        // `type_expr`'s trailing row-var parse to the same two sibling atoms (round-trip identity).
        if let Some(r) = row_var {
            self.doc.hardbreak();
            self.doc.word(".. ");
            self.expr(r, 0);
        }
        self.doc.end();
    }

    /// `(forall (a b) TYPE)` is well-formed for the surface sugar when it is exactly a binder LIST (of
    /// one-or-more `Name` atoms) followed by a body — the shape `forall_type` builds. Anything else
    /// (`forall` as a user application head, an empty binder list, a non-list first arg) falls through to
    /// the generic call form, so a name `forall` is never mis-sugared.
    fn is_forall_shape(&self, args: &[StructId]) -> bool {
        args.len() == 2
            && matches!(self.a.get(args[0]), Struct::List(bs)
                if !bs.is_empty() && bs.iter().all(|&b| self.a.as_name(b).is_some()))
    }

    /// `(forall (a b) TYPE)` -> `forall a b. TYPE` (the inverse of the parser's `forall_type`). The body
    /// prints at `PREC_ARROW` so a function-type body needs no parens (`forall a. a -> a`, matching the
    /// parser's looser-than-arrow binding); a caller that needs the whole `forall` parenthesized (rare —
    /// a `forall` under a tighter operator) is handled by the parent-prec guard.
    fn print_forall(&mut self, args: &[StructId], parent_prec: u8) {
        let binders = match self.a.get(args[0]) {
            Struct::List(bs) => bs.clone(),
            _ => return, // guarded by is_forall_shape
        };
        // A `forall` binds looser than the arrow; if it sits under something tighter, parenthesize.
        let paren = parent_prec > PREC_ARROW;
        if paren {
            self.doc.word("(");
        }
        self.doc.word("forall ");
        for (i, &b) in binders.iter().enumerate() {
            if i > 0 {
                self.doc.word(" ");
            }
            self.expr(b, 0);
        }
        self.doc.word(". ");
        self.expr(args[1], PREC_ARROW);
        if paren {
            self.doc.word(")");
        }
    }

    /// One sum-type variant: a nullary `Ctor` (a `Name` atom) prints as itself; a payload variant
    /// `(Ctor T …)` prints as `Ctor(T, …)` — the same shape as a constructor application.
    fn print_variant(&mut self, id: StructId) {
        // A `(comment-after "text" variant)` wrapper — a `//` comment that trailed this variant on the
        // same source line (`| Ctor(T)  // note`). Print the inner variant, then ` // text` (trailing,
        // no break), so it re-reads to the same wrapper. Unwrap FIRST so the wrapper isn't mistaken for
        // a `(ctor payload…)` variant by the arms below.
        if let Some(a) = self.a.as_form(id, "comment-after")
            && a.len() == 2
            && self.is_string(a[0])
        {
            self.print_variant(a[1]);
            self.doc.word(format!(" //{}", self.doc_line_text(a[0])));
            return;
        }
        match self.a.get(id) {
            Struct::List(items) if items.len() >= 2 => {
                let items = items.clone();
                self.expr(items[0], 0); // constructor name
                self.doc.word("(");
                for (i, &t) in items[1..].iter().enumerate() {
                    if i > 0 {
                        self.doc.word(", ");
                    }
                    self.expr(t, 0);
                }
                self.doc.word(")");
            }
            // A 1-element list `(A)` is a nullary variant in its EMPTY-PARENS spelling — render it as `A()`
            // (the `()` preserved), NOT bare `A`. This is a DISTINCT arena from the bare-atom nullary `A`
            // (`(A)` the 1-elem list vs `A` the atom), and `A()` re-reads to `(A)` while bare `A` re-reads
            // to the atom — so rendering `A` here would CANONICALIZE `(A)` → `A`, which breaks a corpus
            // round-trip whose reference uses the `(A)` spelling (e.g. `(type Nat (Z) (S Nat))`:
            // corpus_roundtrip requires read(ml(read(x))) == read(x) EXACTLY, no canonicalization). Emit
            // `A()` so the exact 1-elem-list shape survives. (The earlier fallback `_` arm below ALSO
            // printed `A()` — via `expr` on the list — but did not count as a type-shape variant, so the
            // whole type fell to the backtick-application render; the point of this arm is that
            // `is_type_shape` now accepts `(A)`, so the type renders as a `type …` decl AND its `(A)`
            // variant prints `A()`, round-trip-preserving.)
            Struct::List(items) if items.len() == 1 && self.head_name(items[0]).is_some() => {
                self.expr(items[0], 0); // constructor name
                self.doc.word("()");
            }
            // a nullary variant (bare name atom), or a defensive fallback for an odd shape
            _ => self.expr(id, 0),
        }
    }

    /// `effect Name = | op : Type | …` — an effect declaration. `args` is `name (op <op> <ty>)…`; each
    /// operation renders as `op : ty` on its own line, led by `| ` (mirroring a `type Name = | A | B`
    /// sum-type declaration — the operations are the effect's "variants"). Never parenthesized.
    fn print_effect(&mut self, args: &[StructId]) {
        // args = name, then optional `(doc …)` forms, then the operations.
        let docs_end = 1 + args[1..].iter().take_while(|&&a| self.is_doc(a)).count();
        let docs = &args[1..docs_end];
        let ops = &args[docs_end..];
        // Leading `///` docs print at the effect declaration's OWN column, OUTSIDE the `cbox(INDENT)` that
        // indents the `|`-led operations — otherwise the doc block's continuation lines (every line after
        // the first `hardbreak`) reflow to the box's INDENT while line 1 stays flush, an inconsistent
        // per-line indent within one doc header. Mirrors `print_type`/`print_def` (docs flush; the `|`-led
        // entries indented under the keyword).
        for &d in docs {
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
        self.doc.cbox(INDENT);
        self.doc.word("effect ");
        self.expr(args[0], 0); // effect name
        self.doc.word(" =");
        // Each operation on its own line, led by `| ` (always, including the first) — symmetric with a
        // sum type's `|`-led variants. The `|` is the surface separator between the operation
        // signatures, never a node in the tree.
        for &op_raw in ops {
            // Peel a trailing `(comment-after "text" op)` the reader attaches to an op-signature (seq-277):
            // print the op, then ` // text` same-line after its signature. Head is `comment-after`, so
            // `as_form(op,"op")` below would otherwise MISS and drop the whole op. `strip_comments` peels it.
            let (op, op_trail) = match self.a.as_form(op_raw, "comment-after") {
                Some(a) if a.len() == 2 && self.is_string(a[0]) => (a[1], Some(a[0])),
                _ => (op_raw, None),
            };
            self.doc.hardbreak();
            self.doc.word("| ");
            // op = (op <name> <ty> (resource <idx>)?). The optional trailing `(resource N)` is the
            // SEC-F1 resource-marker decl-metadata the parser LIFTED off the N-th param's `@resource`; on
            // print we RE-INJECT `(@ resource …)` around that param into a temp type tree, then print via
            // the normal op-type path (which renders the `@`-annotation as `@resource T`), so it
            // round-trips. No resource sibling => print the type as-is.
            if let Some(o) = self.a.as_form(op, "op") {
                self.expr(o[0], 0); // operation name
                self.doc.word(" : ");
                let resource_idx = o
                    .get(2)
                    .and_then(|&sib| self.a.as_form(sib, "resource"))
                    .and_then(|r| r.first().copied())
                    .and_then(|idx| self.a.as_int_usize(idx));
                self.print_op_type_with_resource(o[1], resource_idx);
            }
            if let Some(text) = op_trail {
                self.doc.word(format!(" //{}", self.doc_line_text(text)));
            }
        }
        self.doc.end();
    }

    /// A `(world Name (import|export Iface (member M (func …))…)…)` -> the inline `world …` surface, the
    /// dual of `world_expr` (guarded by [`Self::is_world_shape`]). `world Name =` then each interface on
    /// its own `| import|export Iface =` line, then each member indented `| member : (p : T, …) -> R`.
    /// The direction word IS the interface head (`import`/`export`); the member func node reverses to the
    /// parenthesized named-param + arrow-result surface.
    fn print_world(&mut self, args: &[StructId]) {
        let docs_end = 1 + args[1..].iter().take_while(|&&a| self.is_doc(a)).count();
        let docs = &args[1..docs_end];
        let ifaces = &args[docs_end..];
        for &d in docs {
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
        self.doc.cbox(INDENT);
        self.doc.word("world ");
        self.expr(args[0], 0); // world name
        self.doc.word(" =");
        for &iface in ifaces {
            // iface = (import|export IfaceName (member …)…)
            let (dir, entry) = if let Some(e) = self.a.as_form(iface, "import") {
                ("import", e)
            } else if let Some(e) = self.a.as_form(iface, "export") {
                ("export", e)
            } else {
                continue; // guarded by is_world_shape; be total
            };
            self.doc.hardbreak();
            self.doc.word("| ");
            self.doc.word(dir);
            self.doc.word(" ");
            self.expr(entry[0], 0); // interface name
            self.doc.word(" =");
            // Each member indented under its interface, `| name : sig`.
            self.doc.cbox(INDENT);
            for &m in &entry[1..] {
                if let Some(mem) = self.a.as_form(m, "member") {
                    self.doc.hardbreak();
                    self.doc.word("| ");
                    self.expr(mem[0], 0); // member name
                    self.doc.word(" : ");
                    self.print_world_func_sig(mem[1]);
                }
            }
            self.doc.end();
        }
        self.doc.end();
    }

    /// A member's `(func (param <n> <t>)* (result <t>))` -> `(p1 : T1, …) -> R`. A nullary func (no
    /// params) elides the list to `() -> R`. The dual of `world_member`'s signature parse.
    fn print_world_func_sig(&mut self, func: StructId) {
        let Some(f) = self.a.as_form(func, "func") else {
            self.expr(func, 0); // guarded by is_world_member_shape; be total
            return;
        };
        let Some((&result, params)) = f.split_last() else {
            return;
        };
        self.doc.word("(");
        for (i, &p) in params.iter().enumerate() {
            if i > 0 {
                self.doc.word(", ");
            }
            if let Some(pp) = self.a.as_form(p, "param") {
                self.expr(pp[0], 0); // param name
                self.doc.word(" : ");
                self.print_wit_type(pp[1]); // param type descriptor -> surface type
            }
        }
        self.doc.word(") -> ");
        if let Some(r) = self.a.as_form(result, "result") {
            self.print_wit_type(r[0]); // result type descriptor -> surface type
        }
    }

    /// Print a WIT type DESCRIPTOR back to its inline-world surface type — the inverse of the parser's
    /// `wit_type_desc_of`, so a world member round-trips. A primitive `(name)` prints bare `name`; a
    /// `("list" <elem>)` prints `list(<elem>)`; an `("option" <inner>)` prints `option(<inner>)`; a
    /// `("record" (f <ty>)…)` prints the brace record type `{f: <ty>, …}`; a `("result" <ok> <err>)` prints
    /// `result` / `result(<ok>)` / `result(_, <err>)` / `result(<ok>, <err>)` (`_` = an absent arm); a
    /// `("variant" (Case <ty>?)…)` prints `variant(Case, Case2(<ty>), …)` (a bare case is payload-less); an
    /// `("enum" A …)` / `("flags" A …)` prints `enum(A, …)` / `flags(A, …)`. A node that is NOT one of these
    /// descriptor shapes prints via the generic expr surface (a raw type node the lowering left as-is).
    fn print_wit_type(&mut self, ty: StructId) {
        // Primitive `(name)`: a one-element list whose sole child is a NAME atom -> bare `name`.
        if let Struct::List(kids) = self.a.get(ty)
            && kids.len() == 1
            && let Some(name) = self.a.as_name(kids[0])
        {
            self.doc.word(emit_name(name));
            return;
        }
        // `unit` descriptor `("unit")`: a one-element list whose sole child is a STRING atom -> bare `unit`.
        if let Struct::List(kids) = self.a.get(ty)
            && kids.len() == 1
            && self.a.as_str(kids[0]) == Some("unit")
        {
            self.doc.word("unit");
            return;
        }
        // `list`/`option` `("head" <child>)`: a two-element list, STRING head -> `head(<child>)`.
        if let Struct::List(kids) = self.a.get(ty)
            && kids.len() == 2
            && let Some(head) = self.a.as_str(kids[0])
            && matches!(head, "list" | "option")
        {
            let child = kids[1];
            self.doc.word(head);
            self.doc.word("(");
            self.print_wit_type(child);
            self.doc.word(")");
            return;
        }
        // `tuple` `("tuple" <a> <b> …)`: a STRING head + N element descriptors -> `tuple(<a>, <b>, …)`.
        if let Struct::List(kids) = self.a.get(ty)
            && kids.len() >= 2
            && self.a.as_str(kids[0]) == Some("tuple")
        {
            let elems: Vec<StructId> = kids[1..].to_vec();
            self.doc.word("tuple(");
            for (i, &e) in elems.iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.print_wit_type(e);
            }
            self.doc.word(")");
            return;
        }
        // `record` `("record" (fname <ty>)…)`: STR head + `(name ty)` field pairs -> `{fname: <ty>, …}`
        // (the brace record-TYPE surface, which `wit_type_desc_of` reads back to the same descriptor). An
        // empty record is `{}`. Collect the (owned name, type-id) pairs FIRST so the arena borrow releases
        // before the recursive `print_wit_type` (which mutates the doc).
        let record_fields: Option<Vec<(String, StructId)>> = match self.a.get(ty) {
            Struct::List(kids)
                if kids.first().and_then(|&h| self.a.as_str(h)) == Some("record") =>
            {
                Some(
                    kids[1..]
                        .iter()
                        .filter_map(|&f| match self.a.get(f) {
                            Struct::List(pair) if pair.len() == 2 => {
                                self.a.as_name(pair[0]).map(|n| (n.to_string(), pair[1]))
                            }
                            _ => None,
                        })
                        .collect(),
                )
            }
            _ => None,
        };
        if let Some(fields) = record_fields {
            self.doc.word("{");
            for (i, (fname, fty)) in fields.iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.doc.word(emit_name(fname));
                self.doc.word(": ");
                self.print_wit_type(*fty);
            }
            self.doc.word("}");
            return;
        }
        // `result` `("result" <ok> <err>)`: STR head + exactly two slots, each a type descriptor OR the
        // `("none")` absent-marker. Print the WIT-faithful surface — `result` (both absent), `result(<ok>)`
        // (err absent), `result(_, <err>)` (ok absent), `result(<ok>, <err>)` (both present); `_` spells an
        // absent arm. Extract the slot ids + presence FIRST so the arena borrow releases before recursing.
        let result_arms: Option<(StructId, StructId, bool, bool)> = match self.a.get(ty) {
            Struct::List(kids) if kids.len() == 3 && self.a.as_str(kids[0]) == Some("result") => {
                let (ok, err) = (kids[1], kids[2]);
                Some((
                    ok,
                    err,
                    self.a.head_ctor(ok) == Some("none"),
                    self.a.head_ctor(err) == Some("none"),
                ))
            }
            _ => None,
        };
        if let Some((ok, err, ok_absent, err_absent)) = result_arms {
            self.doc.word("result");
            match (ok_absent, err_absent) {
                (true, true) => {} // bare `result`
                (false, true) => {
                    self.doc.word("(");
                    self.print_wit_type(ok);
                    self.doc.word(")");
                }
                (true, false) => {
                    self.doc.word("(_, ");
                    self.print_wit_type(err);
                    self.doc.word(")");
                }
                (false, false) => {
                    self.doc.word("(");
                    self.print_wit_type(ok);
                    self.doc.word(", ");
                    self.print_wit_type(err);
                    self.doc.word(")");
                }
            }
            return;
        }
        // `variant` `("variant" (Case <ty>?)…)`: STR head + one entry per case — a `(Case)` 1-list is
        // payload-less, a `(Case <ty>)` 2-list carries a payload. Prints `variant(Case, Case2(<ty>), …)`,
        // the inverse of `wit_type_desc_of`'s variant arm. Collect the (owned name, optional payload-id)
        // pairs FIRST so the arena borrow releases before the recursive `print_wit_type`.
        let variant_cases: Option<Vec<(String, Option<StructId>)>> = match self.a.get(ty) {
            Struct::List(kids)
                if kids.first().and_then(|&h| self.a.as_str(h)) == Some("variant") =>
            {
                Some(
                    kids[1..]
                        .iter()
                        .filter_map(|&c| match self.a.get(c) {
                            Struct::List(case) if case.len() == 1 => {
                                self.a.as_name(case[0]).map(|n| (n.to_string(), None))
                            }
                            Struct::List(case) if case.len() == 2 => self
                                .a
                                .as_name(case[0])
                                .map(|n| (n.to_string(), Some(case[1]))),
                            _ => None,
                        })
                        .collect(),
                )
            }
            _ => None,
        };
        if let Some(cases) = variant_cases {
            self.doc.word("variant(");
            for (i, (name, payload)) in cases.iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.doc.word(emit_name(name));
                if let Some(p) = payload {
                    self.doc.word("(");
                    self.print_wit_type(*p);
                    self.doc.word(")");
                }
            }
            self.doc.word(")");
            return;
        }
        // `enum`/`flags` `("enum"|"flags" Name…)`: STR head + bare NAME cases/bits -> `enum(Name, …)` /
        // `flags(Name, …)`, the inverse of `wit_type_desc_of`'s enum/flags arms. Collect the head kind +
        // (owned) names first so the arena borrow releases before touching the doc.
        let enum_flags: Option<(&'static str, Vec<String>)> = match self.a.get(ty) {
            Struct::List(kids) => match kids.first().and_then(|&h| self.a.as_str(h)) {
                Some(head @ ("enum" | "flags")) => {
                    let names: Option<Vec<String>> = kids[1..]
                        .iter()
                        .map(|&c| self.a.as_name(c).map(str::to_string))
                        .collect();
                    names.map(|ns| (if head == "enum" { "enum" } else { "flags" }, ns))
                }
                _ => None,
            },
            _ => None,
        };
        if let Some((head, names)) = enum_flags {
            self.doc.word(head);
            self.doc.word("(");
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.doc.word(emit_name(n));
            }
            self.doc.word(")");
            return;
        }
        // Not a recognized descriptor — the raw type node the lowering left as-is.
        self.expr(ty, 0);
    }

    /// An effect operation's type. An operation type is always a function arrow. The flat two-element
    /// `(-> P R)` prints via the ordinary arrow surface `P -> R`. The NULLARY-elided one-element
    /// `(-> R)` (typed as `Unit -> R`) has no infix form, so it prints with a LEADING arrow `-> R` —
    /// the surface `effect_op` reads back to the same one-element node.
    fn print_op_type(&mut self, ty: StructId) {
        if let Some(a) = self.a.as_form(ty, "->")
            && a.len() == 1
        {
            self.doc.word("-> ");
            self.expr(a[0], PREC_ARROW);
            return;
        }
        self.expr(ty, 0);
    }

    /// Print an effect-op type, re-injecting the SEC-F1 `@resource` marker before the param at
    /// `resource_idx` (the position the parser LIFTED it from into the `(resource N)` sibling). With
    /// `None` this is exactly [`print_op_type`]. With `Some(n)`, walk the curried-arrow operand spine
    /// (mirroring [`Self::arrow`]) and prefix `@resource ` on the n-th PARAM operand, so the surface
    /// round-trips (`write : @resource Bytes -> Bytes -> Unit`). A `resource_idx` past the param count
    /// (shouldn't happen — the parser only records a real param index) degrades to the plain form.
    fn print_op_type_with_resource(&mut self, ty: StructId, resource_idx: Option<usize>) {
        let Some(n) = resource_idx else {
            return self.print_op_type(ty);
        };
        // Collect the flat operand spine `[P0, P1, …, R]` — the same walk `arrow()` does. A leading-arrow
        // nullary op-type `(-> R)` has no params, so the marker can't apply; fall back.
        if self.a.as_form(ty, "->").map(|a| a.len()) == Some(1) {
            return self.print_op_type(ty);
        }
        let mut operands = Vec::new();
        let mut cur = ty;
        loop {
            if let Struct::List(items) = self.a.get(cur)
                && items.len() == 3
                && self.head_name(items[0]).as_deref() == Some("->")
            {
                operands.push(items[1]);
                cur = items[2];
                continue;
            }
            break;
        }
        operands.push(cur); // the result
        // If the recorded index isn't a param position, degrade to the plain type.
        if n + 1 >= operands.len() {
            return self.print_op_type(ty);
        }
        self.doc.ibox(INDENT);
        for (i, &operand) in operands.iter().enumerate() {
            if i > 0 {
                self.doc.space();
                self.doc.word("-> ");
            }
            if i == n {
                self.doc.word("@resource ");
            }
            let operand_prec = if i + 1 < operands.len() {
                PREC_ARROW + 1
            } else {
                PREC_ARROW
            };
            self.expr(operand, operand_prec);
        }
        self.doc.end();
    }

    /// Render a first-class embedded-syntax region `(embedded #<grammar> <subtree>)` as its SURFACE
    /// `grammar{ <text> }`, dispatching to the sub-grammar's OWN printer on the grafted subtree. Returns
    /// `true` if it emitted (a reserved grammar with a printer), `false` for an unknown tag (caller falls
    /// through to the generic render). `#[inline(never)]` so its locals (the sub-arena + rendered body
    /// `String`) do NOT bloat the recursive `expr`/`list` hub's stack frame — inlining them tipped a
    /// MAX_NESTING_DEPTH-deep arena walk over the test thread's stack (the deep-flat-chain guard), exactly
    /// like `print_param_pragma`. Without this arm a `json{ … }` re-printed as the generic application
    /// `embedded(#json, json-object(…))` — structurally equal, so the round-trip test passed, but NOT the
    /// readable surface, so `cdz fmt` destroyed embedded syntax.
    #[inline(never)]
    fn print_embedded(&mut self, grammar: &str, subtree: StructId) -> bool {
        let sub = crate::query::Tree::from_arena(self.a, subtree).to_arena();
        let body = match grammar {
            "json" => crate::json::print(&sub, self.width, crate::printer::print),
            "toml" => crate::toml_surface::print(&sub, self.width, crate::printer::print),
            _ => return false,
        };
        // `grammar{ <body> }` — a single space inside the braces so it re-lexes as the region (the reader
        // scans raw bytes from just after `{`; the tag must be GLUED to `{`, which `word` preserves).
        self.doc.word(format!("{grammar}{{ {} }}", body.trim()));
        true
    }

    /// Render an `@!param` module directive `(pragma param (param <kv>…) (: name Type))` as
    /// `@!param(k: v, …) name : Type` (empty config -> `@!param name : Type`, no parens). `config_node` is
    /// the `(param <kv>…)` sublist, `binder_node` the `(: name Type)` ascription. `#[inline(never)]` so its
    /// locals (the config/kv vecs) do NOT bloat the recursive `expr` hub's stack frame — inlining them
    /// tipped a MAX_NESTING_DEPTH-deep arena walk over the test thread's stack (the deep-flat-chain guard).
    /// The caller has already verified the shape (`param` key, `(param …)` config, 2-element `:` binder).
    #[inline(never)]
    fn print_param_pragma(&mut self, config_node: StructId, binder_node: StructId) {
        let cfg = match self.a.as_form(config_node, "param") {
            Some(c) => c.to_vec(),
            None => return,
        };
        let binder = match self.a.as_form(binder_node, ":") {
            Some(b) if b.len() == 2 => b.to_vec(),
            _ => return,
        };
        self.doc.cbox(0);
        self.doc.word("@!param");
        // config args `(k: v, …)` — only when non-empty (an empty `(param)` prints no parens).
        if !cfg.is_empty() {
            self.doc.word("(");
            for (i, &kv) in cfg.iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                // each kv is `(: key value)` -> `key: value`; render defensively via `expr` otherwise.
                if let Some(pair) = self.a.as_form(kv, ":")
                    && pair.len() == 2
                {
                    let (k, v) = (pair[0], pair[1]);
                    self.expr(k, 0);
                    self.doc.word(": ");
                    self.expr(v, 0);
                } else {
                    self.expr(kv, 0);
                }
            }
            self.doc.word(")");
        }
        self.doc.word(" ");
        self.expr(binder[0], 0); // param name
        self.doc.word(" : ");
        self.expr(binder[1], PREC_ARROW); // declared type (bind tighter than `->` chains)
        self.doc.end();
    }

    /// `handle E(seed) with | op(p…, state) => body … in body` — the effect-handler surface. `args` is
    /// `effect seed (arm…) body`. The effect name and seed promote into the head (`E(seed)`, or bare
    /// `E` when the seed is the stateless `unit`); each arm `(op (p…) state body)` renders
    /// `op(p…, state) => body`, the state binder LAST in the binder list. Mirrors `match`'s `|`-led
    /// arms and `let`'s `… in body` tail. Parenthesizes as a block form when `parent_prec > 0`.
    fn print_handle(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        let (effect, seed, arms_occ, body) = (args[0], args[1], args[2], args[3]);
        self.doc.cbox(0);
        if paren {
            self.doc.word("(");
        }
        self.doc.cbox(INDENT);
        self.doc.word("handle ");
        self.expr(effect, 0); // effect name
        // A `unit` seed is the stateless degenerate case — elide the `(seed)`. Any other seed prints
        // as `E(seed)`.
        if self.head_name(seed).as_deref() != Some("unit") {
            self.doc.word("(");
            self.expr(seed, 0);
            self.doc.word(")");
        }
        self.doc.word(" with");
        // Arms are `|`-led, one per line, indented under the `handle` — the same shape as a `match`'s
        // arms. The arm box closes before `in` so `in` returns to the `handle` column.
        if let Struct::List(arms) = self.a.get(arms_occ) {
            let arms = arms.clone();
            for (i, &arm) in arms.iter().enumerate() {
                self.doc.hardbreak();
                self.doc.word("| ");
                // A NON-LAST arm whose body is a greedy block form (`match`/`let`/`if`/…) must
                // parenthesize, else its own `|`-led arms / trailing body run into the next `| op`
                // handler arm on re-parse (the arm-extent ambiguity: an inner `match`'s arms and the
                // outer handler's arms are both pipe-prefixed at the same column). The LAST arm needs
                // no guard — `in` terminates it. Symmetric with `print_match`'s non-last-arm guard.
                let last = i + 1 == arms.len();
                self.print_handle_arm(arm, last);
            }
        }
        self.doc.end();
        // `in` on its own line at the `handle` column, then the body on the next line at that column —
        // the `let … in` idiom, so a `handle` at the tail of a def body reads as a flat sequence.
        self.doc.hardbreak();
        self.doc.word("in");
        self.doc.hardbreak();
        // The `in`-body is on its own line → a `match`/`handle` body flushes its arms (seq-96/97).
        self.flush_match_arms = self.head_is_match_form(self.a.peel_comments(body));
        self.expr(body, 0);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// One handler arm `(op (p…) state body)` -> `op(p…, state) => body`. The state binder is appended
    /// as the LAST entry of the parenthesized binder list (symmetric with `resume(value, state)`).
    fn print_handle_arm(&mut self, arm: StructId, last: bool) {
        let Struct::List(parts) = self.a.get(arm) else {
            return self.expr(arm, 0);
        };
        let parts = parts.clone();
        // parts = op, (params…), state, body
        let (op, params_occ, state, body) = (parts[0], parts[1], parts[2], parts[3]);
        self.expr(op, 0); // bare operation name
        self.doc.word("(");
        let params: Vec<StructId> = match self.a.get(params_occ) {
            Struct::List(ps) => ps.clone(),
            _ => vec![params_occ],
        };
        for &p in &params {
            self.expr(p, 0);
            self.doc.word(", ");
        }
        self.expr(state, 0); // the state binder, last
        self.doc.word(") =>");
        // A non-last arm's greedy block-form body (`match`/`let`/`if`/…) parenthesizes so its arms
        // don't run into the next `| op` handler arm on re-parse; the last arm's body is terminated
        // by `in`, so it needs no guard. `PREC_KEYWORD` forces block-form parens without wrapping an
        // infix body — identical to `print_match`'s non-last-arm treatment. A bare-`|` (bitwise-or) body
        // parenthesizes for EVERY arm (even the last: a bare `|` still starts a phantom arm before `in`)
        // via `PREC_PIPE_PAREN` (> `|`'s infix prec, so the infix itself parens).
        let body_prec = if self.arm_body_is_bare_pipe_infix(body) {
            PREC_PIPE_PAREN
        } else if last {
            0
        } else {
            PREC_KEYWORD
        };
        self.print_arm_body(body, body_prec);
    }

    /// Emit a match/handle arm body after `=>` (already printed, no trailing space). Layout:
    ///   • `body_prec > 0` — the body PARENTHESIZES (a non-last open-`|`-arm-form tail via `PREC_KEYWORD`,
    ///     or a bare-`|` infix via `PREC_PIPE_PAREN`): explicit parens with a consistent box (operator
    ///     seq-95) — `=> (` on the arm line, the body indented one level, the close `)` on its OWN line
    ///     dedented to the arm indent. A SINGLE-LINE paren body stays inline (`(x | 8)`); a multi-line one
    ///     breaks. The body prints BARE (prec 0) — the explicit parens delimit it, so a trailing open-arm
    ///     form can't absorb the following `| pat`.
    ///   • otherwise a MULTI-LINE body drops to its own indented line, a SINGLE-LINE stays inline after
    ///     `=>` (seq-86/87/89 + #6335): a bare `let` forces the break (its `in`-body always breaks); a
    ///     body with a LEADING `//` comment keeps the comment on the `=>` line (`print_comment` then
    ///     breaks); any other body soft-breaks (inline when it fits, else it WRAPS to the indented line).
    fn print_arm_body(&mut self, body: StructId, body_prec: u8) {
        let peeled = self.a.peel_comments(body);
        // The seq-95 explicit-paren LAYOUT applies ONLY when the body ACTUALLY parenthesizes at
        // `body_prec` — a block form (`if`/`let`/`match`/`fn`/`handle`/`host`, which paren at
        // `PREC_KEYWORD` = 1) or a bare-`|` infix (`PREC_PIPE_PAREN`). A single-expression body
        // (call/var/literal/simple infix) does NOT paren at `PREC_KEYWORD` (operator seq-101 — no
        // UNNEEDED parens): it takes the bare path below, where `expr(body, body_prec)` still emits the
        // right parens for anything mis-classified here, so this stays correctness-safe (a missed block
        // form just renders with the pre-seq-95 glued-paren layout, never a broken round-trip).
        let paren_layout = body_prec == PREC_PIPE_PAREN
            || (body_prec == PREC_KEYWORD && self.head_is_block_form(peeled));
        // A `match`/`handle` arm body starts its own line (under `=>` or inside the paren-wrap), so its
        // arms flush with its keyword (seq-96/97). Set the one-shot per body (false for a non-match body,
        // so it never leaks to a value-position match nested inside).
        let flush = self.head_is_match_form(peeled);
        if paren_layout {
            self.doc.word(" (");
            self.doc.cbox(INDENT);
            self.doc.zerobreak();
            self.flush_match_arms = flush;
            self.expr(body, 0);
            self.doc.break_with(0, -INDENT);
            self.doc.word(")");
            self.doc.end();
            return;
        }
        self.doc.cbox(INDENT);
        let has_lead_comment = self.a.as_form(body, "comment").is_some();
        if !has_lead_comment && self.is_let_shape_form(peeled) {
            self.doc.hardbreak();
        } else if has_lead_comment {
            self.doc.word(" ");
        } else {
            self.doc.space();
        }
        self.flush_match_arms = flush;
        self.expr(body, body_prec);
        self.doc.end();
    }

    /// True if `id` prints as a BLOCK FORM — `if`/`let`/`match`/`fn`/`handle`/`host` — the forms whose
    /// printers wrap in parens at any operand precedence (`parent_prec > 0`, so `PREC_KEYWORD` = 1 wraps
    /// them but never an infix or a call). Used to decide whether a parenthesized arm body gets the seq-95
    /// `=> ( … )` layout (a call/var/literal body does not, so it stays bare — operator seq-101).
    fn head_is_block_form(&self, id: StructId) -> bool {
        let head = match self.a.get(id) {
            Struct::List(items) => items.first().copied(),
            _ => None,
        };
        matches!(
            head.and_then(|h| self.head_name(h)).as_deref(),
            Some("if" | "let" | "match" | "fn" | "handle" | "host")
        )
    }

    /// True if `id` is a `match`/`handle` form — the arm-bearing constructs whose `|` arm alignment the
    /// seq-96/97 flush rule governs. A statement emitter sets `flush_match_arms` to THIS on the body it is
    /// about to print, so a match/handle that starts its own line flushes its arms, while a value-position
    /// match nested inside (a call arg, an operand) stays at the default indent (the assignment is `false`
    /// for a non-match body, so the flag never leaks past this body).
    fn head_is_match_form(&self, id: StructId) -> bool {
        let head = match self.a.get(id) {
            Struct::List(items) => items.first().copied(),
            _ => None,
        };
        matches!(
            head.and_then(|h| self.head_name(h)).as_deref(),
            Some("match" | "handle")
        )
    }

    /// `host E, … in body` — an entrypoint delegation. `args` is `(E …) body`; the effects render as a
    /// comma-separated name list, the body after `in`. Mirrors `handle`'s `… in body` tail.
    fn print_host(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        let (effects_occ, body) = (args[0], args[1]);
        self.doc.cbox(0);
        if paren {
            self.doc.word("(");
        }
        self.doc.word("host ");
        if let Struct::List(effects) = self.a.get(effects_occ) {
            let effects = effects.clone();
            for (i, &e) in effects.iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.expr(e, 0);
            }
        }
        // `host E in body` stays on one line when it fits, else `in` and the body break to fresh lines
        // at the `host` column — the `let … in` idiom.
        self.doc.space();
        self.doc.word("in");
        self.doc.space();
        // A `match`/`handle` `in`-body on its own line flushes its arms (seq-96/97).
        self.flush_match_arms = self.head_is_match_form(self.a.peel_comments(body));
        self.expr(body, 0);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// An `(effect Name (op <op> <ty>)…)` the `effect Name = | … ` surface handles: a name head, then
    /// AT LEAST ONE `(op <name> <ty>)` operation form (each a 3-element list headed `op` with a name
    /// operation). The `|`-led surface can't spell an op-less effect, so a bare `(effect Name)` falls
    /// back to the generic call form to still round-trip; anything else does too.
    fn is_effect_shape(&self, args: &[StructId]) -> bool {
        if args.len() < 2 || self.head_name(args[0]).is_none() {
            return false;
        }
        // A name head, then optional leading `(doc …)` forms, then AT LEAST ONE `(op …)`. The docs
        // splice inside the effect decl exactly as they do for a `type` decl.
        let docs_end = 1 + args[1..].iter().take_while(|&&a| self.is_doc(a)).count();
        let ops = &args[docs_end..];
        !ops.is_empty()
            // Peel any `(comment …)`/`(comment-after …)` wrapper the reader attaches to an op (seq-277):
            // a comment-wrapped op is still an op, so it must NOT knock the effect decl to the generic
            // call form (which drops the comment). `print_effect` peels the same wrapper when printing.
            && ops.iter().all(|&op| match self.a.as_form(self.a.peel_comments(op), "op") {
                // `(op <name> <ty>)` OR `(op <name> <ty> (resource N))` — the optional trailing
                // `(resource N)` is the SEC-F1 marker the op printer resugars as `@resource` (it must not
                // knock the effect decl back to the generic call form). The `o.len()` gate MUST precede the
                // `o[0]` index: a malformed `(op)` with ZERO children yields `o == []`, so an `o[0]` before
                // the length check panics (index-out-of-bounds) on the ML round-trip. With the length check
                // first and short-circuiting, a zero/one-child op returns false here → the whole effect
                // degrades to the generic call form (round-trips), preserving printer totality.
                Some(o) => {
                    (o.len() == 2
                        || (o.len() == 3 && self.a.as_form(o[2], "resource").is_some()))
                        && self.head_name(o[0]).is_some()
                }
                None => false,
            })
    }

    /// A `(world Name (import|export Iface (member M (func …))…)…)` the inline `world …` surface handles:
    /// a NAME head, optional `(doc …)` forms, then AT LEAST ONE interface (each `import`/`export`-headed
    /// with a name + members). Each member is `(member <name> (func …))`, each func a `(param <n> <t>)*`
    /// then a `(result <t>)`. Anything else (a bare `(world Name)`, a malformed member) falls back to the
    /// generic call form so it still round-trips. The dual of `world_expr`'s grammar.
    fn is_world_shape(&self, args: &[StructId]) -> bool {
        if args.len() < 2 || self.head_name(args[0]).is_none() {
            return false;
        }
        let docs_end = 1 + args[1..].iter().take_while(|&&a| self.is_doc(a)).count();
        let ifaces = &args[docs_end..];
        !ifaces.is_empty()
            && ifaces.iter().all(|&i| {
                let entry = self
                    .a
                    .as_form(i, "import")
                    .or_else(|| self.a.as_form(i, "export"));
                match entry {
                    // (import|export IfaceName (member …)…): a name then ≥1 well-shaped member.
                    Some(e) => {
                        e.len() >= 2
                            && self.head_name(e[0]).is_some()
                            && e[1..].iter().all(|&m| self.is_world_member_shape(m))
                    }
                    None => false,
                }
            })
    }

    /// A `(member <name> (func (param <n> <t>)* (result <t>)))` — a member name then a func node whose
    /// children are zero-or-more `(param name type)` then exactly one trailing `(result type)`.
    fn is_world_member_shape(&self, m: StructId) -> bool {
        let Some(mem) = self.a.as_form(m, "member") else {
            return false;
        };
        if mem.len() != 2 || self.head_name(mem[0]).is_none() {
            return false;
        }
        let Some(func) = self.a.as_form(mem[1], "func") else {
            return false;
        };
        // Last child is the result; all earlier children are params.
        let Some((&result, params)) = func.split_last() else {
            return false;
        };
        self.a
            .as_form(result, "result")
            .is_some_and(|r| r.len() == 1)
            && params.iter().all(|&p| {
                self.a
                    .as_form(p, "param")
                    .is_some_and(|pp| pp.len() == 2 && self.head_name(pp[0]).is_some())
            })
    }

    /// A `(handle E seed (arm…) body)` the `handle E(seed) with … in body` surface handles: an effect
    /// NAME head, a seed, an arms LIST (each arm a 4-element `(op (params…) state body)` whose op and
    /// params are well-shaped), and a body. Anything else falls back to the generic call form.
    fn is_handle_shape(&self, args: &[StructId]) -> bool {
        if args.len() != 4 || self.head_name(args[0]).is_none() {
            return false;
        }
        let Struct::List(arms) = self.a.get(args[2]) else {
            return false;
        };
        !arms.is_empty()
            && arms.iter().all(|&arm| match self.a.get(arm) {
                Struct::List(parts) => {
                    parts.len() == 4
                        && self.head_name(parts[0]).is_some() // bare operation name
                        && matches!(self.a.get(parts[1]), Struct::List(_)) // params list
                }
                _ => false,
            })
    }

    /// A `(host (E…) body)` the `host E, … in body` surface handles: an effects LIST (at least one
    /// name) and a body. Anything else falls back to the generic call form.
    fn is_host_shape(&self, args: &[StructId]) -> bool {
        if args.len() != 2 {
            return false;
        }
        match self.a.get(args[0]) {
            Struct::List(effects) => {
                !effects.is_empty() && effects.iter().all(|&e| self.head_name(e).is_some())
            }
            _ => false,
        }
    }

    /// A `(type …)` the `type Name = …` surface handles: a name, zero or more `(doc "…")` forms, then
    /// at least one variant (a `Name` atom, or a `(Ctor T…)` list whose head is a name). Anything else
    /// falls back to the generic call form so it still round-trips.
    fn is_type_shape(&self, args: &[StructId]) -> bool {
        if args.len() < 2 || self.head_name(args[0]).is_none() {
            return false;
        }
        let docs_end = 1 + args[1..].iter().take_while(|&&a| self.is_doc(a)).count();
        let variants = &args[docs_end..];
        !variants.is_empty()
            && variants.iter().all(|&raw| {
                // A variant may be wrapped in a LEADING `(comment …)` (own-line `//` above it) and/or a
                // TRAILING `(comment-after …)` (same-line `//`) — peel both before checking its shape, so
                // a commented variant still counts (else the whole type falls to the backtick call form).
                let v = self.strip_field_comments(raw);
                match self.a.get(v) {
                    // nullary: a bare constructor name `A`
                    Struct::Atom(_) => self.head_name(v).is_some(),
                    // a name-headed list variant: `(A)` (nullary, the empty-parens spelling `A()`, len 1)
                    // or `(Ctor T …)` (payload, len >= 2). BOTH are valid — a nullary variant has two
                    // arena spellings (bare atom `A`, and 1-elem list `(A)` from `A()`), so requiring len
                    // >= 2 here wrongly rejected `(A)` and forced the whole type into the backtick-fallback
                    // render (`` `type`(T, A(), …) ``), which does not round-trip under an
                    // `@invariant`/annotation wrapper (v-verification). Accept the 1-elem nullary too.
                    Struct::List(items) => !items.is_empty() && self.head_name(items[0]).is_some(),
                }
            })
    }

    /// A bracketed, comma-separated sequence with all-or-nothing breaking: `open a, b, c close`
    /// inline if it fits, else one item per line block-indented with the close on its own dedented
    /// line. `emit` renders one item. Braces get inner padding (`{ … }`); brackets/parens do not.
    fn bracketed<T: Copy>(
        &mut self,
        open: &str,
        close: &str,
        pad: bool,
        items: &[T],
        mut emit: impl FnMut(&mut Self, T),
    ) {
        self.doc.cbox(INDENT);
        self.doc.word(open.to_string());
        if items.is_empty() {
            self.doc.word(close.to_string());
            self.doc.end();
            return;
        }
        // opening break: a space inside braces when flat (`{ x }`), nothing for `[`/`(`.
        self.doc.break_with(if pad { 1 } else { 0 }, 0);
        for (i, &item) in items.iter().enumerate() {
            if i > 0 {
                self.doc.word(",");
                self.doc.space();
            }
            emit(self, item);
        }
        // closing break: dedent to the open column; a padding space when flat if `pad`.
        self.doc.break_with(if pad { 1 } else { 0 }, -INDENT);
        self.doc.word(close.to_string());
        self.doc.end();
    }

    /// Like [`Self::bracketed`], but a `Leaf::Name("..")` marker in `items` glues to the FOLLOWING
    /// item as a single `.. rest` comma-slot (rendered by `emit_rest`) — the inverse of the reader's
    /// flat `… ".." rest` rest/spread shape. An ordinary item renders via `emit`. Used by the list/map
    /// construction literals AND their patterns, so `..` reads back uniformly in every position.
    fn bracketed_rest(
        &mut self,
        open: &str,
        close: &str,
        pad: bool,
        items: &[StructId],
        mut emit: impl FnMut(&mut Self, StructId),
        mut emit_rest: impl FnMut(&mut Self, StructId),
    ) {
        self.doc.cbox(INDENT);
        self.doc.word(open.to_string());
        if items.is_empty() {
            self.doc.word(close.to_string());
            self.doc.end();
            return;
        }
        self.doc.break_with(if pad { 1 } else { 0 }, 0);
        let mut i = 0;
        let mut first = true;
        while i < items.len() {
            if !first {
                self.doc.word(",");
                self.doc.space();
            }
            first = false;
            // A `..` rest/spread marker. The WRAPPED form `(.. operand)` (a list headed by `..`, the
            // canonical `(.. v)`-migration shape) carries its operand as a child, so it spans ONE slot;
            // the legacy FLAT form (a bare `Name("..")`) takes the NEXT sibling as its operand, spanning
            // TWO. A marker with no operand is malformed input — render a bare `..` so nothing is dropped.
            if let Some(args) = self.a.as_form(items[i], "..") {
                self.doc.word(".. ");
                emit_rest(self, args.first().copied().unwrap_or(items[i]));
                i += 1;
            } else if self.a.as_name(items[i]) == Some("..") && i + 1 < items.len() {
                self.doc.word(".. ");
                emit_rest(self, items[i + 1]);
                i += 2;
            } else {
                emit(self, items[i]);
                i += 1;
            }
        }
        self.doc.break_with(if pad { 1 } else { 0 }, -INDENT);
        self.doc.word(close.to_string());
        self.doc.end();
    }

    /// Whether `items` contains a rest/spread `..` marker — in EITHER the wrapped `(.. operand)` node or
    /// the legacy flat `Name("..")`+sibling shape — so a list/map/record/set carrying one renders through
    /// the rest-aware path (`bracketed_rest`) rather than the plain `bracketed`. Routes through the shared
    /// Phase-1 recognizer [`Arenas::rest_marker`], which sees both shapes; a bare `head_name` scan would
    /// miss the wrapped form (`head_name` names only atom leaves, not a list headed by `..`).
    fn has_rest_marker(&self, items: &[StructId]) -> bool {
        self.a.rest_marker(items).is_some()
    }

    /// `b[<segment>, …]` — a binary literal, the surface for the `(bin <segment> …)` grammar form (the
    /// inverse of the parser's `bin_literal`/`bin_pattern`). Each segment is an ordinary call-shaped
    /// expression/pattern (`u16(258)`, `bits(1, 1)`, `bytes(rest)`), printed at `0`; the empty form
    /// `(bin)` prints as `b[]`. The `[` is glued to `b` (the lexer emits one `BinOpen` token), so the
    /// open delimiter is the literal string `b[`.
    fn print_bin(&mut self, segs: &[StructId]) {
        // Comment-aware like the list literal: a leading `(comment …)` on a segment prints on its own
        // line above it (via `expr`), and a same-line trailing `(comment-after …)` on the LAST segment
        // re-prints `seg // text` with `]` forced to its own line. A non-last `comment-after` (decoded
        // AST only) declines the sugar → generic call form (round-trips). `strip_comments` peels both.
        self.bracketed_comment_aware("b[", "]", false, segs);
    }

    /// A tagged template `(tagged-template <tag> (chunks <str>…) (holes <expr>…))` renders back to the
    /// glued surface `tag"…"`. B1 (hole-free): the single chunk is the whole body, escaped, between the
    /// quotes; the tag name is glued directly before the opening `"` (the reader re-lexes the glued
    /// ident+string as one `TaggedTemplate` token). Holes `{expr}` are the next brick — until then a
    /// node with any hole falls the `is_tagged_template_shape` guard and prints as a generic call (its
    /// structure stays visible rather than round-tripping to garbage).
    fn print_tagged_template(&mut self, args: &[StructId]) {
        let tag = self.a.as_name(args[0]).unwrap_or("");
        let chunks: Vec<StructId> = match self.a.get(args[1]) {
            Struct::List(items) => items[1..].to_vec(), // drop the "chunks" head
            _ => Vec::new(),
        };
        let holes: Vec<StructId> = match self.a.get(args[2]) {
            Struct::List(items) => items[1..].to_vec(), // drop the "holes" head
            _ => Vec::new(),
        };
        // Reassemble `tag"chunk0{hole0}chunk1…chunkN"` — chunks and holes interleave, one more chunk than
        // holes (the guard guarantees chunks.len() == holes.len() + 1). Each literal chunk re-escapes its
        // string content AND its braces (`{`→`{{`, `}`→`}}`) so a literal brace round-trips (never
        // re-read as a hole); each hole prints as `{<expr>}`.
        self.doc.word(format!("{}\"", emit_name(tag)));
        for (i, &chunk) in chunks.iter().enumerate() {
            let s = self.a.as_str(chunk).unwrap_or("");
            self.doc.word(escape_template_chunk(s));
            if let Some(&hole) = holes.get(i) {
                self.doc.word("{");
                self.expr(hole, 0);
                self.doc.word("}");
            }
        }
        self.doc.word("\"");
    }

    /// Whether `args` is a tagged-template node this printer can re-sugar to `tag"…"`: shape
    /// `[<tag-name>, (chunks <str>…), (holes <expr>…)]`, every chunk a `Str`, and the invariant
    /// chunks.len() == holes.len() + 1. An odd shape returns false so it prints as a generic call
    /// (structure visible, never garbage) rather than a form that would not round-trip.
    fn is_tagged_template_shape(&self, args: &[StructId]) -> bool {
        if args.len() != 3 {
            return false;
        }
        // The tag must be a BARE-SAFE name: the `tag"…"` sugar glues the tag directly before the quote,
        // and the lexer only re-lexes an ident (not a backtick-escaped name) glued to `"` as a
        // TaggedTemplate. A non-bare tag would print as `weird`"…" via `emit_name`, which does NOT
        // re-lex — a garbage render (PR #405). So gate the sugar on bare-safety; a non-bare tag falls
        // through to the generic `(tagged-template …)` call form, which round-trips
        // ([[garbage-render-means-not-canonical-fix-the-source]]).
        match self.a.as_name(args[0]) {
            Some(tag) if name_is_bare_safe(tag) => {}
            _ => return false,
        }
        // Note: `self.a.head_name` reads a LIST's head atom (the Arenas helper); the printer's local
        // `self.head_name` takes an atom id and would return None for these list nodes.
        let n_chunks = match self.a.get(args[1]) {
            Struct::List(items)
                if self.a.head_name(args[1]) == Some("chunks")
                    && items[1..].iter().all(|&c| self.a.as_str(c).is_some()) =>
            {
                items.len() - 1
            }
            _ => return false,
        };
        let n_holes = match self.a.get(args[2]) {
            Struct::List(items) if self.a.head_name(args[2]) == Some("holes") => items.len() - 1,
            _ => return false,
        };
        n_chunks == n_holes + 1
    }

    /// `[e, …]`, with an optional `.. rest` spread (`[1, 2, .. rest]`).
    fn print_list_literal(&mut self, elems: &[StructId]) {
        if self.has_rest_marker(elems) {
            return self.bracketed_rest(
                "[",
                "]",
                false,
                elems,
                |p, e| p.print_elem_maybe_commented(e),
                |p, e| p.print_elem_maybe_commented(e),
            );
        }
        self.bracketed_comment_aware("[", "]", false, elems);
    }

    /// A `bracketed` variant that renders each element via `print_elem_maybe_commented` (so a
    /// `(comment-after "text" elem)` element re-prints its `//` SAME-LINE) and, when the LAST element
    /// carries such a same-line comment, FORCES the closing delimiter onto the next line. The forced break
    /// is essential: a same-line `//` runs to end-of-line, so a flat `[…, x // note]` would swallow the
    /// `]` INTO the comment (`// note]`) and the printed form would fail to re-parse. A container with no
    /// trailing comment on its last element keeps the ordinary flat/soft-break layout via `bracketed`.
    /// Shared by the list and tuple literals (records/maps wrap fields as pairs — a separate follow-up).
    fn bracketed_comment_aware(&mut self, open: &str, close: &str, pad: bool, elems: &[StructId]) {
        // A NON-last `(comment-after …)` element has NO faithful same-line rendering — `print_elem_maybe_
        // commented` would emit `elem // text` and the following `, next` would be swallowed into the
        // comment line → invalid re-parse (PR#763/#781; only a decoded / metaprogramming-built AST can
        // produce this — the reader gates capture to the last element). Render every element via bare
        // `expr` so a `comment-after` prints as a `comment-after(...)` CALL, which round-trips. Guarding
        // HERE defends every caller (bin dispatch is unguarded; list/tuple/set guard at dispatch too, so
        // this is belt-and-suspenders for them). PR#781 (Copilot): `print_bin` reached here unguarded.
        if self.has_nonlast_comment_after(elems) {
            return self.bracketed(open, close, pad, elems, |p, e| p.expr(e, 0));
        }
        let last_has_trailing = elems.last().is_some_and(|&e| self.is_comment_after(e));
        if !last_has_trailing {
            return self.bracketed(open, close, pad, elems, |p, e| {
                p.print_elem_maybe_commented(e)
            });
        }
        self.doc.cbox(INDENT);
        self.doc.word(open.to_string());
        self.doc.break_with(if pad { 1 } else { 0 }, 0);
        for (i, &e) in elems.iter().enumerate() {
            if i > 0 {
                self.doc.word(",");
                self.doc.space();
            }
            self.print_elem_maybe_commented(e);
        }
        // Hard newline before the closer so the trailing `// …` on the last element ends its line.
        self.doc.hardbreak_with(-INDENT);
        self.doc.word(close.to_string());
        self.doc.end();
    }

    /// True if `id` is a `(comment-after "text" inner)` wrapper — a same-line trailing `//` comment node.
    fn is_comment_after(&self, id: StructId) -> bool {
        matches!(self.a.as_form(id, "comment-after"), Some(a) if a.len() == 2 && self.is_string(a[0]))
    }

    /// True if any element EXCEPT the last is a same-line trailing `(comment-after …)` wrapper. Such a node
    /// only ever arises from a decoded / metaprogramming-built AST (the reader gates its capture to the
    /// last element), and it has NO faithful inline rendering: `elem // text , next` would swallow the `,`
    /// into the comment, and `elem, // text` re-reads the comment as LEADING the next element (a different
    /// tree). So a container carrying one must DECLINE its sugared literal surface and fall back to the
    /// generic call render (which round-trips `comment-after(...)` faithfully). Prevents the printer-side
    /// PR#758 break (PR#763 / Copilot: a printer guard must be correct on ANY AST, not just the reader's).
    fn has_nonlast_comment_after(&self, elems: &[StructId]) -> bool {
        elems.len() > 1
            && elems[..elems.len() - 1]
                .iter()
                .any(|&e| self.is_comment_after(e))
    }

    /// Print one collection-literal element, unwrapping a `(comment-after "text" elem)` wrapper (a `//`
    /// that trailed the element on the same source line, e.g. `[1, 2 // last]` / `(1, 2 // last)`) so it
    /// re-emits SAME-LINE as `elem // text` — mirroring `print_variant`'s trailing-comment handling. A
    /// plain element prints via `expr`. Without this the wrapper would render as a spurious
    /// `comment-after(...)` CALL. (Interior own-line comments and `///` docs inside a literal are a
    /// separate, broader gap — see queue `gap-trailing-and-interior-comment-in-collection-literals-dropped`.)
    fn print_elem_maybe_commented(&mut self, e: StructId) {
        if let Some(a) = self.a.as_form(e, "comment-after")
            && a.len() == 2
            && self.is_string(a[0])
        {
            self.expr(a[1], 0);
            self.doc.word(format!(" //{}", self.doc_line_text(a[0])));
            return;
        }
        self.expr(e, 0);
    }

    /// `(e, …)` — a tuple. A 1-element tuple prints as `(e,)` (a trailing comma, Rust-style), which
    /// distinguishes it from `(e)` transparent grouping; 2+ elements print `(e, f, …)`.
    fn print_tuple(&mut self, elems: &[StructId]) {
        if elems.len() == 1 {
            // `(e,)` — the 1-tuple (trailing comma distinguishes it from `(e)` grouping). A same-line
            // comment on the sole element is an awkward rare edge: the `,` is structural and would sit
            // between the element and its comment (`(e, // note)`), a slot the reader does not round-trip
            // — so it is NOT special-cased here; it falls through to the comment-drop guard (fmt refuses,
            // no corruption), like the module empty-body-comment case. The clean `(e,)` prints as before.
            self.doc.word("(");
            self.expr(elems[0], 0);
            self.doc.word(",)");
            return;
        }
        self.bracketed_comment_aware("(", ")", false, elems);
    }

    /// `{ name = e, … }`, with field SHORTHAND: a field whose value is a bare-name reference to the
    /// field's own name (`(x x)`) prints as just `{ x }` (the inverse of the reader's `{ x }` → `(x x)`
    /// pun). A field with any other value prints the full `name = value`.
    fn print_record(&mut self, fields: &[StructId]) {
        let field = |p: &mut Self, field: StructId| {
            // A value-record field is the canonical `(= name value)` triple (RV2, DESIGN-record-type-
            // syntax Phase B) — read name/value from children 1/2, dropping the `=` head; the printed
            // SURFACE `{ name = value }` is UNCHANGED (only the arena gained the explicit `=`). Tolerate
            // the legacy bare `(name value)` pair too, so a stray un-migrated node still prints.
            let (name, value) = match p.a.get(field) {
                Struct::List(items)
                    if items.len() == 3 && p.head_name(items[0]).as_deref() == Some("=") =>
                {
                    (items[1], items[2])
                }
                Struct::List(pair) if pair.len() == 2 => (pair[0], pair[1]),
                _ => return,
            };
            p.expr(name, 0);
            if !p.is_field_pun(name, value) {
                p.doc.word(" = ");
                p.expr(value, 0);
            }
        };
        // A record CONSTRUCTION spread (`{ ..base, a = 1 }`) carries a flat `Name("..")` marker among the
        // field triples — render through the rest-aware path (`.. base`), the twin of the map/list spread.
        if self.has_rest_marker(fields) {
            return self.bracketed_rest("{", "}", true, fields, field, |p, e| p.expr(e, 0));
        }
        self.bracketed_pairs_comment_aware("{", "}", fields, field);
    }

    /// Like `bracketed` (padded braces) but each field/entry may be wrapped in `(comment-after "text"
    /// (pair))` — a same-line trailing `//` on the LAST field/entry. Renders the inner pair via `emit`,
    /// then ` // text` same-line, and (when the last field is wrapped) forces the closing `}` onto its own
    /// line so the comment isn't swallowed. Records/maps use this instead of the bare-value
    /// `bracketed_comment_aware` because their element is a `(name value)` PAIR, not a bare value.
    fn bracketed_pairs_comment_aware(
        &mut self,
        open: &str,
        close: &str,
        fields: &[StructId],
        mut emit: impl FnMut(&mut Self, StructId),
    ) {
        let last_has_trailing = fields.last().is_some_and(|&f| self.is_comment_after(f));
        // An own-line LEADING `(comment …)` on any field forces the container to break (the comment prints
        // on its own line above the field), same as a last-field trailing comment forces the closer break.
        let any_leading = fields.iter().any(|&f| {
            self.a
                .as_form(f, "comment")
                .is_some_and(|a| a.len() == 2 && self.is_string(a[0]))
        });
        let emit_field = |p: &mut Self, f: StructId, emit: &mut dyn FnMut(&mut Self, StructId)| {
            // Peel EVERY comment wrapper around the field down to the inner `(name value)` pair, in a LOOP
            // (a field may carry more than one — a decoded / metaprogramming-built AST can nest
            // `(comment c1 (comment c2 (pair)))` or interleave leading/trailing; `is_pairs` accepts any
            // via `strip_field_comments`, so the printer must handle all of them — PR#768/PR#763-class:
            // the printer must be TOTAL, not just cover the reader's single-comment-per-field output).
            // LEADING `(comment …)` → print `// text` + hardbreak ABOVE the field (own-line). TRAILING
            // `(comment-after …)` → collect its text to emit AFTER the pair (innermost trails closest).
            let mut inner = f;
            let mut trailing_texts: Vec<StructId> = Vec::new();
            loop {
                if let Some(a) = p.a.as_form(inner, "comment")
                    && a.len() == 2
                    && p.is_string(a[0])
                {
                    p.doc.word(format!("//{}", p.doc_line_text(a[0])));
                    p.doc.hardbreak();
                    inner = a[1];
                    continue;
                }
                if let Some(a) = p.a.as_form(inner, "comment-after")
                    && a.len() == 2
                    && p.is_string(a[0])
                {
                    trailing_texts.push(a[0]);
                    inner = a[1];
                    continue;
                }
                break;
            }
            emit(p, inner);
            // Innermost `comment-after` is closest to the pair on the source line, so print in reverse of
            // collection order (`… // outer // inner` reads as inner-then-outer up the nesting).
            for &text in trailing_texts.iter().rev() {
                p.doc.word(format!(" //{}", p.doc_line_text(text)));
            }
        };
        if !last_has_trailing && !any_leading {
            return self.bracketed(open, close, true, fields, |p, f| {
                emit_field(p, f, &mut emit)
            });
        }
        // Forced-break path (last field carries a same-line comment): padded braces, hard newline before
        // the closer so the trailing `// …` on the last field ends its line.
        self.doc.cbox(INDENT);
        self.doc.word(open.to_string());
        self.doc.break_with(1, 0);
        for (i, &f) in fields.iter().enumerate() {
            if i > 0 {
                self.doc.word(",");
                self.doc.space();
            }
            emit_field(self, f, &mut emit);
        }
        self.doc.hardbreak_with(-INDENT);
        self.doc.word(close.to_string());
        self.doc.end();
    }

    /// Whether a record field `(name value)` is a SHORTHAND pun — `value` is a bare-`Name` reference
    /// spelled identically to `name` (also a bare `Name`), so `{ name = value }` collapses to `{ name }`.
    fn is_field_pun(&self, name: StructId, value: StructId) -> bool {
        matches!((self.a.as_name(name), self.a.as_name(value)), (Some(n), Some(v)) if n == v)
    }

    /// `export { name, … }` — the module's public surface as a brace-delimited name list. `args` are
    /// the `(export name…)` form's exported names (all bare names, per `is_export_shape`).
    fn print_export(&mut self, args: &[StructId]) {
        self.doc.word("export ");
        self.print_name_group(args);
    }

    /// `import { name, … } from "path"` — brings a sibling module's public names into scope. `args`
    /// is `["path" (name…)]` (per `is_import_shape`): the path string then the name-list occurrence.
    fn print_import(&mut self, args: &[StructId]) {
        // Alias form: `(import "path" alias)` (bare-name third element) -> `import alias from "path"`
        // (a whole-module bind is a bare name where the named form has a `{ … }` list — both use `from`).
        if self.a.as_name(args[1]).is_some() {
            self.doc.word("import ");
            self.expr(args[1], 0); // the alias name
            self.doc.word(" from ");
            self.expr(args[0], 0); // the path string literal
            return;
        }
        // Named-list form: `(import "path" (name…))` -> `import { name, … } from "path"`.
        let names = match self.a.get(args[1]) {
            Struct::List(items) => items.clone(),
            _ => return,
        };
        self.doc.word("import ");
        self.print_name_group(&names);
        self.doc.word(" from ");
        self.expr(args[0], 0); // the path string literal
    }

    /// A brace-delimited comma-separated name group `{ a, b, … }`, all-or-nothing breaking — the
    /// surface shared by `export` and `import`. Each element prints as a bare (or escaped) name.
    fn print_name_group(&mut self, names: &[StructId]) {
        self.bracketed("{", "}", true, names, |p, name| {
            // A per-name import rename `(as orig alias)` prints `orig as alias`; a plain name (or an
            // export member `T.A`) prints via `expr`.
            if let Some([orig, alias]) = p.import_rename_parts(name) {
                p.expr(orig, 0);
                p.doc.word(" as ");
                p.expr(alias, 0);
            } else {
                p.expr(name, 0);
            }
        });
    }

    /// An `(export name…)` the `export { … }` surface handles: at least one arg, every arg either a
    /// bare name (`main`, a value or a type HANDLE) or a constructor-export member access — `(. T A)`
    /// (one constructor) or `(. T *)` (the wildcard, the whole constructor set). Each renders through
    /// `print_name_group`'s `expr` as `main` / `Color.Red` / `Color.*`, and the parser reads those back
    /// to the same forms (a member-access element inside `{ … }`), so the surface round-trips. A
    /// malformed export element falls back to the generic call form.
    fn is_export_shape(&self, args: &[StructId]) -> bool {
        !args.is_empty()
            && args
                .iter()
                .all(|&a| self.a.as_name(a).is_some() || self.is_export_member(a))
    }

    /// A constructor-export member-access element of an `(export …)` clause: `(. T A)` or `(. T *)`
    /// where `T` is a bare name and the key is a plain field name or the wildcard `*` — the forms
    /// `print_name_group` renders as `T.A` / `T.*` and the parser reads back inside `{ … }`.
    fn is_export_member(&self, id: StructId) -> bool {
        matches!(self.a.get(id), Struct::List(items)
            if items.len() == 3
                && self.head_name(items[0]).as_deref() == Some(".")
                && self.a.as_name(items[1]).is_some()
                && self.plain_key(items[2]).is_some())
    }

    /// An `(import "path" (name…))` the `import { … } from "path"` surface handles: a string path, a
    /// name-LIST of bare names. The alias form `(import "path" alias)` is handled by
    /// [`Self::is_import_alias_shape`]; any other shape falls back to the generic call form.
    fn is_import_shape(&self, args: &[StructId]) -> bool {
        args.len() == 2
            && self.is_string(args[0])
            && matches!(self.a.get(args[1]), Struct::List(names)
                if !names.is_empty()
                    && names.iter().all(|&n| self.a.as_name(n).is_some()
                        || self.import_rename_parts(n).is_some()))
    }

    /// A per-name import RENAME element `(as orig alias)` — a 3-list headed by the Name `as` with two
    /// name children — the `import { orig as alias, … }` surface. Returns `[orig, alias]` (the tail
    /// after the `as` head) for the shape check and the printer. `None` for a plain-name element.
    fn import_rename_parts(&self, id: StructId) -> Option<[StructId; 2]> {
        match self.a.get(id) {
            Struct::List(items)
                if items.len() == 3
                    && self.head_name(items[0]).as_deref() == Some("as")
                    && self.a.as_name(items[1]).is_some()
                    && self.a.as_name(items[2]).is_some() =>
            {
                Some([items[1], items[2]])
            }
            _ => None,
        }
    }

    /// The whole-module ALIAS import `(import "path" alias)` -> `import alias from "path"`: a string path
    /// then a bare NAME (the local alias the linker binds the module's exports record under). Distinct
    /// from the named-list `is_import_shape` by the third element being a NAME, not a LIST.
    fn is_import_alias_shape(&self, args: &[StructId]) -> bool {
        args.len() == 2 && self.is_string(args[0]) && self.a.as_name(args[1]).is_some()
    }

    /// `#{ key = v, … }`, with an optional `.. rest` spread (`#{ 1 = v, .. rest }`).
    fn print_map(&mut self, entries: &[StructId]) {
        let entry = |p: &mut Self, entry: StructId| {
            if let Struct::List(pair) = p.a.get(entry) {
                // A native FieldPair entry `(= key value)` (M2) — key/value are children 1/2, dropping the
                // `=` head; tolerate a legacy bare `(key value)` pair too.
                let (key, value) =
                    if pair.len() == 3 && p.head_name(pair[0]).as_deref() == Some("=") {
                        (pair[1], pair[2])
                    } else {
                        (pair[0], pair[1])
                    };
                p.expr(key, 0);
                p.doc.word(" = ");
                p.expr(value, 0);
            }
        };
        if self.has_rest_marker(entries) {
            return self.bracketed_rest("#{", "}", true, entries, entry, |p, e| p.expr(e, 0));
        }
        self.bracketed_pairs_comment_aware("#{", "}", entries, entry);
    }

    /// `match scrut { pat => body, … }` — one arm per line (consistent box) when broken.
    fn print_match(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        // Operator seq-96/97: a STATEMENT-position match (its `match` starts its own line — set by the
        // caller via `flush_match_arms`) aligns its `|` arms FLUSH with the `match` column (`cbox(0)`); a
        // BOUND match (inline after `def/let/=>`/`(`… — the default, and always the parenthesized/value
        // case) keeps them INDENTED one level. Take the one-shot NOW so it can't leak into the scrutinee
        // or a nested value-position match.
        let arms_indent = if std::mem::take(&mut self.flush_match_arms) && !paren {
            0
        } else {
            INDENT
        };
        self.doc.cbox(arms_indent);
        if paren {
            self.doc.word("(");
        }
        self.doc.word("match ");
        self.expr(args[0], 0);
        self.doc.word(" with");
        // Arms go one per line, each led by `| ` at the `match` column (OCaml style — the leading `|`
        // is always printed, including on the first arm). No braces, no trailing commas.
        let arms = &args[1..];
        for (i, &raw_arm) in arms.iter().enumerate() {
            // Peel ALL comment wrappers around the arm, in a LOOP over either nesting order (a decoded
            // AST may nest `(comment-after t (comment lead (pat body)))` or vice versa; `is_match_shape`
            // accepts any via `strip_field_comments`, so the printer must be total — PR#768-class). A
            // LEADING `(comment …)` prints as a `// …` line ABOVE the arm (before its `| `); a TRAILING
            // `(comment-after …)` is collected to print AFTER the body, same line.
            let mut arm = raw_arm;
            let mut lead_texts: Vec<StructId> = Vec::new();
            let mut trail_texts: Vec<StructId> = Vec::new();
            loop {
                if let Some(a) = self.a.as_form(arm, "comment")
                    && a.len() == 2
                    && self.is_string(a[0])
                {
                    lead_texts.push(a[0]);
                    arm = a[1];
                    continue;
                }
                if let Some(a) = self.a.as_form(arm, "comment-after")
                    && a.len() == 2
                    && self.is_string(a[0])
                {
                    trail_texts.push(a[0]);
                    arm = a[1];
                    continue;
                }
                break;
            }
            // Leading comments, each on its own line above the arm (outermost first — source top-down).
            for &text in &lead_texts {
                self.doc.hardbreak();
                self.doc.word(format!("//{}", self.doc_line_text(text)));
            }
            self.doc.hardbreak();
            self.doc.word("| ");
            if let Struct::List(pair) = self.a.get(arm) {
                let (pat, body) = (pair[0], pair[1]);
                self.pattern(pat);
                self.doc.word(" =>");
                // A NON-LAST arm body whose TRAILING sub-expression is an open `|`-arm list
                // (`match`/`handle`, possibly under `if`-else / `let`-body / `@` / comment wrappers) must
                // parenthesize, else the following `| pat` is absorbed into that inner arm list. Every
                // other body — `if`/`let`/`fn`/infix/call/literal, whose tail ends in a closing token —
                // is delimited by the arm's own `|` and prints BARE (parenthesizing it was the redundant-
                // paren defect: hm-collect.cdz's `(if …)`/`(let …)` match-arm bodies). The last arm needs
                // no guard (nothing follows). PREC_KEYWORD forces the block-form parens; 0 prints bare.
                let last = i + 1 == arms.len();
                // A bare-`|` (bitwise-or) body parenthesizes for EVERY arm (the `|` glyph would start a
                // phantom next arm) — forced with `PREC_PIPE_PAREN` (> `|`'s infix prec 7) so the INFIX
                // itself parenthesizes, since PREC_KEYWORD only wraps block forms not infixes. A greedy
                // open-arm-form tail parenthesizes only for a NON-last arm (PREC_KEYWORD).
                let body_prec = if self.arm_body_is_bare_pipe_infix(body) {
                    PREC_PIPE_PAREN
                } else if !last && self.arm_body_tail_is_open_arm_form(body, 0) {
                    PREC_KEYWORD
                } else {
                    0
                };
                self.print_arm_body(body, body_prec);
            }
            // Trailing comments after the body, same line (innermost closest to the body).
            for &text in trail_texts.iter().rev() {
                self.doc.word(format!(" //{}", self.doc_line_text(text)));
            }
        }
        if paren {
            self.doc.break_with(1, -INDENT);
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// Print a structural pattern. Shapes: a guarded pattern `(guard <pat> <expr>)` -> `pat if g`;
    /// a tuple pattern `(tuple p q…)` (2+ elements) -> `(p, q, …)`; a constructor application
    /// `(Ctor p…)` -> `Ctor(p, …)`; a dotted ctor `(. A B)` -> `A.B`; a bare name / literal prints
    /// as itself.
    ///
    /// DEPTH GUARD: `pattern` is a SECOND recursion hub (a tuple/list/ctor sub-pattern re-enters it),
    /// SEPARATE from `expr`'s — so `expr`'s guard never bounds it. A decoded-only deep pattern arena
    /// (`codec::decode` accepts arbitrary depth; the reader caps at `MAX_NESTING_DEPTH`, so this is
    /// unreachable from source but reachable from a crafted binary AST) overflowed the native stack
    /// (SIGABRT) walking it. Share `MAX_PRINT_DEPTH` + the `self.depth` budget with `expr`: past the
    /// ceiling elide (`…`) instead of recursing; else bump and delegate to `pattern_inner`, decrementing
    /// after — mirroring `expr`'s guard so the printer stays TOTAL on a pathological decode-only arena.
    fn pattern(&mut self, id: StructId) {
        if self.depth >= MAX_PRINT_DEPTH {
            self.doc.word("…");
            return;
        }
        self.depth += 1;
        self.pattern_inner(id);
        self.depth -= 1;
    }

    fn pattern_inner(&mut self, id: StructId) {
        if let Some(tail) = self.a.as_form(id, "guard")
            && tail.len() == 2
        {
            let (pat, guard) = (tail[0], tail[1]);
            self.pattern(pat);
            self.doc.word(" if ");
            self.expr(guard, 0);
            return;
        }
        match self.a.get(id) {
            Struct::List(items) if !items.is_empty() => {
                let items = items.clone();
                // The head spelling for the compound-pattern dispatch, recognized in EITHER kind: the
                // native ctor-LEAF head (M2, what a canonical native compound pattern carries) via
                // `head_ctor`, OR the shadowable NAME/`.`/`=` head via `head_name`. The VALUE path already
                // resugars native ctor heads (through `literal_ctor`/`head_ctor`); the pattern path
                // dispatched on bare `head_name`, so a native `Leaf::Ctor(Tuple/List/Map/Record)` PATTERN
                // head returned `None` here, missed every sugar arm, and fell to the generic `Ctor(p, …)`
                // arm — printing the classic name-head call `tuple(…)`/`list(…)`/… that does NOT re-read to
                // the native compound pattern (the ML compound-PATTERN round-trip gap; mirrors how
                // `is_binder_pattern` already combines both recognizers to ROUTE such a head into `pattern`).
                let pat_head = self
                    .head_ctor(items[0])
                    .or_else(|| self.head_name(items[0]));
                // tuple pattern `(tuple p …)` -> `(p, …)`, matching the value tuple. A 1-element
                // `(tuple p)` prints `(p,)` (trailing comma) so it re-reads as a 1-tuple, not `(p)`
                // grouping.
                if pat_head.as_deref() == Some("tuple") && items.len() >= 2 {
                    let subs = &items[1..];
                    self.doc.word("(");
                    for (i, &sub) in subs.iter().enumerate() {
                        if i > 0 {
                            self.doc.word(", ");
                        }
                        self.pattern(sub);
                    }
                    if subs.len() == 1 {
                        self.doc.word(","); // 1-tuple: trailing comma
                    }
                    self.doc.word(")");
                    return;
                }
                // list pattern `(list p… .. rest)` -> `[p, …, .. rest]`, the value list literal's twin
                // in pattern position (unconditional like the tuple pattern — a pattern head `list` is
                // the list constructor by grammar). A `..` marker glues to its rest binder.
                if pat_head.as_deref() == Some("list") {
                    self.print_pattern_seq("[", "]", &items[1..], |p, e| p.pattern(e));
                    return;
                }
                // map pattern `(map (k p) … .. rest)` -> `#{ k = p, …, .. rest }`, the key-directed
                // twin of the map literal. Each entry is a `(key sub-pattern)` pair; the key is a value
                // expression, the value slot a sub-pattern.
                if pat_head.as_deref() == Some("map") && self.is_map_pattern(&items[1..]) {
                    self.print_pattern_seq("#{ ", " }", &items[1..], |p, entry| {
                        if let Struct::List(pair) = p.a.get(entry) {
                            // A map-pattern entry is EITHER the native FieldPair `(= key sub-pattern)` (M2,
                            // what a canonical native map pattern carries — head is `Leaf::FieldPair`,
                            // `head_name` reports `=`) OR a legacy 2-element `(key sub-pattern)` pair (what
                            // the reader still emits). Both spell `key = sub`; without the FieldPair arm a
                            // native map pattern printed `= = <key>` (the `=` head misread as the key) and
                            // failed to re-read (the ML map-PATTERN round-trip gap, FACE 2).
                            let (key, sub) = if pair.len() == 3
                                && p.head_name(pair[0]).as_deref() == Some("=")
                            {
                                (pair[1], pair[2])
                            } else if pair.len() == 2 {
                                (pair[0], pair[1])
                            } else {
                                return;
                            };
                            p.expr(key, 0);
                            p.doc.word(" = ");
                            p.pattern(sub);
                        }
                    });
                    return;
                }
                // record pattern `(record (= field p) …)` -> `{ field = p, … }`, the field-directed twin
                // of the record literal (the operator-ruled bare-brace pattern surface). Each entry is the
                // canonical `(= field sub-pattern)` triple — the SAME form as a value-record field (path
                // B, full symmetry); the field is a plain name, the value slot a sub-pattern. Always
                // renders `field = p` (a punned `(= x x)` prints `{ x = x }`, re-reading to the same
                // `(record (= x x))`). Guarded on the record-pattern shape (all entries `(= name p)`).
                if pat_head.as_deref() == Some("record") && self.is_record_pattern(&items[1..]) {
                    // Empty record pattern `(record)` -> `{}` (no inner padding), matching the param
                    // path's empty render; `print_pattern_seq` would otherwise emit `{  }` (double space).
                    if items.len() == 1 {
                        self.doc.word("{}");
                        return;
                    }
                    self.print_pattern_seq("{ ", " }", &items[1..], |p, entry| {
                        // `(= field sub-pattern)` — field = child 1, sub-pattern = child 2 (drop `=`).
                        // Tolerate a legacy `(field sub-pattern)` pair so a stray un-migrated node prints.
                        if let Some((field, sub)) = p.record_pattern_field(entry) {
                            p.expr(field, 0);
                            p.doc.word(" = ");
                            p.pattern(sub);
                        }
                    });
                    return;
                }
                // binary pattern `(bin <segment> …)` -> `b[<segment>, …]`, the pattern-position twin of
                // the construction literal (unconditional — `bin` is a reserved grammar form, not a
                // shadowable ctor). Each segment is a sub-pattern (`u16(n)` binds `n`); `(bin)` -> `b[]`.
                if pat_head.as_deref() == Some("bin") {
                    self.print_pattern_seq("b[", "]", &items[1..], |p, s| p.pattern(s));
                    return;
                }
                // dotted constructor `(. A B)` prints as A.B
                if pat_head.as_deref() == Some(".")
                    && items.len() == 3
                    && let Some(key) = self.plain_key(items[2])
                {
                    self.pattern(items[1]);
                    self.doc.word(".");
                    self.doc.word(emit_name(&key));
                    return;
                }
                // constructor applied to sub-patterns: Ctor(p, …)
                self.pattern(items[0]);
                self.doc.word("(");
                for (i, &sub) in items[1..].iter().enumerate() {
                    if i > 0 {
                        self.doc.word(", ");
                    }
                    self.pattern(sub);
                }
                self.doc.word(")");
            }
            // An EMPTY list in pattern position (`Struct::List([])`) — e.g. the inner `()` of a quote
            // PATTERN `(quote ())`. The reader never produces it directly, but a quasiquote/quote over an
            // empty compound does. Render the raw-list escape `#[]`, mirroring `list()`'s expr-position
            // guard (the pattern parser accepts `#[…]` as its twin) — WITHOUT this arm it fell to the
            // `_` catch-all below, which assumed a `Struct::Atom` and hit `unreachable!()` (a never-panic
            // break: CDZ pattern-printer panicked on `(match (quote ()) ((quote ()) 1) (_ 0))`).
            Struct::List(_) => self.doc.word("#[]"),
            Struct::Atom(l) => {
                let leaf = self.a.leaf(*l).clone();
                self.leaf(&leaf);
            }
        }
    }

    /// Render a list/map PATTERN's items as `open p, …, .. rest close`, inline (patterns are small; no
    /// break box). A `Leaf::Name("..")` marker glues to the following item as `.. rest`, the inverse of
    /// the reader's flat rest shape; every other item renders via `emit`. Twin of `bracketed_rest` for
    /// the always-inline pattern surface.
    fn print_pattern_seq(
        &mut self,
        open: &str,
        close: &str,
        items: &[StructId],
        mut emit: impl FnMut(&mut Self, StructId),
    ) {
        self.doc.word(open.to_string());
        let mut i = 0;
        let mut first = true;
        while i < items.len() {
            if !first {
                self.doc.word(", ");
            }
            first = false;
            if let Some(args) = self.a.as_form(items[i], "..") {
                // WRAPPED `(.. operand)` — operand is a child, spans one slot.
                self.doc.word(".. ");
                self.pattern(args.first().copied().unwrap_or(items[i]));
                i += 1;
            } else if self.a.as_name(items[i]) == Some("..") && i + 1 < items.len() {
                // Legacy FLAT `..` marker — operand is the next sibling, spans two slots.
                self.doc.word(".. ");
                self.pattern(items[i + 1]);
                i += 2;
            } else {
                emit(self, items[i]);
                i += 1;
            }
        }
        self.doc.word(close.to_string());
    }

    /// A map PATTERN the `#{ k = p, … }` surface handles: each entry is a `(key sub-pattern)` pair,
    /// with an optional trailing `.. rest` marker + one binder (as the reader writes it). A shape that
    /// is not well-formed falls back to the generic `map(...)` call form so it still round-trips.
    fn is_map_pattern(&self, items: &[StructId]) -> bool {
        match items
            .iter()
            .position(|&a| self.head_name(a).as_deref() == Some(".."))
        {
            // `..` at index i binds the rest at i+1; ALL other entries (before AND after the rest) are
            // `(key sub)` / `(= key sub)` pairs. A well-formed map rest is LAST (`i + 2 == len`), but a
            // MALFORMED post-rest entry (`#{ 1 = v, .. rest, 2 = w }`, a CDZ0201 rest-shape error case) is
            // STILL a map PATTERN for printing — render it via `#{ … }` so it re-parses (parse-ok) and the
            // CDZ0201 fires at RESOLVE as before, rather than falling to the generic `map(…)` arm, which
            // prints a `FieldPair` entry as the un-reparseable `=(k, v)` application. `print_pattern_seq`
            // emits post-rest entries in order and read_ml parses them back to the same shape.
            Some(i) => {
                i + 1 < items.len() && self.is_pairs(&items[..i]) && self.is_pairs(&items[i + 2..])
            }
            None => self.is_pairs(items),
        }
    }

    /// A record PATTERN body `(field sub-pattern)…` — every entry is a 2-element pair whose FIELD (first
    /// element) is a plain name (`head_name`), so `{ field = p, … }` re-reads to the same `(record …)`.
    /// (No `..` rest — a record destructure names fields by exact label; a PARTIAL pattern simply lists
    /// fewer fields, still all `(name p)` pairs.) Distinguishes a genuine record pattern from a
    /// name-headed constructor application (`(Record a b)`, positional) that must NOT print as braces.
    fn is_record_pattern(&self, items: &[StructId]) -> bool {
        // Every entry is the canonical `(= field sub-pattern)` triple with a plain-name field (path B,
        // same form as a value-record field) — or a legacy `(field sub-pattern)` pair (tolerated). An
        // EMPTY record pattern `(record)` (`{}`, binds nothing) is ALSO a valid record pattern — the
        // parser produces it uniformly in param/let/match (`def f({}) = …` -> `(record)`), so accepting
        // it here lets the let-binder render `{}` consistently with the param, not the backtick-`let`
        // fallback. (`items.is_empty()` short-circuits the `all` to true, so no field check runs.)
        items
            .iter()
            .all(|&e| self.record_pattern_field(e).is_some())
    }

    /// The (field, sub-pattern) of a record-pattern entry, handling both the canonical
    /// `(= field sub-pattern)` triple (field = child 1) and a legacy `(field sub-pattern)` pair
    /// (field = child 0). The field must be a plain name. `None` if neither shape.
    fn record_pattern_field(&self, entry: StructId) -> Option<(StructId, StructId)> {
        match self.a.get(entry) {
            Struct::List(p) if p.len() == 3 && self.head_name(p[0]).as_deref() == Some("=") => {
                self.head_name(p[1]).is_some().then_some((p[1], p[2]))
            }
            Struct::List(p) if p.len() == 2 && self.head_name(p[0]).is_some() => Some((p[0], p[1])),
            _ => None,
        }
    }

    /// `,x` when the interior is atomic (a name / literal / member chain), else `,{ expr }`.
    fn unquote(&mut self, sigil: &str, inner: StructId, parent_prec: u8) {
        // In head/application position the bare sigil would swallow following args, so brace it.
        let brace = !self.unquote_atomic(inner) || parent_prec >= PREC_MEMBER;
        self.doc.word(sigil);
        if brace {
            self.doc.word("{ ");
            self.expr(inner, 0);
            self.doc.word(" }");
        } else {
            self.expr(inner, PREC_MEMBER);
        }
    }

    /// Print the FORM an annotation wraps (`args[1]` of `(@ name form)`). The parser re-reads the
    /// annotated form via `prefix()` (PREFIX position — no infix/postfix loop), so a form that `prefix()`
    /// would NOT recapture whole must be PARENTHESIZED here or the round-trip breaks: `@inline a + 1` (no
    /// parens) re-reads as `(+ (@ inline a) 1)` — the `@` binding only the leading atom `a` — instead of
    /// the intended `(@ inline (+ a 1))`. The forms `prefix()` does NOT recapture are exactly the ones the
    /// Pratt/`postfix` loops build AFTER the prefix atom: an INFIX operator application, a computed/name
    /// APPLICATION (a call), and a `.member` access. Every other form round-trips bare — an atom, a keyword
    /// form (`if`/`let`/`match`/`fn`/`do`/`type`/…, each self-delimiting in prefix position), a
    /// bracket-delimited literal (`[…]`/`(…,)`/`{…}`/`#{…}`), or a nested `@`. Wrapping only the three
    /// post-prefix shapes keeps the multi-line keyword forms unparenthesized (an `@inline (if …)` would be
    /// ugly and needless). The wrapped form prints at prec 0 inside the parens (a fresh sub-expression).
    /// If `form` is a documented `def`/value-def (a `def` list with leading `(doc …)` forms — the shape
    /// the reader's `carry_docs` produces from a `/// header` written ABOVE an `@`-annotation), print
    /// those docs NOW (above the `@name` about to be emitted) and set `suppress_leading_docs` so the def
    /// does not re-print them below the annotation. Restores the user's `/// header` \n `@test` \n `def`
    /// order instead of the reordered `@test` \n `/// header` \n `def`. Only handles the immediate-def
    /// case AND a stacked `@a @b def` (descends through nested `@` wrappers to the def, so the doc
    /// prints above ALL annotations). No-op for a non-def / doc-less form. The arena is unchanged
    /// (round-trip-safe — a print-position fix only). Only the OUTERMOST `@` arm's call actually prints
    /// (it sets the suppress flag; inner `@` arms then find the flag already pending — but since the flag
    /// is consumed only at the def, an inner call would re-hoist; guard on the flag to hoist ONCE).
    fn hoist_annotated_docs(&mut self, form: StructId) {
        // Already hoisted by an outer `@` this chain — don't print twice.
        if self.suppress_leading_docs {
            return;
        }
        // Descend through nested `@name` wrappers (a stacked `@a @b def`) to the innermost annotated form.
        let mut inner = form;
        while let Some(ann) = self.a.as_form(inner, "@") {
            if ann.len() != 2 {
                break;
            }
            inner = ann[1];
        }
        // A def is `(def SIG doc… body)` or value-def `(def NAME doc… value)`; either way the leading
        // `(doc …)` run sits at args[1..]. Only hoist when the head is `def` and it has ≥1 leading doc.
        let Some(args) = self.a.as_form(inner, "def") else {
            return;
        };
        let docs: Vec<StructId> = args
            .iter()
            .skip(1)
            .take_while(|&&a| self.is_doc(a))
            .copied()
            .collect();
        if docs.is_empty() {
            return;
        }
        for d in docs {
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
        self.suppress_leading_docs = true;
    }

    fn annotated_form(&mut self, form: StructId) {
        if self.form_needs_prefix_parens(form) {
            self.doc.word("(");
            self.expr(form, 0);
            self.doc.word(")");
        } else {
            self.expr(form, 0);
        }
    }

    /// Whether `form` is one of the three shapes the parser's `prefix()` would NOT recapture whole (so it
    /// needs parens when it appears in a prefix-only position — the annotated form of an `@`): an INFIX
    /// application (head in `infix_prec`), a `.member` access, or a bare APPLICATION/call (a list whose
    /// head is not a recognized special form and whose shape the generic call path renders as `head(args)`
    /// / `expr(args)`). A special-form/keyword/ctor list (`if`/`let`/`match`/`list`/`tuple`/`record`/`map`/
    /// `@`/…) is self-delimiting and does NOT need parens. Mirrors the parse asymmetry, not a full re-derivation
    /// of dispatch — conservative in the safe direction: an unrecognized name-headed list is treated as a
    /// call (needs parens), which is exactly how `prefix()`+`postfix` would (fail to) round-trip it.
    fn form_needs_prefix_parens(&self, form: StructId) -> bool {
        let items = match self.a.get(form) {
            Struct::List(items) if !items.is_empty() => items,
            _ => return false, // an atom (or empty list) is a prefix atom — no parens
        };
        // A member access `(. obj key)` — postfix, not recaptured by prefix alone.
        if self.head_name(items[0]).as_deref() == Some(".") {
            return true;
        }
        // An INFIX application `(op a b)` — a binary operator head with two operands.
        if items.len() == 3
            && let Some(h) = self.head_name(items[0])
            && infix_prec(&h).is_some()
        {
            return true;
        }
        // A special form / keyword / ctor list is self-delimiting (prints as a keyword form or a
        // bracketed literal, each recaptured whole by `prefix()`); anything else name-headed is a
        // generic call `head(args)`, and a non-name head is a computed application `expr(args)` — both
        // postfix-built, so they need parens.
        !self.is_self_delimiting_form(form)
    }

    /// Whether `form` prints as a SELF-DELIMITING surface form — a keyword form (`if`/`let`/`match`/`fn`/
    /// `do`/`type`/`effect`/`handle`/`host`/`module`/`export`/`import`/`comment`/`bin`/`forall`/
    /// `tagged-template`), a bracket-delimited compound literal (`list`/`tuple`/`record`/`map`), a nested
    /// annotation/pragma (`@`/`pragma`), or a quote/unquote sigil form — i.e. a form the parser's
    /// `prefix()` recaptures whole (it begins with a keyword, sigil, or opening bracket, never fusing with
    /// a following infix/postfix). Used by [`Self::form_needs_prefix_parens`] to leave these unparenthesized
    /// as an annotated form. A NAME head that merely SHADOWS one of these ctors (`list`/`tuple`/… as a user
    /// value) is a call, not the literal — but those print via the string-headed ctor path, so a NAME-headed
    /// `list` is correctly NOT matched here (it falls through to the call case → parens).
    fn is_self_delimiting_form(&self, form: StructId) -> bool {
        let items = match self.a.get(form) {
            Struct::List(items) if !items.is_empty() => items,
            _ => return false,
        };
        // A compound literal `("list" …)`/`("tuple" …)`/`("record" …)`/`("map" …)` — string-headed, or a
        // non-shadowed NAME alias — prints bracket-delimited (`[…]`/`(…,)`/`{…}`/`#{…}`), self-delimiting.
        // `literal_ctor` is the same gate the ctor sugar uses, so a SHADOWED `list`/… (an ordinary value)
        // correctly returns `None` here and falls through to the call case (→ parens).
        if self.literal_ctor(items[0]).is_some() {
            return true;
        }
        let Some(head) = self.head_name(items[0]) else {
            return false; // computed-callee application — a call, needs parens
        };
        let args = &items[1..];
        match head.as_str() {
            "let" => self.is_let_shape(args),
            "if" => args.len() == 3,
            "fn" => args.len() == 2,
            "match" => self.is_match_shape(args),
            // A `def` (function or value) is keyword-led and self-delimiting — the common annotated form
            // (`@inline def …`, `@test def …`). Both def shapes the print dispatch recognizes.
            "def" => self.is_def_shape(args) || self.is_value_def_shape(args),
            "do" => !args.is_empty(),
            "type" => self.is_type_shape(args),
            "effect" => self.is_effect_shape(args),
            "handle" => self.is_handle_shape(args),
            "host" => self.is_host_shape(args),
            "module" => self.is_module_shape(args),
            "export" => self.is_export_shape(args),
            "import" => self.is_import_shape(args),
            "comment" => args.len() == 2 && self.is_string(args[0]),
            "bin" => true,
            "tagged-template" => self.is_tagged_template_shape(args),
            "forall" => self.is_forall_shape(args),
            // A nested annotation/pragma is itself self-delimiting (leads with `@`/`@!`).
            "@" => args.len() == 2,
            "pragma" => args.len() == 2 && self.a.as_name(args[0]).is_some(),
            _ => false, // a generic name-headed call — needs parens
        }
    }

    fn unquote_atomic(&self, id: StructId) -> bool {
        match self.a.get(id) {
            Struct::Atom(_) => true,
            // a pure member-access chain `(. a b)` with a plain-ident key. An EMPTY list (`items` is
            // empty) has no head to inspect — it is not a member chain, so it is not atomic. Guard the
            // `items[0]`/`items[2]` indexing (an empty list is a valid arena node, e.g. `(unquote ())`,
            // and must not panic the printer — the reader-never-panics contract extends to the printer).
            Struct::List(items) => {
                items.len() == 3
                    && self.head_name(items[0]).as_deref() == Some(".")
                    && self.plain_key(items[2]).is_some()
                    && self.unquote_atomic(items[1])
            }
        }
    }

    // ---- shape helpers ----

    fn head_name(&self, id: StructId) -> Option<String> {
        match self.a.get(id) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Name(n) => Some(n.to_string()),
                // Native compound heads (M2) render through the SAME head-keyed recognizers as their
                // legacy spellings: a `Member` head IS the `.` of a `(. obj key)` projection, a `FieldPair`
                // head IS the `=` of a record/map entry `(= k v)`. Reporting their surface spelling here
                // lets `member_key`/`is_member_call`/`is_record_shape`/`print_record` etc. recognize a
                // native-headed node with no per-site change (a `(FieldPair k v)`/`(Member o k)` has the
                // same child positions as `(= k v)`/`(. o k)`). Head-IDENTITY comparison (`node_eq`) is a
                // separate cadenza-ast concern and does NOT collapse these.
                Leaf::Member => Some(".".to_string()),
                Leaf::FieldPair => Some("=".to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    /// The head STRING-LITERAL of a `List` occurrence, if its first child is an `Atom(Str)` — the
    /// compound-value CONSTRUCTOR primitive (`("list" …)`/`("tuple" …)`/`("record" …)`/`("map" …)`).
    /// The reader desugars a `[…]`/`(a,b)`/`{…}`/`#{…}` literal to a string-headed form (unshadowable);
    /// the printer round-trips it BACK to the literal by recognizing this string head.
    fn head_ctor(&self, id: StructId) -> Option<String> {
        match self.a.get(id) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Str(s) => Some(s.to_string()),
                // A native ctor-leaf head (M2) is the unshadowable compound primitive, exactly like the
                // string head — report its surface word so `literal_ctor` sugars `(<ctor> …)` to `[…]` /
                // `(a, b)` / `{…}` / `#{…}` / a set literal.
                Leaf::Ctor(c) => Some(crate::sexpr::compound_ctor_word(*c).to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    /// The compound-ctor spelling this head denotes IF it should render as a surface literal: the
    /// STRING primitive (`("record" …)` — always) or the NAME alias (`(record …)`) when that name is
    /// NOT shadowed by a binder in this tree. A shadowed name (or any non-ctor head) returns `None`,
    /// so it falls through to the ordinary call/keyword dispatch. This is the single gate through which
    /// both the reader's string-headed literals and hand-authored name-headed forms reach the sugar.
    fn literal_ctor(&self, id: StructId) -> Option<String> {
        if let Some(s) = self.head_ctor(id) {
            return Some(s); // string primitive — unshadowable, always a literal
        }
        let name = self.head_name(id)?;
        match ctor_index(&name) {
            Some(i) if !self.shadowed_ctors[i] => Some(name),
            _ => None,
        }
    }

    /// If `items` is a quantity `(Qty.of <operand> (Unit.of #"name"))`, return its operand and the unit
    /// name — so it prints as the concise `<operand> name` surface (the inverse of the parser's general
    /// `maybe_unit_suffix` postfix). All of these must hold, else the general call form renders it (a
    /// faithful round-trip either way):
    ///   * head is the member access `(. Qty of)` and there are exactly two arguments;
    ///   * arg 0 is a TIGHT unit operand (see [`Self::is_tight_unit_operand`]) — a non-negative numeric
    ///     literal OR a name / call / member-chain — i.e. an expression that binds at least as tight as
    ///     the unit-suffix postfix, so `<operand> name` re-lexes back to the SAME `(Qty.of operand unit)`.
    ///     An infix / keyword-form / already-quantity operand is NOT tight (`a + b meter` binds the unit
    ///     to `b`; `if … meter` to the else-branch), so it falls back to the explicit `Qty.of(…)` call;
    ///   * arg 1 is `(Unit.of #"name")` where `name` is a bare-safe identifier (re-lexes to one `Ident`).
    fn quantity_literal(&self, items: &[StructId]) -> Option<(StructId, String)> {
        if items.len() != 3 || !self.is_member_call(items[0], "Qty", "of") {
            return None;
        }
        if !self.is_tight_unit_operand(items[1]) {
            return None;
        }
        let Struct::List(unit) = self.a.get(items[2]) else {
            return None;
        };
        if unit.len() != 2 || !self.is_member_call(unit[0], "Unit", "of") {
            return None;
        }
        let name = match self.a.get(unit[1]) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Sym(s) if name_is_bare_safe(s) => s.to_string(),
                _ => return None,
            },
            _ => return None,
        };
        Some((items[1], name))
    }

    /// Whether `id` is a TIGHT operand for the concise `<operand> unit` quantity surface — one that binds
    /// at least as tight as the unit-suffix postfix, so printing `<operand> name` re-parses to the SAME
    /// `(Qty.of operand (Unit.of #name))` node. Accepts: a non-negative numeric literal (`5 meter`), a
    /// bare name (`x meter`), a member-access chain (`x.y meter`, via `unquote_atomic`'s plain-key walk),
    /// or a NAME-headed application / call (`f(x) meter`, `f(g(x)) meter`). REJECTS an infix application
    /// (`a + b meter` would bind the unit to `b`), a keyword form (`if`/`let`/`match`/`fn` — the unit
    /// would bind to the tail), a computed-callee application, and an already-`Qty.of` operand (which
    /// would double-suffix). Those fall back to the explicit `Qty.of(…)` render (a faithful round-trip).
    fn is_tight_unit_operand(&self, id: StructId) -> bool {
        if self.is_nonneg_number(id) {
            return true;
        }
        match self.a.get(id) {
            // a bare name atom (`x`)
            Struct::Atom(_) => self.head_name(id).is_some(),
            Struct::List(items) if !items.is_empty() => {
                // a member-access chain `(. a b …)` with plain keys (prints `a.b`, re-lexes tight)
                if self.head_name(items[0]).as_deref() == Some(".") {
                    return self.unquote_atomic(id);
                }
                // a NAME-headed application `(f arg…)` -> `f(arg…)` — tight (a call binds at postfix). A
                // head that is a special form / infix operator / string-ctor is NOT a plain call, and a
                // computed callee (`(expr) arg`) is excluded (its callee could itself be non-tight).
                match self.head_name(items[0]) {
                    Some(h) => {
                        infix_prec(&h).is_none() && !self.is_self_delimiting_form(id) && h != "."
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// DISPLAY-only. If `items` is a quantity VALUE `(Qty.of <value> <unit>)`, return its magnitude and
    /// its unit — where the unit is `None` for the DIMENSIONLESS `Unit.one` (rendered as just the
    /// value). Unlike `quantity_literal` this is for the VALUE form a result carries, so it accepts any
    /// magnitude (a `Rational`/`Float`/`Int` leaf — the display leaf renderer handles each) and any unit
    /// EXPRESSION (`Unit.base`, `Unit.one`, or a composite `*`/`/`/`^` of them — `display_unit` renders
    /// it), with no round-trip guards. Returns `None` for a non-quantity, so the general form renders it.
    fn display_quantity(&self, items: &[StructId]) -> Option<(StructId, Option<StructId>)> {
        if items.len() != 3 || !self.is_member_call(items[0], "Qty", "of") {
            return None;
        }
        let unit = if self.is_member_call(items[2], "Unit", "one") {
            None // dimensionless — show just the value
        } else {
            Some(items[2])
        };
        Some((items[1], unit))
    }

    /// DISPLAY-only. Render a unit EXPRESSION as compact math: `(Unit.base #"meter")` → `meter`,
    /// `Unit.one` → `1`, and a composite (heads `Unit.*`/`Unit./`/`Unit.^`, or the bare glyphs the value
    /// form may carry) as its infix form with NO surrounding spaces — `meter/second`, `meter^2`,
    /// `meter/second^2` — the mathematical convention for a unit, and distinct from the spaced arithmetic
    /// a magnitude uses. `parent_prec` drives minimal parens via the shared `infix_prec` table (a
    /// left-associative operator prints its right child one tier tighter, so `meter/(second*second)`
    /// keeps its parens). Any shape this does not recognize falls back to the ordinary expression form.
    fn display_unit(&mut self, id: StructId, parent_prec: u8) {
        if let Some(name) = self.unit_base_name(id) {
            self.doc.word(name);
            return;
        }
        if self.is_member_call(id, "Unit", "one") {
            self.doc.word("1");
            return;
        }
        if let Struct::List(m) = self.a.get(id)
            && m.len() == 3
            && let Some(head) = self.head_name(m[0]).map(|h| h.to_string())
            && let Some(prec) = infix_prec(&head)
            && matches!(infix_glyph(&head), "*" | "/" | "^")
        {
            let (glyph, l, r) = (infix_glyph(&head).to_string(), m[1], m[2]);
            let paren = prec < parent_prec;
            if paren {
                self.doc.word("(");
            }
            self.display_unit(l, prec);
            self.doc.word(glyph);
            // `^`'s right operand is the integer exponent (a literal, not a unit); every other
            // composition's right operand is a unit, printed one tier tighter for left-associativity.
            if head_glyph_is_pow(&head) {
                self.expr(r, PREC_MEMBER);
            } else {
                self.display_unit(r, prec + 1);
            }
            if paren {
                self.doc.word(")");
            }
            return;
        }
        // Unrecognized unit shape — render it as an ordinary expression (still readable).
        self.expr(id, parent_prec);
    }

    /// If `id` is `(Unit.base #"name")`, the base-dimension NAME (the symbol's text). Used by the
    /// display surface to print a base unit as its bare name.
    fn unit_base_name(&self, id: StructId) -> Option<String> {
        let Struct::List(m) = self.a.get(id) else {
            return None;
        };
        if m.len() != 2 || !self.is_member_call(m[0], "Unit", "base") {
            return None;
        }
        match self.a.get(m[1]) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Sym(s) => Some(s.to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    /// If `items` is a unit conversion `(Unit.in (Unit.of #"name") value)` whose target is a BARE-NAME
    /// family unit, return the converted value and the target unit name — so it prints as the concise
    /// `value as name` surface (the inverse of the parser's `as_conversion`). All must hold, else the
    /// general call form renders it (a faithful round-trip either way):
    ///   * head is the member access `(. Unit in)` and there are exactly two arguments;
    ///   * arg 0 is `(Unit.of #"name")` where `name` is a bare-safe identifier (re-lexes to one `Ident`,
    ///     which the parser reads back as the same `(Unit.of #"name")` target — a COMPOUND/computed
    ///     target has no bare-name surface and falls back to the call form).
    fn unit_conversion(&self, items: &[StructId]) -> Option<(StructId, String)> {
        if items.len() != 3 || !self.is_member_call(items[0], "Unit", "in") {
            return None;
        }
        let Struct::List(unit) = self.a.get(items[1]) else {
            return None;
        };
        if unit.len() != 2 || !self.is_member_call(unit[0], "Unit", "of") {
            return None;
        }
        let name = match self.a.get(unit[1]) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Sym(s) if name_is_bare_safe(s) => s.to_string(),
                _ => return None,
            },
            _ => return None,
        };
        Some((items[2], name))
    }

    /// If `items` is a set literal `((. Set of) (list e …))`, return its element occurrences — so it
    /// prints as the concise `#(e, …)` surface (the inverse of the parser's `set_literal`). All of
    /// these must hold, else the general `Set.of(…)` call form renders it (a faithful round-trip
    /// either way):
    ///   * head is the member access `(. Set of)` and there is exactly one argument;
    ///   * that argument is a `list` LITERAL (a `("list" …)` primitive or an UNSHADOWED `(list …)`
    ///     alias, via `literal_ctor` — a shadowed `list` is a user application, kept as a call);
    ///   * the list carries no `.. rest` marker (the `#(…)` surface has no rest form, so a spread list
    ///     falls back to `Set.of([…, .. rest])`).
    fn set_literal(&self, items: &[StructId]) -> Option<Vec<StructId>> {
        if items.len() != 2 || !self.is_member_call(items[0], "Set", "of") {
            return None;
        }
        let Struct::List(list) = self.a.get(items[1]) else {
            return None;
        };
        let &head = list.first()?;
        if self.literal_ctor(head).as_deref() != Some("list") {
            return None;
        }
        let elems = &list[1..];
        if self.has_rest_marker(elems) {
            return None;
        }
        // A non-last `(comment-after …)` element has no faithful `#(…)` rendering (would swallow the
        // following `, …`) — decline so it falls to the generic `Set.of([…])` call form, which round-trips.
        if self.has_nonlast_comment_after(elems) {
            return None;
        }
        Some(elems.to_vec())
    }

    /// True iff `id` is the member-access head `(. obj key)` with the given object and key names.
    fn is_member_call(&self, id: StructId, obj: &str, key: &str) -> bool {
        matches!(self.a.get(id), Struct::List(m)
            if m.len() == 3
                && self.head_name(m[0]).as_deref() == Some(".")
                && self.head_name(m[1]).as_deref() == Some(obj)
                && self.head_name(m[2]).as_deref() == Some(key))
    }

    /// True iff `id` is a non-negative `Int` or `Float` literal — the numeric operand a quantity literal
    /// can render bare before a unit name.
    fn is_nonneg_number(&self, id: StructId) -> bool {
        match self.a.get(id) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Int { value, .. } => !value.negative,
                Leaf::Float(d) => !d.negative,
                _ => false,
            },
            _ => false,
        }
    }

    /// If `id` is an `Atom(Name)` that is a plain member key (alpha/underscore start, no dots), that
    /// name — so `(. a b)` prints as `a.b` but a dotted/odd key falls back to the call form.
    fn plain_key(&self, id: StructId) -> Option<String> {
        let n = self.head_name(id)?;
        // The wildcard member `*` (`obj.*` — the whole-constructor-set key the export surface uses).
        // A reserved final member segment: the parser reads `.` + `*` as this key (`dot_is_member`), so
        // rendering it bare round-trips. `*` alone is never a plain field name outside member position.
        if n == "*" {
            return Some(n);
        }
        let mut chars = n.chars();
        match chars.next() {
            Some(c) if c.is_alphabetic() || c == '_' => {}
            _ => return None,
        }
        if n.contains('.') { None } else { Some(n) }
    }

    /// The rendered text of a member-access key `(. obj KEY)`, if it re-reads as a `.KEY` postfix: a
    /// plain field NAME (`obj.field`) or a non-negative decimal INTEGER index (`obj.0` — positional
    /// tuple access). A negative/non-decimal integer, a float, or any other atom returns `None`, so
    /// the access falls back to the round-tripping head-call form `` `.`(obj, key) ``.
    fn member_key(&self, id: StructId) -> Option<String> {
        if let Some(name) = self.plain_key(id) {
            return Some(name);
        }
        // A non-negative decimal integer index: rendered as its bare digits (the parser reads a `.N`
        // postfix as this same `Int` key). Hex/binary or negative indices aren't valid `.N` syntax.
        match self.a.get(id) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Int {
                    value,
                    radix: crate::ast::Radix::Dec,
                } if !value.negative => Some(literal::render_int(value, crate::ast::Radix::Dec)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether the ML rendering of `id` ends in a decimal digit at its top level — the operands a
    /// following numeric `.N` would glue into a float token. That is a bare non-negative decimal
    /// integer (`5` → `5.0`), or a member access whose own key is a numeric index (`x.0` → `x.0.1`).
    /// A parenthesized/name/string operand does not, so it needs no extra parens.
    fn ends_in_decimal_digit(&self, id: StructId) -> bool {
        match self.a.get(id) {
            Struct::Atom(l) => matches!(
                self.a.leaf(*l),
                Leaf::Int { radix: crate::ast::Radix::Dec, value } if !value.negative
            ),
            Struct::List(items) => {
                // a `(. inner NUMERIC)` renders `inner.N`, ending in the numeric key's digit.
                items.len() == 3
                    && self.head_name(items[0]).as_deref() == Some(".")
                    && self
                        .member_key(items[2])
                        .is_some_and(|k| k.bytes().next().is_some_and(|b| b.is_ascii_digit()))
            }
        }
    }

    fn is_let_shape(&self, args: &[StructId]) -> bool {
        if args.len() != 2 {
            return false;
        }
        // The binds arg may carry a TRAILING `(comment-after … binds)` (a `//` after `in`) — peel it to
        // the real bindings list before shape-checking, else the wrapped `let` falls to the backtick
        // call form (and `print_let` peels the same wrapper to re-emit the comment after `in`).
        let binds_arg = self.strip_comment_after(args[0]);
        match self.a.get(binds_arg) {
            Struct::List(binds) => binds.iter().all(|&raw| {
                // A binding may be wrapped in a LEADING `(comment …)` (own-line `//` above it) and/or a
                // TRAILING `(comment-after …)` — peel both to the real `(binder value)` pair before
                // checking its shape (else a commented binding fails and the whole `let` falls to the
                // backtick call form). Own-line comments are printed above the binding by `print_let`.
                let b = self.strip_field_comments(raw);
                match self.a.get(b) {
                    // Each binding is a `(binder value)` pair. The binder is a plain NAME (`head_name`) or
                    // a destructuring PATTERN the surface can round-trip (`is_binder_pattern`). Any OTHER
                    // list binder — e.g. a constructor-application pattern `Mk(n)` (`((. Id Mk) n)`), which
                    // the reader has no let-binder surface for — falls back to the generic call form,
                    // which round-trips via idempotence.
                    Struct::List(p) => {
                        p.len() == 2
                            && (self.head_name(p[0]).is_some() || self.is_binder_pattern(p[0]))
                    }
                    _ => false,
                }
            }),
            _ => false,
        }
    }

    /// Whether `id` is a destructuring pattern the ML surface can render AND read back in a BINDER
    /// position (a `let` binder or a `def`/`fn` parameter): a tuple `(tuple …)`, a list `(list …)`, a
    /// map `(map …)`, or a binary `(bin …)`. These are exactly the compound patterns `param`/`let_expr`
    /// route to `pattern` (their surfaces `(a, b)`/`[x, .. rest]`/`#{ k = p }`/`b[u16(n)]` re-lex to a
    /// binder-position pattern). A constructor-application pattern like `Mk(n)` is deliberately EXCLUDED
    /// — the reader has no binder surface for it, so sugaring it would not round-trip; it stays the
    /// generic call form. STRING-headed ctor primitives (`"tuple"`/`"list"`/`"map"`) and their NAME
    /// aliases both qualify (a name alias is not shadowable in this structural position).
    fn is_binder_pattern(&self, id: StructId) -> bool {
        // Read the HEAD child of the pattern list (the printer's `head_ctor`/`head_name` inspect an
        // atom, so apply them to `items[0]`, not the list itself). The head is a STRING primitive
        // (`"tuple"`) or a NAME alias (`tuple`); both denote the same binder-position construct.
        let Struct::List(items) = self.a.get(id) else {
            return false;
        };
        let Some(&head) = items.first() else {
            return false;
        };
        let name = self.head_ctor(head).or_else(|| self.head_name(head));
        if matches!(name.as_deref(), Some("tuple" | "list" | "map" | "bin")) {
            return true;
        }
        // a RECORD pattern `(record (field p) …)` — the operator-ruled bare-brace binder surface
        // (`let { x = a } = r in …`, `def f({ x = a }) = …`). Renders `{ field = p, … }`, re-read by the
        // parser's `LBrace` pattern arm. Guarded on the record-pattern shape so a positional `(record …)`
        // (which is not a valid field-pattern body) still falls through rather than mis-sugaring.
        if name.as_deref() == Some("record") && self.is_record_pattern(&items[1..]) {
            return true;
        }
        // A CONSTRUCTOR pattern in binding position — `Ctor(p…)` (name-headed application, e.g. `(C c)` /
        // `(Some x)`) or a qualified `Mod.Ctor(p…)` whose head is the member-access list `(. Mod Ctor)`
        // (so the whole binder is `((. Id Mk) n)`). The parser now routes such a head to `pattern()` in a
        // `let`/param binder (a single-constructor destructure binds like a tuple — the corpus
        // `(let (((Id.Mk n) …)) …)`), and the printer's `pattern` already renders `Ctor(p…)` / `A.B(p…)`,
        // so the `let` prints its proper surface instead of the backtick-`let` fallback. A head that is a
        // special-form / infix operator is NOT a constructor (guarded so only a genuine ctor application
        // binder — a name head, or a member-access head — is recognized).
        if items.len() < 2 {
            return false; // a bare atom / 1-element list is not a ctor APPLICATION binder
        }
        match self.a.get(head) {
            // qualified constructor head: the member-access list `(. Mod Ctor)`
            Struct::List(_) => self.head_name(head).is_none() && self.is_member_access_chain(head),
            // a plain name-headed application `(Ctor p…)` — not an infix operator / special form.
            Struct::Atom(_) => {
                matches!(&name, Some(h) if infix_prec(h).is_none())
                    && !self.is_self_delimiting_form(id)
            }
        }
    }

    /// Whether `id` is a member-access chain `(. base key …)` (a qualified name like `Mod.Ctor`), so it
    /// heads a qualified constructor pattern. The chain's own head is the `.` name; a plain-key member
    /// walk (reusing `unquote_atomic`, which validates `(. a b)` chains with plain keys) confirms it.
    fn is_member_access_chain(&self, id: StructId) -> bool {
        matches!(self.a.get(id), Struct::List(items)
            if items.first().and_then(|&h| self.head_name(h)).as_deref() == Some("."))
            && self.unquote_atomic(id)
    }

    fn is_match_shape(&self, args: &[StructId]) -> bool {
        // A scrutinee plus at LEAST ONE arm. A zero-arm match (`(match x)` — vacuously exhaustive on
        // a Never-typed scrutinee) has no `| arm` to render and no closer after `with`, so it falls
        // through to the generic call form `` `match`(x) `` instead (which round-trips as a call).
        if args.len() < 2 {
            return false;
        }
        args[1..].iter().all(|&a| {
            // An arm may be wrapped in a LEADING `(comment …)` (an own-line `//` above the arm) and/or a
            // TRAILING `(comment-after …)` (a `//` on the arm's line) — peel both to the real arm before
            // checking it's a 2-element `(pat body)`.
            let arm = self.strip_field_comments(a);
            matches!(self.a.get(arm), Struct::List(p) if p.len() == 2)
        })
    }

    /// If `id` is a `(comment-after "text" inner)` wrapper, return `(Some(text_id), inner)`; else
    /// `(None, id)`. The dual of the leading `(comment "text" inner)` — a `//` that TRAILED `inner` on
    /// the same source line (`Ctor(T) // note`). The printer prints `inner` then ` // text` (same line).
    fn strip_comment_after(&self, id: StructId) -> StructId {
        match self.a.as_form(id, "comment-after") {
            Some(a) if a.len() == 2 && self.is_string(a[0]) => a[1],
            _ => id,
        }
    }

    /// Peel any comment wrappers around a record/map field/entry down to the inner `(name value)` pair:
    /// a LEADING `(comment "text" inner)` (an own-line `//` above the field) and/or a TRAILING
    /// `(comment-after "text" inner)` (a same-line `//` after the last field), in any nesting. Returns the
    /// innermost non-comment node. Used by `is_pairs` so a commented field still counts as a pair.
    fn strip_field_comments(&self, id: StructId) -> StructId {
        let mut cur = id;
        loop {
            let next = self.strip_comment_after(cur);
            let next = match self.a.as_form(next, "comment") {
                Some(a) if a.len() == 2 && self.is_string(a[0]) => a[1],
                _ => next,
            };
            if next == cur {
                return cur;
            }
            cur = next;
        }
    }

    /// Every arg is a 2-element `(key value)` pair — the shape the record/map surfaces render. A
    /// malformed record/map (an arg that isn't a pair) falls back to the generic call form so it
    /// still round-trips. A record additionally needs its field key to be a name; a map key is any
    /// expression.
    fn is_pairs(&self, args: &[StructId]) -> bool {
        // A same-line trailing-comment wrapper is only faithfully renderable on the LAST field/entry (the
        // printer emits ` // text` then forces the closing brace to its own line). A NON-last wrapped pair
        // — only from a decoded / metaprogramming-built AST, never the gated reader — has NO faithful
        // `{…}`/`#{…}` rendering (`k = v // text, …` swallows the `,`), so reject it here → the literal
        // falls back to the generic `"record"(…)`/`"map"(…)` call form, which round-trips (PR#763 /
        // Copilot: a printer shape-guard must be correct on ANY AST, not just the reader's output).
        if self.has_nonlast_comment_after(args) {
            return false;
        }
        args.iter().all(|&a| {
            // See through any comment wrappers (own-line LEADING `(comment …)` on any field + a same-line
            // TRAILING `(comment-after …)` on the last) so a commented field still counts as a pair.
            let inner = self.strip_field_comments(a);
            match self.a.get(inner) {
                // Native FieldPair entry `(= key value)` (M2) — head "=" (native-aware `head_name`).
                Struct::List(p) if p.len() == 3 && self.head_name(p[0]).as_deref() == Some("=") => {
                    true
                }
                // Legacy bare `(key value)` pair.
                Struct::List(p) if p.len() == 2 => true,
                _ => false,
            }
        })
    }

    /// A record the `{ name = e, … }` surface handles: every field is the canonical `(= name value)`
    /// ascription triple (RV2, Phase B) — or the legacy `(name value)` pair — whose KEY is a plain
    /// field name (so it re-reads as a `name = value` binding). NOTE this is record-SPECIFIC (not shared
    /// with `is_pairs`, which maps still use for their pair entries): only value-RECORD fields gained the
    /// `=` head; a map entry stays a bare `(key value)` pair.
    fn is_record_shape(&self, args: &[StructId]) -> bool {
        // Reject a non-last same-line comment wrapper (no faithful `{…}` rendering) — same as `is_pairs`.
        if self.has_nonlast_comment_after(args) {
            return false;
        }
        // Scan, skipping CONSTRUCTION spreads: a `..` marker (followed by one operand) may appear at ANY
        // position and more than once (`{ a = 1, ..b, c = 2, ..d }`); `bracketed_rest` renders them.
        // Every other item must be a well-formed field.
        let mut i = 0;
        while i < args.len() {
            // A construction spread: the wrapped `(.. operand)` node spans ONE slot; the legacy flat
            // `Name("..")`+operand marker spans TWO. Both may appear at any position, more than once.
            if self.a.as_form(args[i], "..").is_some() {
                i += 1;
                continue;
            }
            if self.a.as_name(args[i]) == Some("..") {
                if i + 1 >= args.len() {
                    return false; // a `..` with no operand is malformed → generic form
                }
                i += 2;
                continue;
            }
            let inner = self.strip_field_comments(args[i]);
            let ok = match self.a.get(inner) {
                // `(= name value)` — the canonical field; key is `p[1]`, must be a plain field name.
                Struct::List(p) if p.len() == 3 && self.head_name(p[0]).as_deref() == Some("=") => {
                    self.plain_key(p[1]).is_some()
                }
                // Legacy `(name value)` pair — key is `p[0]`.
                Struct::List(p) if p.len() == 2 => self.plain_key(p[0]).is_some(),
                _ => false,
            };
            if !ok {
                return false;
            }
            i += 1;
        }
        true
    }

    /// A map the `#{ key: v, … }` surface handles: every entry is a `(key value)` pair (any key),
    /// with an optional trailing `.. rest` spread (a `Leaf::Name("..")` marker followed by one binder,
    /// as the reader writes it). The pairs before the marker must be well-formed; the marker + its
    /// binder are rendered by `bracketed_rest`. A map whose `..` is not a well-formed trailing
    /// `.. rest` falls back to the generic call form so it still round-trips.
    fn is_map_shape(&self, args: &[StructId]) -> bool {
        // Reject a non-last same-line comment wrapper (no faithful `#{…}` rendering) — the whole-slice check
        // the old `is_pairs(args)` performed, kept explicit now that the scan below is per-item.
        if self.has_nonlast_comment_after(args) {
            return false;
        }
        // Scan, skipping spreads: a `..` marker (+ one operand) may appear at ANY position and more than
        // once — a PATTERN rest is trailing-only, but a CONSTRUCTION spread (`#{ ..a, k = v, ..b }`) is
        // uniform with list/record; `bracketed_rest` renders them. Every other item must be a pair.
        let mut i = 0;
        while i < args.len() {
            // A construction spread: the wrapped `(.. operand)` node spans ONE slot; the legacy flat
            // `Name("..")`+operand marker spans TWO. Both may appear at any position, more than once.
            if self.a.as_form(args[i], "..").is_some() {
                i += 1;
                continue;
            }
            if self.a.as_name(args[i]) == Some("..") {
                if i + 1 >= args.len() {
                    return false; // a `..` with no operand is malformed → generic form
                }
                i += 2;
                continue;
            }
            if !self.is_pairs(std::slice::from_ref(&args[i])) {
                return false;
            }
            i += 1;
        }
        true
    }

    /// A def the `def name(…) = body` (function) surface handles: a signature LIST `(name p…)` (head
    /// is a name, so params lower as binders), then zero or more `(doc "…")` forms, then a single
    /// body. Any OTHER interleaved body form (e.g. a `(: type)` annotation) falls back to the generic
    /// call form so it still round-trips.
    fn is_def_shape(&self, args: &[StructId]) -> bool {
        if args.len() < 2 {
            return false;
        }
        let sig_ok = matches!(self.a.get(args[0]), Struct::List(sig) if !sig.is_empty()
            && self.head_name(sig[0]).is_some());
        // args[1..last] must all be `(doc "…")`.
        let docs_ok = args[1..args.len() - 1].iter().all(|&a| self.is_doc(a));
        sig_ok && docs_ok
    }

    /// A def the `def name = value` (value) surface handles: `args[0]` is an atom NAME (not a
    /// signature list — that is what distinguishes it from the function form), then zero or more
    /// `(doc "…")` forms, then a single value expression.
    fn is_value_def_shape(&self, args: &[StructId]) -> bool {
        if args.len() < 2 {
            return false;
        }
        let name_ok = self.head_name(args[0]).is_some();
        let docs_ok = args[1..args.len() - 1].iter().all(|&a| self.is_doc(a));
        name_ok && docs_ok
    }

    /// A module the `module name { … }` surface handles: a name, then any members (docs included).
    fn is_module_shape(&self, args: &[StructId]) -> bool {
        !args.is_empty() && self.head_name(args[0]).is_some()
    }

    /// True if `id` is a well-formed `(doc "text")` node.
    fn is_doc(&self, id: StructId) -> bool {
        matches!(self.a.as_form(id, "doc"), Some(a) if a.len() == 1 && self.is_string(a[0]))
    }

    /// True if `id` is a string-literal atom.
    fn is_string(&self, id: StructId) -> bool {
        matches!(self.a.get(id), Struct::Atom(l) if matches!(self.a.leaf(*l), Leaf::Str(_)))
    }
}

/// True iff `head` is a unit-EXPONENTIATION head (arena `^` or the qualified `Unit.^`) — the one unit
/// composition whose right operand is an integer exponent, not a nested unit. The display unit renderer
/// prints that operand as a plain expression while recursing into the unit operands of `*`/`/`.
fn head_glyph_is_pow(head: &str) -> bool {
    infix_glyph(head) == "^"
}

/// Escape a tagged-template literal CHUNK for re-emission between the quotes of `tag"…"`: apply the
/// string escapes (`\n`/`\t`/`\r`/`\\`/`\"`) AND double literal braces (`{`→`{{`, `}`→`}}`) so a brace
/// in the chunk is NOT re-read as a hole delimiter. The inverse of the reader's chunk-unescape in
/// `literal::split_template_body`, so `tag"…"` round-trips.
fn escape_template_chunk(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            _ => out.push(c),
        }
    }
    out
}

pub fn emit_name(s: &str) -> String {
    if name_is_bare_safe(s) {
        s.to_string()
    } else {
        let mut out = String::from("`");
        for c in s.chars() {
            if c == '`' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('`');
        out
    }
}

/// True iff a symbol with content `s` may print as the UNQUOTED `#name` sugar rather than `#"…"`.
/// Operational (runs the real lexer over `#s`), so the sugar can never drift from what the lexer
/// accepts: it holds exactly when `#s` lexes to a single `SymLit` token spanning all of `#s` AND that
/// token re-decodes to `s` unchanged. The span check rejects any content the lexer would not glue into
/// one token (empty, a space, a leading digit, an operator glyph, `.`); the decode check rejects
/// content that is not already NFC-normalized, since the unquoted body is NFC-normalized on the way
/// back in (`unescape_sym_token`) and would otherwise round-trip to a DIFFERENT symbol. Everything else
/// keeps the explicit `#"…"` form.
fn sym_is_bare_safe(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let candidate = format!("#{s}");
    let mut toks = Lexer::new(&candidate).filter(|t| !t.kind.is_trivia());
    match (toks.next(), toks.next()) {
        (Some(t), None) => {
            t.kind == Kind::SymLit
                && t.span.start == 0
                && t.span.end == candidate.len()
                && literal::unescape_sym_token(&candidate) == Leaf::Sym(s.into())
        }
        _ => false,
    }
}

/// True iff `s` lexes to exactly one `Ident` token spanning all of `s`, and `s` is not a reserved
/// word. Operational (runs the real lexer), so the escape can never drift from what the lexer
/// accepts bare. An operator glyph (`+`, `->`) lexes as an operator token, NOT an `Ident`, so it is
/// never bare-safe — a bare `+` used as a name must be backtick-quoted to read back as a name.
fn name_is_bare_safe(s: &str) -> bool {
    if s.is_empty() || token::is_reserved(s) {
        return false;
    }
    let mut toks = Lexer::new(s).filter(|t| !t.kind.is_trivia());
    match (toks.next(), toks.next()) {
        (Some(t), None) => t.kind == Kind::Ident && t.span.start == 0 && t.span.end == s.len(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Builder;
    use crate::{parser, sexpr};

    // `inline_world_decl_round_trips_through_the_ml_surface` (full export+import Reducer world) and
    // `inline_world_nullary_member_round_trips` (Clock) MIGRATED to the spec/syntax corpus (inc-6):
    // spec/syntax/ml/13-world-full-decl + ml/12-world-nullary-member pin those inline-`world` parse trees
    // (render_sexpr) + canonical formats (fmt) language-neutrally, graded by the per-case nix check + the
    // self-consistency test. The `contains(…)` sanity checks are subsumed by the byte-exact `format.cdz`
    // goldens; the `assert_roundtrip` idempotence by the corpus's fmt-idempotence property.

    // `inline_world_wit_type_descriptors_round_trip` (string/list(u8)/option(list) + the str-head
    // descriptor storage `("list" (u8))`/`(result (u8))`) and `inline_world_unit_and_tuple_member_types_round_trip`
    // (`("unit")` result + `("tuple" …)`) MIGRATED to the spec/syntax corpus (inc-6):
    // ml/14-world-wit-prim-list-option + ml/15-world-wit-list-result + ml/16-world-unit-tuple pin those
    // inline-`world` WIT-type parse trees (render_sexpr — the tree.sexp carries the exact str-head
    // descriptors these tests asserted via sexpr::print) + canonical formats, graded by the per-case
    // nix check + the self-consistency test. Round-trip idempotence ← the corpus's fmt-idempotence.

    #[test]
    fn inline_world_record_member_type_round_trips() {
        // A record TYPE `{f: T, …}` as a world-member param/result lowers to the canonical str-head
        // descriptor `("record" (f <T>)…)` (matching rcdzc's parse_wit_type + cadenza-ast wit_type_record)
        // AND prints back to the brace surface, so a world binding a record member round-trips ML->ML. This
        // is the shape a reducer guest uses for a message/request record (v-platform's reducer world).
        let printed = assert_roundtrip(
            "world W = | export i = \
             | step : (msg : {id : string, payload : list(u8)}) -> {ok : bool}",
            80,
        );
        assert!(
            printed.contains("msg : {id: string, payload: list(u8)}"),
            "record param round-trips as a brace type: {printed}"
        );
        assert!(
            printed.contains("-> {ok: bool}"),
            "record result round-trips as a brace type: {printed}"
        );
        // The stored descriptor is the canonical str-head record form (a `(name ty)` pair per field, field
        // types themselves lowered), NOT the name-head `(Record (: …))` type-application node.
        let parsed = parser::read_ml(
            "world W = | export i = | step : (msg : {id : string, payload : list(u8)}) -> bool",
        );
        let sexp = sexpr::print(&parsed.arenas);
        assert!(
            sexp.contains("(\"record\" (id (string)) (payload (\"list\" (u8))))"),
            "record stored as canonical str-head descriptor with lowered field types, got: {sexp}"
        );
        assert!(
            !sexp.contains("(Record"),
            "the name-head (Record (: …)) type node must be fully lowered, got: {sexp}"
        );
    }

    #[test]
    fn inline_world_result_member_type_round_trips() {
        // A `result` world-member type lowers to the canonical str-head `("result" <ok> <err>)` descriptor
        // (matching rcdzc's parse_wit_type + cadenza-ast wit_type_result) and prints back to the WIT-faithful
        // surface, so a world binding a result member round-trips ML->ML. All four arm-presence spellings —
        // both present, err absent, ok absent (`_`), and bare `result` — are covered; `Response.answer =
        // result<payload, error>` (v-platform's reducer world) is the both-present shape.
        let both = assert_roundtrip(
            "world W = | export i = | f : (x : u8) -> result(bool, string)",
            80,
        );
        assert!(
            both.contains("-> result(bool, string)"),
            "both arms present: {both}"
        );
        let no_err = assert_roundtrip("world W = | export i = | f : (x : u8) -> result(u8)", 80);
        assert!(no_err.contains("-> result(u8)"), "err arm absent: {no_err}");
        let no_ok = assert_roundtrip(
            "world W = | export i = | f : (x : u8) -> result(_, string)",
            80,
        );
        assert!(
            no_ok.contains("-> result(_, string)"),
            "ok arm absent (`_`): {no_ok}"
        );
        let bare = assert_roundtrip("world W = | export i = | f : (x : u8) -> result", 80);
        assert!(
            bare.contains("-> result") && !bare.contains("-> result("),
            "bare result (both arms absent): {bare}"
        );

        // The stored descriptors are the canonical str-head form; an absent arm is the ("none") marker.
        let parsed = parser::read_ml(
            "world W = | export i = | f : (x : u8) -> result(bool, string) \
             | g : (x : u8) -> result(u8) \
             | h : (x : u8) -> result(_, string) \
             | k : (x : u8) -> result",
        );
        let sexp = sexpr::print(&parsed.arenas);
        assert!(
            sexp.contains("(\"result\" (bool) (string))"),
            "both-present stored as ('result' <ok> <err>), got: {sexp}"
        );
        assert!(
            sexp.contains("(\"result\" (u8) (\"none\"))"),
            "err-absent stored with ('none') err slot, got: {sexp}"
        );
        assert!(
            sexp.contains("(\"result\" (\"none\") (string))"),
            "ok-absent stored with ('none') ok slot, got: {sexp}"
        );
        assert!(
            sexp.contains("(\"result\" (\"none\") (\"none\"))"),
            "bare result stored with both ('none') slots, got: {sexp}"
        );
    }

    #[test]
    fn inline_world_variant_member_type_round_trips() {
        // A `variant(Case, Case2(T), …)` world-member type lowers to the canonical str-head
        // `("variant" (Case <T>?)…)` descriptor (matching rcdzc's parse_wit_type + cadenza-ast
        // wit_type_variant) and prints back, so a world binding a variant member round-trips ML->ML. A bare
        // case is payload-less; a `Case(T)` application carries a payload (itself lowered — here a record,
        // the shape v-platform's `outcome { continue, break(record) }` uses).
        let printed = assert_roundtrip(
            "world W = | export i = \
             | f : (x : u8) -> variant(Continue, Break({schema: string, reason: string}))",
            120,
        );
        assert!(
            printed.contains("-> variant(Continue, Break({schema: string, reason: string}))"),
            "variant with a payload-less case + a record-payload case round-trips: {printed}"
        );
        // The stored descriptor is the canonical str-head variant form: a payload-less case is a 1-list
        // `(Continue)`, a payload case is a 2-list `(Break <lowered-ty>)`.
        let parsed = parser::read_ml(
            "world W = | export i = | f : (x : u8) -> variant(Continue, Break(u8))",
        );
        let sexp = sexpr::print(&parsed.arenas);
        assert!(
            sexp.contains("(\"variant\" (Continue) (Break (u8)))"),
            "variant stored as str-head with (Case)/(Case ty) entries, got: {sexp}"
        );
    }

    #[test]
    fn inline_world_enum_and_flags_member_types_round_trip() {
        // `enum(A, …)` and `flags(A, …)` world-member types lower to the canonical str-head `("enum" A …)`
        // / `("flags" A …)` descriptors (bare-NAME cases/bits, matching rcdzc's parse_wit_type +
        // cadenza-ast wit_type_enum/wit_type_flags) and print back, so a world binding them round-trips
        // ML->ML. They share the node shape but are DISTINCT types — the head keyword selects which.
        let printed = assert_roundtrip(
            "world W = | export i = \
             | color : (x : u8) -> enum(Red, Green, Blue) \
             | perms : (x : u8) -> flags(Read, Write)",
            100,
        );
        assert!(
            printed.contains("-> enum(Red, Green, Blue)"),
            "enum round-trips: {printed}"
        );
        assert!(
            printed.contains("-> flags(Read, Write)"),
            "flags round-trips: {printed}"
        );
        // Stored as the canonical str-head descriptors with bare-NAME children (NOT the name-head `(enum …)`
        // application), and enum vs flags stay distinct.
        let parsed = parser::read_ml(
            "world W = | export i = | color : (x : u8) -> enum(Red, Green) \
             | perms : (x : u8) -> flags(Read, Write)",
        );
        let sexp = sexpr::print(&parsed.arenas);
        assert!(
            sexp.contains("(\"enum\" Red Green)"),
            "enum stored as str-head with bare-name cases, got: {sexp}"
        );
        assert!(
            sexp.contains("(\"flags\" Read Write)"),
            "flags stored as str-head with bare-name bits, got: {sexp}"
        );
    }

    #[test]
    fn inline_world_deeply_nested_aggregate_member_type_round_trips() {
        // Hardening: the WIT type-descriptor lower (`wit_type_desc_of`) and its printer inverse recurse, so a
        // member type composing EVERY aggregate — option over a result whose ok is a record with list fields
        // and whose err is a variant with a payload — must lower fully (no name-head aggregate node left
        // un-lowered anywhere in the tree) and print back to the same surface. This pins the recursion
        // against a drift that only shows under composition (e.g. a lower/print arm that forgets to recurse
        // into a nested slot). A realistic key/value get-with-tags-or-error shape.
        let ty = "option(result({val: list(u8), tags: list(string)}, variant(NotFound, Corrupt(string))))";
        let src = format!("world W = | export i = | get : (key : string) -> {ty}");
        let printed = assert_roundtrip(&src, 200);
        assert!(
            printed.contains(&format!("-> {ty}")),
            "the deeply-nested member type round-trips to the same surface: {printed}"
        );
        // Every aggregate is fully lowered to its canonical str-head descriptor — no name-head `(Record`/
        // `(option`/`(result`/`(variant` application node survives anywhere in the stored tree.
        let parsed = parser::read_ml(&src);
        let sexp = sexpr::print(&parsed.arenas);
        assert!(
            sexp.contains(
                "(\"option\" (\"result\" \
                 (\"record\" (val (\"list\" (u8))) (tags (\"list\" (string)))) \
                 (\"variant\" (NotFound) (Corrupt (string)))))"
            ),
            "the whole nested type lowers to composed str-head descriptors, got: {sexp}"
        );
        // No un-lowered name-head aggregate TYPE node should survive. (`(result …)` is EXCLUDED: it also
        // names the func signature's result-slot wrapper — a legitimate structural node — so it cannot
        // discriminate an un-lowered result type; the positive assertion above already proves result lowered.)
        for name_head in ["(Record", "(option ", "(variant ", "(Tuple"] {
            assert!(
                !sexp.contains(name_head),
                "no un-lowered name-head aggregate node `{name_head}` should survive, got: {sexp}"
            );
        }
    }

    #[test]
    fn a_nested_do_in_a_greedy_body_keeps_its_block_boundary_across_the_ml_round_trip() {
        // Regression (v-metaprogramming handoff, breaker bucket do:1): a nested `(do B C)` used as the
        // BODY of a handle/let/fn — a bare `expr(0)` body rendered by print_do_stmts — used to have its
        // inner do INLINED into the parent sequence, dropping the inner-do node. The reparse then yields
        // the FLAT `(do … B C)` (one fewer AST node). Idempotent (both sides flatten identically), so
        // assert_roundtrip alone can't catch it — we need STRUCTURAL equality of the original arena vs
        // the ML re-parse. Covers the final slot, a non-final slot, and the handle-body repro from the
        // issue; each inner `(do …)` must survive as its own block.
        for src in [
            // handle body — the minimal harness repro from the issue (one outer `do`, export inside)
            "(do (effect E (op tick (-> Int64))) \
               (def (main (: n Int64)) \
                 (handle E (% n 3) ((tick () s (resume s (+ s 1)))) \
                   (do ((. E tick)) (do ((. E tick)) ((. E tick)))))) \
               (export main))",
            // let body, nested-do in the FINAL statement slot
            "(def (main) (let ((x 1)) (do (a) (do (b) (c)))))",
            // let body, nested-do in a NON-FINAL statement slot
            "(def (main) (let ((x 1)) (do (do (a) (b)) (c))))",
            // fn body, nested-do final slot
            "(def (f (x)) (do (a) (do (b) (c))))",
        ] {
            let a = sexpr::read(src).expect("sexpr parses");
            let printed = print(&a, 80);
            let back = parser::read_ml(&printed);
            assert!(back.ok(), "reparse of {printed:?}: {:?}", back.errors);
            assert!(
                a.structurally_eq(&back.arenas),
                "nested-do block boundary lost in round-trip\n src:  {src}\n ml:   {printed}\n \
                 back: {}",
                sexpr::print(&back.arenas)
            );
        }
    }

    #[test]
    fn a_world_headed_form_that_is_not_a_world_decl_falls_back_to_the_generic_form() {
        // A `(world x)` that is NOT a well-shaped world decl (no interfaces) must NOT crash print_world;
        // is_world_shape declines it and it prints as the generic call form, round-tripping.
        assert_roundtrip("world(x)", 80);
    }

    #[test]
    fn every_reserved_word_used_as_a_bare_name_backtick_round_trips_to_a_name() {
        // A NAME leaf whose text collides with a keyword or word-operator (`let`, `if`, `match`, `and`,
        // `or`, …) MUST print backtick-quoted (`` `let` ``) so it re-reads as that NAME, not as the
        // reserved word — otherwise `def main() = let` would re-parse as a broken `let` form (a silent
        // corruption of the author's identifier). `name_is_bare_safe` gates this via `token::is_reserved`,
        // but no test swept the WHOLE reserved set through the printer→parser round-trip. Pin it here so a
        // new keyword added to `token::keyword` (or a drift in the escape predicate) can't quietly make a
        // bare name matching it re-lex as the keyword. Each reserved word is placed as a def body (an
        // expression-position Name); the round-trip must yield the identical arena.
        //
        // The reserved set = every `token::keyword` + every `token::word_op`. Kept in lockstep with those
        // tables by the assertion below (each listed word IS reserved, and the count matches), so a word
        // added to `keyword`/`word_op` but not here fails the count check rather than going unpinned.
        let reserved = [
            "let", "in", "if", "then", "else", "fn", "def", "type", "match", "with", "module",
            "import", "export", "effect", "handle", "host", "as", "forall", // token::keyword
            "and", "or", // token::word_op
        ];
        for w in reserved {
            assert!(
                token::is_reserved(w),
                "{w:?} is listed here but token::is_reserved says it is NOT reserved — the printer would \
                 print it BARE and it would re-lex as an identifier or (if a new keyword) the keyword"
            );
            // Build `(def (main) <w-as-Name>)` directly in the arena so we don't lean on the ML parser to
            // construct the input (the parser would reject a bare reserved word in body position).
            let mut b = Builder::new();
            let main_head = b.name("main");
            let main_sig = b.list(vec![main_head]);
            let def_head = b.name("def");
            let body = b.name(w); // the reserved word AS A BARE NAME leaf
            let root = b.list(vec![def_head, main_sig, body]);
            let a = b.finish(root);

            let ml = print(&a, 80);
            // The printer MUST have backtick-quoted it (a bare `w` would re-lex as the reserved word).
            assert!(
                ml.contains(&format!("`{w}`")),
                "reserved name {w:?} must print backtick-quoted, got ML: {ml:?}"
            );
            let back = parser::read_ml(&ml);
            assert!(
                back.ok(),
                "backtick-quoted reserved name {w:?} must re-parse clean, got {:?} for ML {ml:?}",
                back.errors
            );
            assert!(
                a.structurally_eq(&back.arenas),
                "reserved name {w:?} did not round-trip faithfully; ML was {ml:?}"
            );
        }
        // Guard the set against `token::keyword`/`word_op` drift: a new reserved word not listed above
        // would go unpinned. `token` exposes no iterator over the reserved set, so pin the COUNT (18
        // keywords + 2 word-ops = 20) — adding a keyword bumps the real count and fails this, prompting an
        // addition here. (A crude but effective lockstep, mirroring `INFIX_HEADS`/`every_infix_head_*`.)
        assert_eq!(
            reserved.len(),
            20,
            "reserved-word count changed — add the new keyword/word-op to this round-trip sweep"
        );
    }

    #[test]
    fn ml_print_is_depth_guarded_not_a_stack_overflow_on_a_deep_arena() {
        // The ML printer is a mutually-recursive machine (`expr`→`list`→shape helpers→`expr`), one native
        // frame per level; `print` runs on arenas from ANY source, including a decoded binary AST that
        // `codec::decode` accepts at ARBITRARY depth (no cap, unlike the reader's MAX_NESTING_DEPTH). A
        // deep tree overflowed the native stack (SIGABRT) on `cdz convert binary → ml`. Unlike the s-expr
        // printers (rewritten to explicit stacks), the ML printer is too mutually-recursive to iterativize
        // cheaply, so `expr` is DEPTH-GUARDED at MAX_PRINT_DEPTH (elides past it). A chain far deeper than
        // the guard must render — bounded, no overflow, with the elision sentinel present. Run on a big
        // stack because even MAX_PRINT_DEPTH (4096) frames exceed a default test worker (the guard bounds
        // native use to a fixed ceiling; the compiler's own deep walks use the same 64 MB-sized stack).
        let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let depth = (MAX_PRINT_DEPTH as usize) + 5_000; // well past the guard
                let mut b = Builder::new();
                let mut cur = b.name("x");
                for _ in 0..depth {
                    cur = b.list(vec![cur]);
                }
                let a = b.finish(cur);
                let out = print(&a, 80); // must NOT overflow — the guard bounds recursion
                assert!(
                    out.contains('…'),
                    "past MAX_PRINT_DEPTH the printer elides with `…`"
                );
                // A chain JUST under the guard renders fully (no elision) — the guard doesn't fire early.
                let shallow_depth = (MAX_PRINT_DEPTH as usize) - 10;
                let mut b2 = Builder::new();
                let mut cur2 = b2.name("y");
                for _ in 0..shallow_depth {
                    cur2 = b2.list(vec![cur2]);
                }
                let a2 = b2.finish(cur2);
                let out2 = print(&a2, 80);
                assert!(!out2.contains('…'), "under the guard, nothing is elided");
                assert!(
                    out2.contains('y'),
                    "the deep-but-under-guard leaf is rendered"
                );
            })
            .expect("spawn big-stack printer worker");
        if let Err(p) = h.join() {
            std::panic::resume_unwind(p);
        }
    }

    #[test]
    fn ml_print_is_depth_guarded_on_a_deep_pattern_arena_not_a_stack_overflow() {
        // `pattern` is a SECOND printer recursion hub (a tuple/list/ctor sub-pattern re-enters it),
        // SEPARATE from `expr`'s — so `expr`'s MAX_PRINT_DEPTH guard never bounded it. A decoded-only
        // deep pattern arena (the reader caps at MAX_NESTING_DEPTH, but `codec::decode` accepts arbitrary
        // depth) overflowed the native stack (SIGABRT) when the ML printer walked it — the printer-side
        // twin of the reader-side pattern guard. `pattern` now shares the MAX_PRINT_DEPTH/`self.depth`
        // budget and elides past it. A pattern far deeper than the guard must render bounded (elision
        // present), no overflow. Big (64 MB) stack like the sibling `expr`-guard test — even the GUARDED
        // MAX_PRINT_DEPTH (4096) frames exceed a default worker; the guard bounds native use to that fixed
        // ceiling (without it the walk was unbounded → SIGABRT regardless of stack size).
        let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let n = (MAX_PRINT_DEPTH as usize) + 20_000; // well past the guard
                let mut b = Builder::new();
                let tuple = b.name("tuple");
                let mut pat = b.name("x");
                for _ in 0..n {
                    pat = b.list(vec![tuple, pat]); // nested `(tuple <sub>)` pattern
                }
                let body = b.name("z");
                let arm = b.list(vec![pat, body]);
                let matchkw = b.name("match");
                let scrut = b.name("y");
                let root = b.list(vec![matchkw, scrut, arm]);
                let a = b.finish(root);
                let out = print(&a, 80); // must NOT overflow — the pattern guard bounds recursion
                assert!(
                    out.contains('…'),
                    "past MAX_PRINT_DEPTH the pattern printer elides with `…`"
                );
            })
            .expect("spawn small-stack printer worker");
        if let Err(p) = h.join() {
            std::panic::resume_unwind(p);
        }
    }

    #[test]
    fn const_force_eval_expression_round_trips_to_a_fixed_point() {
        // `const(expr)` is the compile-time force-eval EXPRESSION form: an ordinary application of the
        // head name `const` to one argument, so it lowers to the homoiconic list `(const EXPR)` — an
        // `Ast.List` headed by `Name "const"`, NOT a bespoke codec node (v-inference/force-eval resolve
        // the head). The ML surface must reach a FIXED POINT `ml(ml(x)) == ml(x)` for it, across the
        // expression shapes a user writes.
        for src in [
            "const(x)",
            "const(f(a))",
            "const(1 + 2)",
            "const(const(x))",
            "y + const(x)",
        ] {
            assert_roundtrip(src, 80);
        }
        // The lowering is the plain `(const EXPR)` list, so the s-expr surface round-trips it for free
        // (no codec change) — this pins that the ML surface produces exactly that homoiconic shape.
        let parsed = parser::read_ml("const(f(a))");
        assert!(parsed.ok(), "parse const(f(a)): {:?}", parsed.errors);
        let sexp = sexpr::print(&parsed.arenas);
        assert!(
            sexp.contains("(const (f a))"),
            "const(expr) lowers to the homoiconic list, got: {sexp}"
        );
    }

    #[test]
    fn const_expression_is_distinct_from_the_const_param_modifier() {
        // DISAMBIGUATION: `const(expr)` in EXPRESSION position (force-eval) and a `const`-prefixed
        // PARAMETER (the compile-time-parameter modifier, `(const BINDER)`) are told apart by POSITION,
        // not by a different head — the expression is a call, the modifier prefixes a param binder.
        // Round-tripping a def whose parameter carries the modifier through the ML surface must preserve
        // the `(const x)` binder shape (it must NOT be re-read as a force-eval expression), so the two
        // uses of the head name `const` never collide.
        let arenas = sexpr::read("(def f ((const x)) x)").expect("read const-param def sexp");
        let ml = print(&arenas, 80);
        let reparsed = parser::read_ml(&ml);
        assert!(reparsed.ok(), "reparse {ml:?}: {:?}", reparsed.errors);
        let sexp = sexpr::print(&reparsed.arenas);
        assert!(
            sexp.contains("((const x))"),
            "const param modifier survives the ML surface as `(const x)` in param position, \
             got ml={ml:?} sexp={sexp}"
        );
    }

    #[test]
    fn import_alias_and_named_list_forms_round_trip() {
        // The whole-module ALIAS import `import alias from "path"` -> `(import "path" alias)` (bare-name
        // third element — the linker's module-alias discriminant), distinct from the named-list
        // `import { a, b } from "path"` -> `(import "path" (a b))` (list third element). Both share the
        // `from "path"` tail and round-trip through the ML surface; the alias composes with member
        // projection `alias.member` — the shape a multi-contract guest uses to disambiguate two modules
        // that export the same name.
        assert_roundtrip("import kv-put from \"kv-put\"", 80);
        assert_roundtrip("import bput from \"blob-put\"", 80);
        assert_roundtrip("import { get, put } from \"kv\"", 80);
        // Per-name RENAME `import { orig as alias, … }` -> `(import "path" ((as orig alias) …))`: bind a
        // single export under a distinct local name (a reducer imports each contract's `descriptor`
        // under a unique name), mixed with plain names.
        assert_roundtrip("import { descriptor as foo, other } from \"path\"", 80);
        assert_roundtrip("import { descriptor as bput-desc } from \"blob-put\"", 80);
        let rename = parser::read_ml("import { descriptor as foo, other } from \"m\"");
        assert!(rename.ok(), "parse rename import: {:?}", rename.errors);
        assert!(
            sexpr::print(&rename.arenas).contains("(import \"m\" ((as descriptor foo) other))"),
            "per-name rename lowers to an (as orig alias) element, got: {}",
            sexpr::print(&rename.arenas)
        );
        let full = "import bput from \"blob-put\"\n\nimport bget from \"blob-get\"\n\n\
                    def main() = bput.descriptor().id == bget.descriptor().id\n\nexport { main }";
        assert_roundtrip(full, 80);
        // The alias lowers to the bare-NAME third element; the named-list to a LIST — the two forms the
        // linker (resolve/link) tells apart.
        let alias = parser::read_ml("import kv from \"kv-put\"");
        assert!(alias.ok(), "parse alias import: {:?}", alias.errors);
        assert!(
            sexpr::print(&alias.arenas).contains("(import \"kv-put\" kv)"),
            "alias import lowers to a bare-name third element, got: {}",
            sexpr::print(&alias.arenas)
        );
        let named = parser::read_ml("import { a, b } from \"m\"");
        assert!(
            sexpr::print(&named.arenas).contains("(import \"m\" (a b))"),
            "named-list import lowers to a name-list third element, got: {}",
            sexpr::print(&named.arenas)
        );
    }

    /// Parse ML, print it, re-parse — the printed form must yield a structurally-equal arena, and
    /// printing again must be byte-identical (idempotence).
    fn assert_roundtrip(src: &str, width: usize) -> String {
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse {src:?}: {:?}", a.errors);
        let printed = print(&a.arenas, width);
        let b = parser::read_ml(&printed);
        assert!(b.ok(), "reparse of printed {printed:?}: {:?}", b.errors);
        let printed2 = print(&b.arenas, width);
        assert_eq!(
            printed, printed2,
            "not idempotent: {src:?} -> {printed:?} -> {printed2:?}"
        );
        printed
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible fuzz without a dependency, matching the
    /// lexer/parser/sexpr/codec house style (the crate stays "plain").
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

    #[test]
    fn printer_is_total_on_arbitrary_input_and_idempotent_on_clean_parses() {
        // The PRINTER half of the round-trip contract, on ARBITRARY input. `read_ml` recovers (never
        // bails), so every fuzzed string yields an arena to print. Two tiers of invariant:
        //
        //   * UNIVERSAL (any input, incl. an error-recovery arena holding synthetic `<error>` marker
        //     nodes): `print` never PANICS, at any width, and its output RE-PARSES without panicking —
        //     the printer never emits un-lexable/un-parsable text.
        //   * IDEMPOTENCE (`print(read(print(x))) == print(x)`) — asserted ONLY when the original input
        //     parsed CLEANLY. The idempotence contract is for WELL-FORMED arenas; an arena recovered
        //     from malformed input contains `<error>` markers that are a best-effort recovery artifact,
        //     not a spec'd round-trip surface (e.g. `@` → `` @`<error>` `` re-parses to a different tree
        //     — a known, acceptable limitation, NOT a miscompile of any valid program).
        //
        // Widths vary so the layout engine's break decisions are exercised, not just the flat form.
        let alphabet: Vec<char> = "()[]{}|,;=>-+*/<:.@#`\"\\ \tabcdefimntxλ中0123456789\n"
            .chars()
            .collect();
        let mut rng = SplitMix64(0x7b1e_5111_0d1c_a5f1);
        for len in 0..=32usize {
            for _ in 0..80 {
                let s: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                let parsed = parser::read_ml(&s);
                let clean = parsed.ok();
                for width in [0usize, 1, 20, 80] {
                    let printed = print(&parsed.arenas, width); // must not panic
                    let reparsed = parser::read_ml(&printed); // must not panic
                    if clean {
                        // A clean parse's printed form must itself re-parse clean and print identically.
                        assert!(
                            reparsed.ok(),
                            "printed form of a clean parse must re-parse clean: {s:?} -> {printed:?}: \
                             {:?}",
                            reparsed.errors
                        );
                        let printed2 = print(&reparsed.arenas, width);
                        assert_eq!(
                            printed, printed2,
                            "printer not idempotent at width {width} for clean input {s:?}: \
                             {printed:?} -> {printed2:?}"
                        );
                    }
                }
            }
        }
        // Pathological shapes that stress the layout engine + minimal-paren logic (all parse clean).
        for s in [
            "a+a+a+a+a+a",
            "f(f(f(f(f(x)))))",
            "a.b.c.d.e.f",
            "1;2;3;4;5;6",
            "a:b:c:d",
        ] {
            for width in [0usize, 1, 40, 200] {
                let printed = assert_roundtrip(s, width); // clean-parse round-trip + idempotence
                let _ = printed;
            }
        }
    }

    /// Generate a random VALID program as S-EXPR text (bounded by `depth`) — a shape the ML printer can
    /// render (infix, call, `let`, `if`, `record`, nested). Read via `sexpr::read` it gives a valid arena
    /// to print AS ML; keeping generation in s-expr avoids depending on the ML printer we are testing.
    fn gen_ml_expressible(rng: &mut SplitMix64, depth: usize) -> String {
        let atoms = ["a", "b", "x", "y", "f", "g", "1", "42", "true"];
        if depth == 0 || rng.next().is_multiple_of(3) {
            return atoms[(rng.next() as usize) % atoms.len()].to_string();
        }
        let sub = |rng: &mut SplitMix64| gen_ml_expressible(rng, depth - 1);
        match rng.next() % 6 {
            0 => format!("(+ {} {})", sub(rng), sub(rng)),
            1 => format!("(f {} {})", sub(rng), sub(rng)),
            2 => format!("(if {} {} {})", sub(rng), sub(rng), sub(rng)),
            3 => format!("(let ((x {}) (y {})) {})", sub(rng), sub(rng), sub(rng)),
            // A value-RECORD field is the canonical `(= name value)` triple (RV1/RV2, Phase B), so the
            // generated arena matches what the parser emits and the printer round-trips faithfully. The head
            // is the native `#record(…)` ctor surface (`Leaf::Ctor`), matching what `read_ml` produces.
            4 => format!("#record((= x {}) (= y {}))", sub(rng), sub(rng)),
            _ => format!("(* {} (+ {} {}))", sub(rng), sub(rng), sub(rng)),
        }
    }

    /// Generate a random VALID match PATTERN as S-EXPR text (bounded by `depth`), spanning every pattern
    /// SURFACE the printer's `pattern()` dispatches — tuple/list/map/record/bin sequences, the `.`-dotted
    /// and ctor-application heads, guards, AND the quote/quasiquote forms + EMPTY compounds. The last two
    /// are the coverage gap that let the empty-compound quote-pattern `unreachable!()` reach breaker: the
    /// expression generator never emits a pattern, so the never-panic fuzz never rendered one. Kept in
    /// s-expr (like `gen_ml_expressible`) so generation never leans on the ML printer under test. Every
    /// arm chosen here round-trips FAITHFULLY (verified shapes), so the caller can assert structural
    /// equality, not just no-panic.
    fn gen_ml_pattern(rng: &mut SplitMix64, depth: usize) -> String {
        // Leaf patterns: a binder/wildcard name, a literal, or the two degenerate EMPTY compounds (the
        // empty raw-list `()` — the shape that panicked — and the empty record/bin, which have their own
        // printer arms). All read back to the same node.
        let leaves = ["a", "x", "_", "1", "true", "()", "(record)", "(bin)"];
        if depth == 0 || rng.next().is_multiple_of(3) {
            return leaves[(rng.next() as usize) % leaves.len()].to_string();
        }
        let sub = |rng: &mut SplitMix64| gen_ml_pattern(rng, depth - 1);
        // NOTE: no `guard` arm here — a guard `p if c` is only valid at a match arm's TOP level (the
        // reader rejects it NESTED inside a tuple/quote/etc.), so nesting it would build an arena with no
        // reader-reachable ML surface. The caller applies a guard at the arm level instead.
        match rng.next() % 11 {
            0 => format!("(tuple {} {})", sub(rng), sub(rng)),
            1 => format!("(tuple {})", sub(rng)), // 1-tuple: `(p,)`
            2 => format!("(list {} {})", sub(rng), sub(rng)),
            // A record PATTERN field is the canonical `(= field sub-pattern)` `FieldPair` triple (path B —
            // the SAME form as a value-record field, per the operator's full-symmetry ruling). The pattern
            // reader emits the shadowable NAME-alias head `(record …)` with `FieldPair` fields; the s-expr
            // reader `field_pairify`s a `(= k v)` DIRECT entry under a bare-name `record`/`map` alias head
            // (not only under `#record(…)`), so this authored form matches `read_ml`'s pattern arena.
            3 => format!("(record (= f {}) (= g {}))", sub(rng), sub(rng)),
            4 => format!("(map (= k {}))", sub(rng)),
            // ctor application `Ctor(p, …)` (name head, so it prints as an application, not a literal).
            5 => format!("(C {} {})", sub(rng), sub(rng)),
            // quote / quasiquote PATTERN — inner is itself a pattern, so a quote OVER an empty compound
            // (`(quote ())`) is reachable here, exercising the once-panicking path.
            6 => format!("(quote {})", sub(rng)),
            7 => format!("(quasiquote {})", sub(rng)),
            // NATIVE ctor-leaf-head compound PATTERNS (`#tuple`/`#list`/`#map`) — the canonical M2 form a
            // native compound match pattern carries (`Leaf::Ctor` head). These print through the SAME bracket
            // sugar as the name-alias heads above (`(a, b)`/`[a, b]`/`#{ k = p }`) — without the pattern
            // printer recognizing the native ctor head they fell to the generic `Ctor(p, …)` arm and printed
            // the classic `tuple(…)`/`list(…)`/`map(…)` call form, breaking idempotence (the ML compound-
            // PATTERN round-trip gap). Map entries are the canonical `FieldPair (= key sub)` triple —
            // symmetric with map VALUES and record-pattern fields (operator M3 ruling), which the reader
            // now emits for `#{ k = p }` patterns.
            8 => format!("#tuple({} {})", sub(rng), sub(rng)),
            9 => format!("#list({} {})", sub(rng), sub(rng)),
            _ => format!("#map((= k {}))", sub(rng)),
        }
    }

    #[test]
    fn ml_pattern_round_trip_is_faithful_and_never_panics_over_generated_patterns() {
        // The never-panic + faithful-round-trip contract for the PATTERN printer, swept — the coverage the
        // expression fuzz above lacks (it emits no patterns, so the empty-compound quote-pattern
        // `unreachable!()` slipped past it to breaker). For each random pattern, wrap it in a `match` and
        // assert: `print` never panics at any width, the printed ML re-parses clean, and the re-read arena
        // is STRUCTURALLY EQUAL to the source (the pattern node survives, including the empty compounds and
        // quote patterns). Generation is in s-expr so it never leans on the printer under test.
        let mut rng = SplitMix64(0x5eed_9a77_e40f_1c03);
        for _ in 0..3000 {
            let depth = 1 + (rng.next() % 4) as usize;
            let inner = gen_ml_pattern(&mut rng, depth);
            // A guard `p if c` is legal only at the arm's TOP level — apply it here (not inside
            // `gen_ml_pattern`) with 1/4 probability, so the `(guard …)` arm shape is still swept.
            let pat = if rng.next().is_multiple_of(4) {
                format!("(guard {inner} c)")
            } else {
                inner
            };
            let sx = format!("(def (main x) (match x ({pat} 1) (_ 0)))");
            let a = sexpr::read(&sx)
                .unwrap_or_else(|e| panic!("generated s-expr {sx:?} reads: {}", e.0));
            for &width in &[0usize, 1, 8, 30, 100] {
                let ml = print(&a, width); // must not panic
                let back = parser::read_ml(&ml);
                assert!(
                    back.ok(),
                    "ML print (w={width}) of pattern {sx:?} must re-parse clean, got {:?}\n\
                     --- ml ---\n{ml}",
                    back.errors
                );
                assert!(
                    a.structurally_eq(&back.arenas),
                    "ML print (w={width}) not faithful for pattern {sx:?}\n--- ml ---\n{ml}"
                );
            }
        }
    }

    /// Generate a random VALID top-level DECLARATION as S-EXPR text — a `(type …)` sum or an `(effect …)`,
    /// the two declaration-form surfaces `print_type`/`print_effect` render with their own `|`-led layout.
    /// The expression + pattern fuzz above never emit a declaration, so the type/effect PRINTERS (each
    /// layout-sensitive across widths) had no generative round-trip coverage — this closes that. Every
    /// shape here round-trips FAITHFULLY (verified), so the caller asserts structural equality across
    /// widths. Kept in s-expr so it never leans on the ML printer under test (matches the sibling gens).
    fn gen_ml_type_decl(rng: &mut SplitMix64) -> String {
        // A payload type atom (kept simple — a name or a type var; the point is the DECL layout, not deep
        // type expressions, which the expression fuzz's `:` ascriptions already exercise).
        fn ty(rng: &mut SplitMix64) -> String {
            const TYNAMES: [&str; 5] = ["Int", "String", "Bool", "a", "b"];
            TYNAMES[(rng.next() as usize) % TYNAMES.len()].to_string()
        }
        if rng.next().is_multiple_of(2) {
            // A SUM type. Optional type params `(T a b)`; 1..=4 variants, each with 0..=2 payload types.
            let nparams = (rng.next() % 3) as usize; // 0, 1, or 2 params
            let params: Vec<String> = (0..nparams)
                .map(|i| ((b'a' + i as u8) as char).to_string())
                .collect();
            let head = if params.is_empty() {
                "T".to_string()
            } else {
                format!("(T {})", params.join(" "))
            };
            let nvariants = 1 + (rng.next() % 4) as usize;
            let variants: Vec<String> = (0..nvariants)
                .map(|i| {
                    let ctor = ((b'A' + i as u8) as char).to_string();
                    let npayload = (rng.next() % 3) as usize; // 0, 1, or 2 payload types
                    if npayload == 0 {
                        format!("({ctor})")
                    } else {
                        let tys: Vec<String> = (0..npayload).map(|_| ty(rng)).collect();
                        format!("({ctor} {})", tys.join(" "))
                    }
                })
                .collect();
            format!("(type {head} (sum {}))", variants.join(" "))
        } else {
            // An EFFECT with 1..=3 operations `(op <name> <arg-ty> <ret-ty>)`.
            let nops = 1 + (rng.next() % 3) as usize;
            let ops: Vec<String> = (0..nops)
                .map(|i| {
                    let arg = ty(rng);
                    let ret = ty(rng);
                    format!("(op op{i} {arg} {ret})")
                })
                .collect();
            format!("(effect E {})", ops.join(" "))
        }
    }

    #[test]
    fn effect_op_resource_marker_round_trips_through_the_ml_surface() {
        // SEC-F1 `@resource` marker (concierge-ruled, v-agent-harness coord 2026-08-13). The parser LIFTS
        // `@resource T` off the marked param into a `(resource N)` decl-sibling (hash-clean); the printer
        // RE-INJECTS `@resource ` before the N-th param, so the surface round-trips ML->ML. Cover the
        // marker on the 1st + 2nd param, and confirm a no-marker effect is unchanged (regression).
        let printed =
            assert_roundtrip("effect Fs = | write : @resource Bytes -> Bytes -> Unit", 80);
        assert!(
            printed.contains("write : @resource Bytes -> Bytes -> Unit"),
            "1st-param resource marker round-trips: {printed}"
        );
        let p2 = assert_roundtrip("effect Fs = | store : Bytes -> @resource Bytes -> Unit", 80);
        assert!(
            p2.contains("store : Bytes -> @resource Bytes -> Unit"),
            "2nd-param resource marker round-trips: {p2}"
        );
        // A no-marker effect op is unaffected (no spurious @resource).
        let plain = assert_roundtrip("effect Fs = | read : Bytes -> Bytes", 80);
        assert!(
            !plain.contains("@resource"),
            "no marker => no @resource: {plain}"
        );
    }

    #[test]
    fn ml_type_and_effect_decl_round_trip_is_faithful_over_widths() {
        // The declaration-form printers (`print_type` sum surface, `print_effect` op surface) swept for
        // never-panic + faithful round-trip across widths — the coverage the expression/pattern fuzz lacks
        // (neither emits a top-level declaration, so a layout break in the `|`-led sum/effect surface at
        // some width would go uncaught). For each random decl: `print` never panics, the ML re-parses
        // clean, and the re-read arena is STRUCTURALLY EQUAL to the source. Generation is in s-expr.
        let mut rng = SplitMix64(0xda7a_7ec1_2c0d_e55e);
        for _ in 0..3000 {
            let sx = gen_ml_type_decl(&mut rng);
            let a = sexpr::read(&sx)
                .unwrap_or_else(|e| panic!("generated s-expr {sx:?} reads: {}", e.0));
            for &width in &[0usize, 1, 8, 30, 100] {
                let ml = print(&a, width); // must not panic
                let back = parser::read_ml(&ml);
                assert!(
                    back.ok(),
                    "ML print (w={width}) of decl {sx:?} must re-parse clean, got {:?}\n--- ml ---\n{ml}",
                    back.errors
                );
                assert!(
                    a.structurally_eq(&back.arenas),
                    "ML print (w={width}) not faithful for decl {sx:?}\n--- ml ---\n{ml}"
                );
            }
        }
    }

    /// Generate a random QUANTITY / UNIT literal as ML SOURCE — `<num> <unit-chain>` — spanning the
    /// postfix-unit surface: a bare unit (`5 meter`), a glued RATE (`59 GiB/s`), a product (`3 kg*m`),
    /// and exponents (`9 m/s^2`). The four sibling generators emit NO unit literals, so the postfix-unit
    /// sugar + the compound-unit chain (`maybe_unit_suffix`/`compound_unit_tail`) had no GENERATIVE
    /// round-trip coverage across widths. Unlike the siblings this generates ML SOURCE directly (the
    /// feature under test is the PARSE of the glued surface; s-expr has no postfix-unit sugar), then the
    /// caller round-trips read_ml → print → read_ml for structural stability.
    fn gen_ml_quantity(rng: &mut SplitMix64) -> String {
        const UNITS: [&str; 6] = ["meter", "second", "GiB", "kg", "m", "s"];
        let mag = ["5", "42", "2.5", "100", "1"][(rng.next() as usize) % 5];
        let u = |rng: &mut SplitMix64| UNITS[(rng.next() as usize) % UNITS.len()];
        // A unit FACTOR: a name, optionally a glued `^n` exponent (1..=3).
        let factor = |rng: &mut SplitMix64| {
            let name = u(rng);
            if rng.next().is_multiple_of(3) {
                format!("{name}^{}", 1 + (rng.next() % 3))
            } else {
                name.to_string()
            }
        };
        // 1..=3 factors joined by glued `/` or `*` — a compound/rate unit (or a single factor).
        let nfactors = 1 + (rng.next() % 3) as usize;
        let mut unit = factor(rng);
        for _ in 1..nfactors {
            let op = if rng.next().is_multiple_of(2) {
                "/"
            } else {
                "*"
            };
            unit.push_str(op);
            unit.push_str(&factor(rng));
        }
        format!("{mag} {unit}")
    }

    #[test]
    fn ml_quantity_and_compound_unit_literals_round_trip_over_widths() {
        // The postfix-unit / compound-rate surface (`5 meter`, `59 GiB/s`, `9 m/s^2`, `3 kg*m`) swept for
        // structural round-trip across widths — the coverage the expr/pattern/decl/annotation generators
        // lack (none emit a unit literal). For each random quantity ML source: print never panics, the ML
        // re-parses clean at every width, and re-read is structurally equal to the first read (parse →
        // print → parse stability). Generation is ML SOURCE (the parse of the glued surface is the feature
        // under test; the desugared arena has no glue sugar to regenerate).
        let mut rng = SplitMix64(0x9107_1740_c0de_5197);
        for _ in 0..3000 {
            let src = format!("def main() = {}", gen_ml_quantity(&mut rng));
            let a = parser::read_ml(&src);
            assert!(a.ok(), "generated quantity {src:?} parses: {:?}", a.errors);
            for &width in &[0usize, 1, 8, 30, 100] {
                let ml = print(&a.arenas, width); // must not panic
                let back = parser::read_ml(&ml);
                assert!(
                    back.ok(),
                    "ML print (w={width}) of {src:?} must re-parse clean, got {:?}\n--- ml ---\n{ml}",
                    back.errors
                );
                assert!(
                    a.arenas.structurally_eq(&back.arenas),
                    "ML print (w={width}) not faithful for quantity {src:?}\n--- ml ---\n{ml}"
                );
            }
        }
    }

    #[test]
    fn ml_comment_and_doc_wrapped_programs_round_trip_over_widths() {
        // The ANNOTATION-node surface — `(comment "…" form)` (`//`), a leading `(comment "…")` statement,
        // an inner `(comment "…" expr)` in a def body, and a file-header `(module-doc "…")` before a
        // non-documentable form (`///`) — swept for never-panic + faithful round-trip across widths. The
        // expression/pattern/decl generators emit NO annotation nodes, so the printer's comment/doc layout
        // (which threads hardbreaks + the `///`-vs-`//` marker distinction) had no generative round-trip
        // coverage — a break here would silently downgrade a `///` to `//` or drop a comment at some width.
        // Every shape is verified reader-reachable + faithful; generation is in s-expr (no lean on the
        // printer under test). Non-reader-reachable degenerate arenas (a lone `(module-doc)`, a malformed
        // `(comment "x")` missing its wrapped node) are covered for TOTALITY by
        // `printer_is_total_on_arbitrary_input…`, not here — here we pin the fidelity of the real shapes.
        let mut rng = SplitMix64(0xc011_7ee5_d0c5_a11e);
        for _ in 0..3000 {
            let depth = 1 + (rng.next() % 3) as usize;
            let inner = gen_ml_expressible(&mut rng, depth);
            // Wrap the generated expr in one of the reader-reachable annotation shapes, all inside a
            // `def main` so `print` takes the real top-level path.
            let sx = match rng.next() % 5 {
                // `//` comment attached to the def (wraps the whole def).
                0 => format!("(comment \"note\" (def (main) {inner}))"),
                // A leading `//` statement comment before the def (a `(do …)` with a bare comment first).
                1 => format!("(do (comment \"lead\") (def (main) {inner}))"),
                // An inner `//` comment on the def body expression.
                2 => format!("(def (main) (comment \"inner\" {inner}))"),
                // A `///` file-header module-doc before a non-documentable form (an int, not a def — a def
                // would DRAIN the doc as its own docstring; the module-doc path needs a non-doc-consuming
                // form after it).
                3 => format!("(do (module-doc \"header\") {inner})"),
                // A `(comment-after …)` trailing node (the same-line trailing comment surface).
                _ => format!("(def (main) (comment-after \"trail\" {inner}))"),
            };
            let a = sexpr::read(&sx)
                .unwrap_or_else(|e| panic!("generated s-expr {sx:?} reads: {}", e.0));
            for &width in &[0usize, 1, 8, 30, 100] {
                let ml = print(&a, width); // must not panic
                let back = parser::read_ml(&ml);
                assert!(
                    back.ok(),
                    "ML print (w={width}) of {sx:?} must re-parse clean, got {:?}\n--- ml ---\n{ml}",
                    back.errors
                );
                assert!(
                    a.structurally_eq(&back.arenas),
                    "ML print (w={width}) not faithful for {sx:?}\n--- ml ---\n{ml}"
                );
            }
        }
    }

    #[test]
    fn ml_print_round_trip_is_faithful_over_generated_programs_and_widths() {
        // The ML printer's structural FIDELITY, swept: `read_ml(print(a, w))` is STRUCTURALLY EQUAL to
        // `a`, over random valid programs at a range of widths. The byte-soup sweep above pins that a
        // clean parse re-parses clean + prints idempotently — but NOT that the tree is unchanged; a
        // printer could re-parse-clean and be idempotent yet subtly alter the tree at some break width.
        // This asserts equality against the source arena (via a `def main = <expr>` wrapper so `print`
        // takes the real top-level path), across widths that force different layout breaks. (Generation
        // is in S-EXPR so it doesn't lean on the ML printer under test.)
        let mut rng = SplitMix64(0xa11d_e5c0_de5e_ed01);
        for _ in 0..3000 {
            let depth = 1 + (rng.next() % 4) as usize;
            let sx = format!("(def (main) {})", gen_ml_expressible(&mut rng, depth));
            let a = sexpr::read(&sx)
                .unwrap_or_else(|e| panic!("generated s-expr {sx:?} reads: {}", e.0));
            for &width in &[0usize, 1, 8, 30, 100] {
                let ml = print(&a, width);
                let back = parser::read_ml(&ml);
                assert!(
                    back.ok(),
                    "ML print (w={width}) of {sx:?} must re-parse clean, got {:?}\n--- ml ---\n{ml}",
                    back.errors
                );
                assert!(
                    a.structurally_eq(&back.arenas),
                    "ML print (w={width}) not faithful for {sx:?}\n--- ml ---\n{ml}"
                );
            }
        }
    }

    #[test]
    fn every_infix_operator_pairing_round_trips_through_minimal_parens() {
        // The minimal-parenthesization logic — the printer's `prec`/`prec+1` split against `infix_prec`
        // + `is_right_assoc` — is the part most sensitive to a precedence-table drift, yet the faithful
        // round-trip sweep only exercises ONE operator pair (`+` vs `*`). Here we sweep the WHOLE infix
        // grid: for every ordered pair (outer, inner) of arena operator heads, build BOTH `(outer (inner a
        // b) c)` (inner on the left spine) and `(outer a (inner b c))` (inner on the right spine) directly
        // in the arena, print to ML, re-read, and assert STRUCTURAL EQUALITY. If the printer omits a
        // needed paren (inner binds looser and would re-associate) or the parser disagrees on a tier, the
        // re-read tree differs and this fails — naming the exact operator pair and spine. Generation is at
        // the arena level (via s-expr heads), so it never leans on the printer under test to construct the
        // input. Covers comparisons, bitwise (`| ^ &`), shifts (`<< >>`), `%`, and the right-associative
        // arrow `->` against every other operator — the pairings no existing test reaches.
        //
        // Arena heads that `infix_prec` recognizes AND that a bare `(op a b)` arena prints+reparses as an
        // ordinary binary infix. `:`/`->` are the type-level operators (ascription/arrow); the arithmetic,
        // comparison, bitwise, shift set is the value grid. `=` is equality (surface `==`). The `Unit.*`
        // family is excluded here (it renders via the units glyph path, covered by the unit tests).
        let ops = [
            "->", "|>", "or", "and", "=", "<", ">", "<=", ">=", "|", "^", "&", "<<", ">>", "+",
            "-", "*", "/", "%",
        ];
        // Sanity: each op is one the shared precedence table recognizes (guards a typo in this list).
        for op in ops {
            assert!(
                token::infix_prec(op).is_some(),
                "test op {op:?} must be in infix_prec"
            );
        }
        // A single `(op a b)` must itself round-trip — if an operator's bare form doesn't, the grid
        // results below would be meaningless. Assert it up front so a broken operator fails clearly here
        // (the split_template_body lesson: check the primitive before the combinations).
        for op in ops {
            let sx = format!("(def (main) ({op} a b))");
            let a = sexpr::read(&sx).unwrap_or_else(|e| panic!("{sx:?}: {}", e.0));
            let back = parser::read_ml(&print(&a, 80));
            assert!(
                back.ok() && a.structurally_eq(&back.arenas),
                "single-op form {op:?} must round-trip; ml={:?}",
                print(&a, 80)
            );
        }
        // The grid: every (outer, inner) pair, inner nested on the left AND the right spine, at a range of
        // widths (the break decisions must not change the parenthesization).
        for outer in ops {
            for inner in ops {
                for spine in ["left", "right"] {
                    let sx = match spine {
                        "left" => format!("(def (main) ({outer} ({inner} a b) c))"),
                        _ => format!("(def (main) ({outer} a ({inner} b c)))"),
                    };
                    let a = sexpr::read(&sx).unwrap_or_else(|e| panic!("{sx:?}: {}", e.0));
                    for &width in &[0usize, 1, 12, 40, 200] {
                        let ml = print(&a, width);
                        let back = parser::read_ml(&ml);
                        assert!(
                            back.ok(),
                            "grid ({outer},{inner},{spine}) w={width} must re-parse clean: {sx:?}\n\
                             --- ml ---\n{ml}\n{:?}",
                            back.errors
                        );
                        assert!(
                            a.structurally_eq(&back.arenas),
                            "grid ({outer},{inner},{spine}) w={width} NOT faithful — a paren/precedence \
                             drift changed the tree for {sx:?}\n--- ml ---\n{ml}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn as_conversion_does_not_break_before_as_so_it_round_trips_at_every_width() {
        // Regression: the `as` unit-conversion used a BREAKABLE space before `as`, so at a narrow width
        // the printer emitted `<value>⏎  as <unit>` — which then FAILED to re-parse, because the `as`
        // operator declines a leading `as` across a newline (the statement-sequencing guard). A chained
        // conversion (`… as millimeter … as centimeter`) is wide enough to trigger the break at the
        // default width. The fix makes ` as ` a NON-breaking space; assert the value round-trips at every
        // width (the value still wraps internally, but ` as <unit>` stays glued to its last line).
        let src = concat!(
            "(Unit.in (Unit.of #\"centimeter\") ",
            "(Qty.of (Unit.in (Unit.of #\"millimeter\") ",
            "(Qty.of (Rational.of 1 1) (Unit.of #\"inch\"))) (Unit.of #\"millimeter\")))"
        );
        let a = sexpr::read(src).unwrap();
        for w in [0usize, 1, 20, 40, 80, 100] {
            let ml = print(&a, w);
            let back = parser::read_ml(&ml);
            assert!(
                back.ok(),
                "the `as`-conversion must re-parse clean at width {w}: {ml:?} errs={:?}",
                back.errors
            );
            assert!(
                back.arenas.structurally_eq(&a),
                "the `as`-conversion round-trips structurally at width {w}: {ml:?}"
            );
        }
    }

    #[test]
    fn ml_printer_is_total_over_sexpr_sourced_arenas() {
        // The ML printer runs on ANY arena, including ones the ML READER can never build — a bare empty
        // list `()`, an empty list in head/operand position, a construct head applied to too-few/too-many
        // children. Those reach the ML printer via `cdz convert --from sexpr --to ml`. The `read_ml`-only
        // fuzz above misses them (the reader always fills a slot); the `(unquote ())` panic that fuzz
        // caught was exactly this class, so source arenas from `sexpr::read` too and assert the ML printer
        // is TOTAL over them: `print` never panics at any width, and its output re-parses (the ML reader
        // never panics on it either). No idempotence claim — an arena the ML surface can't express need
        // not round-trip through it; the invariant is no-crash + re-parsable.
        let alphabet: Vec<char> = "()\"#\\b. ,;|=>-+*/<:@`0123456789abcxeNR_\tλ中\n"
            .chars()
            .collect();
        let mut rng = SplitMix64(0x5153_7e37_c0de_a5f1);
        let mut clean = 0usize;
        for len in 0..=32usize {
            for _ in 0..160 {
                let s: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                // Only well-formed s-expr arenas — the point is arenas the ML surface can't make, not
                // s-expr parse errors (the s-expr reader's own fuzz covers those).
                let Ok(arena) = sexpr::read(&s) else { continue };
                clean += 1;
                for width in [0usize, 1, 20, 80] {
                    let printed = print(&arena, width); // must not panic on a sexpr-only shape
                    let _ = parser::read_ml(&printed); // its output must not panic the ML reader
                }
            }
        }
        // Directly-built empty/odd shapes that stress the printer's construct-head arms head-on — the
        // heads with special layout (let/fn/match/if/annotation/member/unquote) given an EMPTY body.
        for head in [
            "let",
            "fn",
            "match",
            "if",
            "def",
            "module",
            "do",
            "@",
            ".",
            "unquote",
            "quasiquote",
            "->",
            ":",
            "list",
            "tuple",
            "record",
            "map",
        ] {
            for shape in [
                format!("({head})"),       // head, no children
                format!("({head} ())"),    // head + one empty-list child
                format!("({head} () ())"), // head + two empty-list children
                format!("(({head}))"),     // empty-headed-by-a-list
            ] {
                if let Ok(a) = sexpr::read(&shape) {
                    for width in [0usize, 1, 40] {
                        let printed = print(&a, width); // must not panic
                        let _ = parser::read_ml(&printed);
                    }
                }
            }
        }
        assert!(
            clean > 1000,
            "swept a meaningful space of sexpr arenas, got {clean}"
        );
    }

    #[test]
    fn display_printer_is_total_over_sexpr_sourced_arenas_including_malformed_quantities() {
        // `print_display` is a DISTINCT code path from `print` (the `display` flag routes through
        // `display_quantity` / `display_unit`, the `Rational` bare-resugar, and the root `(: v t)` strip),
        // yet only ~10 hand-picked WELL-FORMED value programs exercise it. It renders compiler-produced
        // values in the REPL / notebook, so it must be TOTAL — never panic — on ANY arena, including a
        // MALFORMED quantity/unit shape (a `Qty.of` / `Unit./` / `Unit.^` with the wrong arity or an
        // empty operand slot) that a value-producing path could hand it. `print`'s own totality is swept
        // by `ml_printer_is_total_over_sexpr_sourced_arenas`; this asserts the DISPLAY arms are equally
        // total. No round-trip claim (display output is not required to re-read); the invariant is
        // no-panic at every width, and — since display output should still be lexable text — that the
        // ML reader doesn't panic on it either.
        let alphabet: Vec<char> = "()\"#\\b. ,;|=>-+*/<:@`0123456789abcxeNR_\tλ中\n"
            .chars()
            .collect();
        let mut rng = SplitMix64(0xd150_1a70_1a1c_0de1);
        let mut clean = 0usize;
        for len in 0..=32usize {
            for _ in 0..160 {
                let s: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                let Ok(arena) = sexpr::read(&s) else { continue };
                clean += 1;
                for width in [0usize, 1, 20, 80] {
                    let printed = print_display(&arena, width); // DISPLAY mode must not panic
                    let _ = parser::read_ml(&printed); // its output must not panic the ML reader
                }
            }
        }
        // Directly-built quantity/unit shapes that drive the display-only arms head-on — malformed arities
        // and empty operand slots the value-rendering path could conceivably hand `print_display`.
        for shape in [
            "(: 1/3 Rational)",                             // the resugar arm
            "(: 8/1 Rational)",                             // integral rational (drops /1)
            "(Qty.of)",                                     // no value, no unit
            "(Qty.of 1/4)",                                 // value, no unit
            "(Qty.of 1/4 (Unit.base #\"meter\") extra)",    // arity overflow
            "(Qty.of 1/4 (Unit./ (Unit.base #\"meter\")))", // Unit./ missing an operand
            "(Qty.of 1/4 (Unit./))",                        // Unit./ no operands
            "(Qty.of 1/4 (Unit.^ (Unit.base #\"meter\")))", // Unit.^ missing the exponent
            "(Qty.of 1/4 (Unit.^))",                        // Unit.^ empty
            "(Qty.of 1/4 (Unit.base))",                     // Unit.base no name
            "(: (Qty.of 5.0 (Unit.base #\"meter\")) ())",   // empty type annotation
            "(: () Rational)",                              // empty value in an annotation
        ] {
            if let Ok(a) = sexpr::read(shape) {
                for width in [0usize, 1, 40, 80] {
                    let printed = print_display(&a, width); // must not panic on a malformed quantity
                    let _ = parser::read_ml(&printed);
                }
            }
        }
        assert!(
            clean > 1000,
            "swept a meaningful space of display-mode arenas, got {clean}"
        );
    }

    #[test]
    fn unquote_over_an_empty_list_does_not_panic() {
        // Regression (found by `printer_is_total_on_arbitrary_input_…`): `unquote_atomic` indexed
        // `items[0]` on a `Struct::List` WITHOUT checking it was non-empty, so an unquote wrapping an
        // EMPTY list — `(unquote ())`, a valid arena node — panicked the printer (`index out of bounds:
        // len 0`) instead of printing. `cdz convert` on that s-expr crashed. The printer must be TOTAL.
        let a = sexpr::read("(unquote ())").unwrap();
        let _ = print(&a, 80); // must not panic
        // Sibling empty-list shapes through the same predicate path.
        for src in [
            "(unquote ())",
            "(quasiquote ())",
            "(unquote (. ()))",
            "(unquote (. a))",
        ] {
            let a = sexpr::read(src).unwrap();
            let printed = print(&a, 80); // must not panic
            // And the printed form re-parses (the printer emits well-formed text).
            assert!(
                parser::read_ml(&printed).ok(),
                "{src:?} printed to non-reparsing {printed:?}"
            );
        }
    }

    #[test]
    fn an_empty_compound_quote_pattern_round_trips_and_never_panics() {
        // NEVER-PANIC regression: the pattern printer's list arm was `List(items) if !items.is_empty()`,
        // so an EMPTY `Struct::List([])` in pattern position (the inner `()` of a quote PATTERN
        // `(quote ())`) fell to the `_` catch-all, which assumed a `Struct::Atom` and hit `unreachable!()`
        // (verified by breaker + corpus-bugfix on `(match (quote ()) ((quote ()) 1) (_ 0))`). The fix: the
        // printer renders an empty list pattern as `#[]` (mirroring `list()`'s expr-position escape), and
        // the parser accepts `#[…]` as the raw-list pattern twin — so it round-trips to the same node.
        let oracle = "(match (quote ()) ((quote ()) 1) (_ 0))";
        let a = sexpr::read(oracle).unwrap();
        let ml = print(&a, 80); // must NOT panic
        assert!(
            ml.contains("quote(#[])"),
            "empty quote pattern renders `quote(#[])`: {ml}"
        );
        // The printed ML re-reads to a STRUCTURALLY IDENTICAL oracle (the empty-list node survives).
        let back = parser::read_ml(&ml);
        assert!(back.ok(), "reparse {ml:?}: {:?}", back.errors);
        assert_eq!(
            sexpr::print(&back.arenas).trim(),
            oracle,
            "empty-compound quote pattern round-trips to the same s-expr"
        );
        // Idempotent.
        assert_eq!(print(&back.arenas, 80), ml, "not idempotent: {ml}");
    }

    #[test]
    fn a_non_last_handler_arm_with_a_greedy_block_body_parenthesizes_so_it_round_trips() {
        // ARM-EXTENT regression (breaker report): a handler arm whose BODY is a greedy block form
        // (`match`/`let`/`if`) printed UNGUARDED, so its own `|`-led arms (a match) or trailing body
        // ran into the NEXT `| op` handler arm on re-parse — the re-reader swallowed the following
        // handler arm as an extra inner-`match` arm (`corpus_roundtrip` AST-mismatch). `print_match`
        // already guards a non-last arm's block body with `PREC_KEYWORD`; `print_handle_arm` now does
        // the same. Each case must re-parse STRUCTURALLY-EQUAL and print idempotently, at every width.
        let cases = [
            // A match-bodied FIRST arm, a plain SECOND arm — the reported shape.
            "(do (effect S (op a (-> Int64 Int64)) (op b (-> Int64 Int64))) \
             (def (main (: n Int64)) (handle S 0 \
               ((a (v) st (match v (0 (resume 1 st)) (_ (resume 2 st)))) \
                (b (w) st (resume w st))) \
               ((. S a) n))) (export main))",
            // A let-bodied non-last arm.
            "(do (effect S (op a (-> Int64 Int64)) (op b (-> Int64 Int64))) \
             (def (main (: n Int64)) (handle S 0 \
               ((a (v) st (let ((x 5)) (resume x st))) \
                (b (w) st (resume w st))) \
               ((. S a) n))) (export main))",
            // An if-bodied non-last arm.
            "(do (effect S (op a (-> Int64 Int64)) (op b (-> Int64 Int64))) \
             (def (main (: n Int64)) (handle S 0 \
               ((a (v) st (if (< v 0) (resume 0 st) (resume 1 st))) \
                (b (w) st (resume w st))) \
               ((. S a) n))) (export main))",
        ];
        for sx in cases {
            let a = sexpr::read(sx).unwrap_or_else(|e| panic!("oracle {sx:?}: {}", e.0));
            for &width in &[0usize, 20, 40, 100] {
                let ml = print(&a, width);
                let back = parser::read_ml(&ml);
                assert!(back.ok(), "reparse (w={width}) {ml:?}: {:?}", back.errors);
                assert!(
                    a.structurally_eq(&back.arenas),
                    "non-last block-bodied handler arm not faithful (w={width})\n\
                     --- ml ---\n{ml}\n--- input ---\n{}\n--- reread ---\n{}",
                    sexpr::print(&a),
                    sexpr::print(&back.arenas)
                );
                assert_eq!(
                    print(&back.arenas, width),
                    ml,
                    "not idempotent (w={width}): {ml}"
                );
            }
        }
        // A LAST arm needs no guard — a bare `match`-bodied final arm still prints WITHOUT wrapping
        // parens (nothing follows it at the arm level; `in` terminates it).
        let last_arm = "(do (effect S (op note (-> Int64 Int64))) \
             (def (main (: n Int64)) (handle S 0 \
               ((note (v) st (match v (0 (resume 1 st)) (_ (resume 2 st))))) \
               ((. S note) n))) (export main))";
        let a = sexpr::read(last_arm).unwrap();
        let ml = print(&a, 100);
        assert!(
            ml.contains("match v with") && !ml.contains("(match"),
            "a LAST handler arm's match body is NOT wrapped in parens:\n{ml}"
        );
    }

    #[test]
    fn quasiquote_family_round_trips() {
        // The metaprogramming surface: a `` `{…} `` quasiquote block with `,x` unquotes and `,@xs`
        // splices. The lexer has `quasiquote_sigils` (token level) and the printer has the `(unquote ())`
        // panic guard, but the FULL ML round-trip of the quasiquote/unquote/unquote-splicing family was
        // unpinned. Each must re-parse to a structurally-equal arena (heads `quasiquote`/`unquote`/
        // `unquote-splicing`) and print idempotently.
        assert_eq!(assert_roundtrip("`{a + b}", 80), "`{ a + b }");
        assert_eq!(assert_roundtrip("`{,x}", 80), "`{ ,x }");
        assert_eq!(assert_roundtrip("`{,a + ,b}", 80), "`{ ,a + ,b }");
        assert_eq!(assert_roundtrip("`{f(,@xs)}", 80), "`{ f(,@xs) }");
        assert_eq!(
            assert_roundtrip("`{f(,@args, last)}", 80),
            "`{ f(,@args, last) }"
        );
        // The s-expr oracle's canonical heads print back to the `` `{…} `` / `,` / `,@` ML sugar.
        assert_eq!(
            print(&sexpr::read("(quasiquote (+ a b))").unwrap(), 80),
            "`{ a + b }"
        );
        assert_eq!(
            print(&sexpr::read("(quasiquote (unquote x))").unwrap(), 80),
            "`{ ,x }"
        );
        assert_eq!(
            print(
                &sexpr::read("(quasiquote (f (unquote-splicing xs)))").unwrap(),
                80
            ),
            "`{ f(,@xs) }"
        );
    }

    #[test]
    fn small_forms_inline() {
        assert_eq!(assert_roundtrip("1 + 2 * 3", 80), "1 + 2 * 3");
        assert_eq!(assert_roundtrip("f(a, b, c)", 80), "f(a, b, c)");
        assert_eq!(assert_roundtrip("a.b.c", 80), "a.b.c");
        assert_eq!(
            assert_roundtrip("if a then b else c", 80),
            "if a then b else c"
        );
        // `let … in` always breaks the body to its own line, FLAT at the let column (ML idiom;
        // operator seq-86 — the final body is not indented below the `let`).
        assert_eq!(assert_roundtrip("let x = 1 in x", 80), "let x = 1 in\nx");
        assert_eq!(
            assert_roundtrip("fn(x, y) => x + y", 80),
            "fn(x, y) => x + y"
        );
    }

    #[test]
    fn prefix_unary_minus_round_trips() {
        // `-<expr>` (prefix negation applied to a NAME / call / member / paren) parses to the arity-1
        // subtraction `(- e)` and prints back to `-e`, tight over its operand. A `-<digit>` is a signed
        // LITERAL (a separate leaf), unaffected. The independent s-expr reader is the oracle for shape.
        assert_eq!(assert_roundtrip("-x", 80), "-x");
        assert_eq!(assert_roundtrip("-f(x)", 80), "-f(x)");
        assert_eq!(assert_roundtrip("-x.field", 80), "-x.field");
        // Negation binds TIGHTER than binary `+`: `-x + 1` is `(+ (- x) 1)`, printed back the same.
        assert_eq!(assert_roundtrip("-x + 1", 80), "-x + 1");
        // A parenthesized operand keeps its parens (the whole sum is negated): `(- (+ x 1))`.
        assert_eq!(assert_roundtrip("-(x + 1)", 80), "-(x + 1)");
        // A negative literal stays a literal (no `(- …)` wrapper), and `3 * -2` is binary `*` of two
        // literals — round-trips unchanged.
        assert_eq!(assert_roundtrip("-1", 80), "-1");
        assert_eq!(assert_roundtrip("3 * -2", 80), "3 * -2");
        // The s-expr reader is the oracle: `(- x)` (arity-1) prints as the ML prefix `-x`.
        let a = sexpr::read("(- x)").unwrap();
        assert_eq!(print(&a, 80), "-x");
        // Arity-2 `(- a b)` stays binary subtraction.
        let b = sexpr::read("(- a b)").unwrap();
        assert_eq!(print(&b, 80), "a - b");
    }

    #[test]
    fn pipeline_operator_round_trips() {
        // `|>` prints and re-parses like any infix operator; the minimal-paren split keeps a bare
        // chain bare and drops the parens `(+ total tax)` needs only when precedence demands them.
        assert_eq!(assert_roundtrip("x |> f", 80), "x |> f");
        assert_eq!(assert_roundtrip("x |> f(a)", 80), "x |> f(a)");
        assert_eq!(assert_roundtrip("x |> f |> g", 80), "x |> f |> g");
        // Looser than `+`, so the left sum needs no parens; the pipe as a whole is the value.
        assert_eq!(
            assert_roundtrip("total + tax |> round", 80),
            "total + tax |> round"
        );
        // The independent s-expr reader is the oracle: `(|> x f)` prints as the ML pipeline.
        let a = sexpr::read("(|> x f)").unwrap();
        assert_eq!(print(&a, 80), "x |> f");
    }

    #[test]
    fn symbol_sugar_round_trips() {
        // An identifier-content symbol prints with the unquoted `#name` sugar and re-reads to the same
        // `Leaf::Sym`. The two surfaces (ML `#meter` and the sugar it prints) agree with the s-expr
        // oracle's `#"meter"`.
        assert_eq!(assert_roundtrip("#meter", 80), "#meter");
        assert_eq!(assert_roundtrip("#map-insert", 80), "#map-insert");
        // Both spellings read to the same value, so the quoted input canonicalizes to the sugar.
        assert_eq!(assert_roundtrip("#\"meter\"", 80), "#meter");
        // Non-identifier content keeps the explicit `#"…"` form (a space, an empty symbol, a leading
        // digit, an operator glyph, a `.`): the sugar would not re-lex to the same symbol.
        assert_eq!(assert_roundtrip("#\"foo bar\"", 80), "#\"foo bar\"");
        assert_eq!(assert_roundtrip("#\"\"", 80), "#\"\"");
        assert_eq!(assert_roundtrip("#\"1st\"", 80), "#\"1st\"");
        assert_eq!(assert_roundtrip("#\"a.b\"", 80), "#\"a.b\"");
        // The ML `#name` spelling agrees with the s-expr oracle reading `#"name"`.
        assert_eq!(
            print(&sexpr::read(r#"(= #"map-insert" x)"#).unwrap(), 80),
            "#map-insert == x"
        );
    }

    #[test]
    fn member_call_with_a_symbol_operand_round_trips() {
        // The surface `Record.with r #field v` relies on (already has): a member-access application
        // whose args include a `#symbol` field-selector. Pin that it round-trips on BOTH surfaces so the
        // feature's SURFACE half stays supported independently of the rcdzc `record-with` special-form
        // arm (the crux the impl issue named "novel #symbol-operand parse/print" already works — this
        // guards against a regression). ML `#field` prints as the `#name` sugar; the s-expr oracle uses
        // the canonical `#"field"`.
        assert_eq!(
            assert_roundtrip("Record.with(rec, #price, 9)", 80),
            "Record.with(rec, #price, 9)"
        );
        // The canonical s-expr form (member-access head + `#"price"` symbol operand) prints to the ML
        // surface with the `#price` sugar — the two surfaces agree on the shape rcdzc receives.
        assert_eq!(
            print(
                &sexpr::read(r#"((. Record with) rec #"price" 9)"#).unwrap(),
                80
            ),
            "Record.with(rec, #price, 9)"
        );
        // A non-identifier field name keeps the explicit `#"…"` operand (the sugar would not re-lex).
        assert_eq!(
            assert_roundtrip(r#"Record.with(rec, #"has space", 9)"#, 80),
            "Record.with(rec, #\"has space\", 9)"
        );
    }

    #[test]
    fn a_hash_prefixed_name_is_backtick_escaped_to_disambiguate_from_a_symbol() {
        // A `Leaf::Name` whose text STARTS WITH `#` (e.g. the s-expr `#price` NAME, as opposed to the
        // `#"price"` SYMBOL) must print backtick-escaped as `` `#price` `` in ML — a bare `#price` there
        // lexes as a SYMBOL (`Leaf::Sym`), a DIFFERENT node, so the backtick is load-bearing for the
        // round-trip. This edge sits right next to the `Record.with(r, #field, v)` symbol-operand surface
        // (a `#`-headed name vs a `#`-symbol is exactly the ambiguity the record-update feature's `#field`
        // operand leans on), yet no test pinned the NAME side — pin it so a printer change can't drop the
        // escape and silently reinterpret a `#`-name as a symbol.
        // The s-expr oracle's `#price` is a Name leaf; ML must backtick it.
        assert_eq!(
            print(&sexpr::read("(def (f) #price)").unwrap(), 80),
            "def f() = `#price`"
        );
        // And the ML backtick form round-trips: `` `#price` `` -> Name `#price` -> `` `#price` ``.
        assert_eq!(assert_roundtrip("`#price`", 80), "`#price`");
        // Contrast (the SYMBOL side, already covered by symbol_sugar_round_trips): a bare ML `#price`
        // reads to a `Leaf::Sym` and prints back as the bare `#price` sugar — the two `#price` surfaces
        // are DISTINCT nodes, and each round-trips to its own spelling.
        assert_eq!(assert_roundtrip("#price", 80), "#price");
    }

    #[test]
    fn quantity_literal_round_trips() {
        // A quantity literal `(Qty.of <num> (Unit.of #"name"))` renders as the concise `<num> name`
        // surface and re-parses to the same arena — the inverse of `maybe_quantity_literal`.
        assert_eq!(assert_roundtrip("5 feet", 80), "5 feet");
        assert_eq!(assert_roundtrip("5.0 meter", 80), "5.0 meter");
        // It binds tighter than any operator, so `5 feet / 1 second` is a RATE — the division of two
        // quantity literals, not `5 (feet / 1) second`.
        assert_eq!(
            assert_roundtrip("5 feet / 1 second", 80),
            "5 feet / 1 second"
        );
        assert_eq!(
            assert_roundtrip("3 meter + 2 meter", 80),
            "3 meter + 2 meter"
        );
        assert_eq!(assert_roundtrip("dist(5 feet)", 80), "dist(5 feet)");
        // COMPOUND / RATE units (BUG #51): a glued `/`/`*`/`^` on the unit builds a composite (bare
        // `/`/`*`/`^` between `Unit.of` operands). The canonical printer renders the composite VERBOSE
        // (`Qty.of(…, Unit.of(#a) / Unit.of(#b))`, since the concise `<num> a/b` sugar is DISPLAY-only),
        // but that form is idempotent + structurally round-trips — which `assert_roundtrip` pins. (Pairs
        // with the parser's `compound_unit_desugars_on_glued_operators`; here we pin the print side.)
        assert_eq!(
            assert_roundtrip("59 GiB/s", 80),
            "Qty.of(59, Unit.of(#GiB) / Unit.of(#s))"
        );
        assert_eq!(
            assert_roundtrip("9 m/s^2", 80),
            "Qty.of(9, Unit.of(#m) / (Unit.of(#s) ^ 2))"
        );
        assert_eq!(
            assert_roundtrip("3 kg*m/s^2", 80),
            "Qty.of(3, Unit.of(#kg) * Unit.of(#m) / (Unit.of(#s) ^ 2))"
        );
        // The independent s-expr reader is the oracle: the canonical `(Qty.of …)` form prints concise.
        let a = sexpr::read(r#"(Qty.of 5 (Unit.of #"meter"))"#).unwrap();
        assert_eq!(print(&a, 80), "5 meter");
        // Shapes the concise surface can't express fall back to the round-tripping call form: a
        // computed unit, a non-bare-safe unit name, and `Unit.of` used outside a `Qty.of`.
        let computed =
            sexpr::read(r#"(Qty.of 5.0 (Unit./ (Unit.of #"meter") (Unit.of #"second")))"#).unwrap();
        // Identifier-content symbols print with the unquoted `#name` sugar.
        assert_eq!(
            print(&computed, 80),
            "Qty.of(5.0, Unit.of(#meter) / Unit.of(#second))"
        );
        // A non-identifier symbol (a space) keeps the explicit `#"…"` form.
        let odd = sexpr::read(r#"(Qty.of 5 (Unit.of #"foo bar"))"#).unwrap();
        assert_eq!(print(&odd, 80), "Qty.of(5, Unit.of(#\"foo bar\"))");
        let bare = sexpr::read(r#"(Unit.of #"meter")"#).unwrap();
        assert_eq!(print(&bare, 80), "Unit.of(#meter)");
    }

    #[test]
    fn quantity_over_a_tight_non_literal_operand_prints_concise() {
        // The parser accepts unit application as a general POSTFIX on any tight expression
        // (`x meter`, `f(x) meter`, `x.y meter` — not just a literal `5 meter`). The printer now mirrors
        // that: a `(Qty.of <tight> (Unit.of #name))` renders concise `<tight> name` for a name / call /
        // member-chain operand, re-reading to the same node. Before, only a numeric-literal operand
        // rendered concise and every other operand fell back to the verbose `Qty.of(x, Unit.of(#meter))`.
        assert_eq!(assert_roundtrip("x meter", 80), "x meter");
        assert_eq!(assert_roundtrip("f(x) meter", 80), "f(x) meter");
        assert_eq!(assert_roundtrip("f(g(x)) meter", 80), "f(g(x)) meter");
        assert_eq!(assert_roundtrip("x.y meter", 80), "x.y meter");
        // The independent s-expr oracle: a var/call operand prints concise.
        assert_eq!(
            print(
                &sexpr::read(r#"(Qty.of x (Unit.of #"meter"))"#).unwrap(),
                80
            ),
            "x meter"
        );
        assert_eq!(
            print(
                &sexpr::read(r#"(Qty.of (f x) (Unit.of #"meter"))"#).unwrap(),
                80
            ),
            "f(x) meter"
        );
        // NON-tight operands stay in the explicit `Qty.of(…)` call form — the concise `<op> name` surface
        // would MISBIND (`a + b meter` binds the unit to `b`; `if … meter` to the else-branch; a nested
        // `Qty.of` would double-suffix). Each still round-trips via the call form.
        assert_eq!(
            print(
                &sexpr::read(r#"(Qty.of (+ a b) (Unit.of #"meter"))"#).unwrap(),
                80
            ),
            "Qty.of(a + b, Unit.of(#meter))"
        );
        assert_eq!(
            print(
                &sexpr::read(r#"(Qty.of (if a b c) (Unit.of #"meter"))"#).unwrap(),
                80
            ),
            "Qty.of(if a then b else c, Unit.of(#meter))"
        );
        // A negative literal is not the `<digit>… name` surface either — stays explicit.
        assert_eq!(
            print(
                &sexpr::read(r#"(Qty.of -3 (Unit.of #"meter"))"#).unwrap(),
                80
            ),
            "Qty.of(-3, Unit.of(#meter))"
        );
    }

    /// The DISPLAY surface renders a VALUE for a human — dropping the round-trip ceremony the canonical
    /// printer must keep. Each case pairs the compiler's canonical VALUE FORM (as `cdz-run` emits it,
    /// read by the s-expr oracle) with its expected display text.
    #[test]
    fn display_surface_renders_values_readably() {
        let disp = |src: &str| {
            let a = sexpr::read(src).unwrap();
            print_display(&a, 80)
        };
        // A rational value is the native `(RationalTag num den)` node (seq-204). DISPLAY renders the
        // mathematical `num/den` (bare), dropping an integral `/1`. The sexpr surface spells the literal
        // `num/den` (slash) and reads it straight back to the node — safe because sexpr division is the
        // prefix `(/ a b)`.
        assert_eq!(disp("(: 1/3 Rational)"), "1/3");
        assert_eq!(print(&sexpr::read("1/3").unwrap(), 80), "1/3");
        // An integral rational drops its `/1` denominator in display.
        assert_eq!(disp("(: 8/1 Rational)"), "8");
        // A negative rational keeps its sign on the numerator.
        assert_eq!(disp("(: -1/2 Rational)"), "-1/2");
        // The outer `(: value type)` result annotation is stripped in display; a scalar shows bare.
        assert_eq!(disp("(: 5.0 Float64)"), "5.0");
        // A quantity value renders in its concise `<value> <unit>` surface — a base unit bare, a
        // rational value bare — instead of `Qty.of(`1/4`, Unit.base(#meter) / Unit.base(#second))`.
        assert_eq!(
            disp(concat!(
                "(: (Qty.of 1/4 (Unit./ (Unit.base #\"meter\") (Unit.base #\"second\")))",
                "   (Qty Rational (Unit./ (Unit.base #\"meter\") (Unit.base #\"second\"))))"
            )),
            "1/4 meter/second"
        );
        assert_eq!(
            disp("(: (Qty.of 5.0 (Unit.base #\"meter\")) (Qty Float64 (Unit.base #\"meter\")))"),
            "5.0 meter"
        );
        // An exponentiated unit reads `meter^2`; its integer exponent is a plain literal.
        assert_eq!(
            disp(concat!(
                "(: (Qty.of 9.0 (Unit.^ (Unit.base #\"meter\") 2))",
                "   (Qty Float64 (Unit.^ (Unit.base #\"meter\") 2)))"
            )),
            "9.0 meter^2"
        );
        // A DIMENSIONLESS quantity (`Unit.one`) shows just its value — no unit.
        assert_eq!(
            disp("(: (Qty.of 3.0 Unit.one) (Qty Float64 Unit.one))"),
            "3.0"
        );
        // A rational inside a compound value is rendered bare too (the display mode reaches every leaf).
        assert_eq!(
            disp("(: (tuple 1/2 3/1) (Tuple Rational Rational))"),
            "(1/2, 3)"
        );
    }

    #[test]
    fn display_of_a_rational_value_preserves_the_numeric_value_over_generated_rationals() {
        // VALUE PRESERVATION of the display Rational transform, swept — the existing display test pins ~10
        // HAND cases (`1/3`, `8/1`, `-1/2`, …) for no-panic + exact text, but never that the bare form the
        // display emits has the SAME numeric value as the input over the whole space. A display printer
        // that rendered `6/4` as `1/2` (or dropped a sign) would pass the totality + hand tests yet be a
        // real value miscompile in the REPL/notebook. Here: for a generated rational `n/d`, `print_display`
        // of `(: n/d Rational)` must yield a bare text that PARSES BACK to a rational of value n/d — checked
        // by cross-multiplication (a·d' == a'·d), independent of whether display reduces or keeps the
        // spelling. The form is either `p/q` or a bare integer `p` (when the value is integral).
        //
        // Parse the display's bare rational text `-p/q` or `-p` into (num, den) BigInts, or None if it is
        // not that shape (which would itself be a display bug for a Rational value — asserted by the caller).
        fn parse_bare_rational(s: &str) -> Option<(num_bigint::BigInt, num_bigint::BigInt)> {
            use std::str::FromStr;
            let (num_str, den_str) = match s.split_once('/') {
                Some((n, d)) => (n, d),
                None => (s, "1"), // an integral value displays as a bare integer
            };
            let num = num_bigint::BigInt::from_str(num_str.trim()).ok()?;
            let den = num_bigint::BigInt::from_str(den_str.trim()).ok()?;
            Some((num, den))
        }
        let mut rng = SplitMix64(0x5a71_0a11_0d15_9107);
        for _ in 0..4000 {
            // A random CANONICAL rational n/d — the sign lives on the NUMERATOR and the denominator is
            // POSITIVE (the value form `cdz-run` emits; a `n/-d` spelling is not a well-formed Rational
            // leaf — it backtick-escapes — so it is not a value the display path ever receives). Mixed
            // numerator signs, incl. integral (d|n) and both reduced + reducible spellings.
            let n = (rng.next() % 201) as i64 - 100; // -100..=100 (sign on the numerator)
            let d = (rng.next() % 100) as i64 + 1; // 1..=100, always positive (never zero)
            let src = format!("(: {n}/{d} Rational)");
            let Ok(arena) = sexpr::read(&src) else {
                continue;
            };
            let shown = print_display(&arena, 80);
            let (pn, pd) = parse_bare_rational(&shown).unwrap_or_else(|| {
                panic!(
                    "display of a Rational must be a bare `p/q` or integer, got {shown:?} for {src}"
                )
            });
            assert!(
                pd != num_bigint::BigInt::from(0),
                "display produced a zero denominator: {shown:?} for {src}"
            );
            // Value equality by cross-multiplication: n/d == pn/pd  ⟺  n·pd == pn·d.
            let (n_big, d_big) = (num_bigint::BigInt::from(n), num_bigint::BigInt::from(d));
            assert_eq!(
                &n_big * &pd,
                &pn * &d_big,
                "display of {src} = {shown:?} changed the numeric value (n/d != shown)"
            );
        }
    }

    #[test]
    fn unit_conversion_as_round_trips() {
        // `value as name` renders `(Unit.in (Unit.of #"name") value)` and re-parses to the same arena
        // — the inverse of the parser's `as_conversion`.
        assert_eq!(assert_roundtrip("q as meter", 80), "q as meter");
        assert_eq!(
            assert_roundtrip("2.0 kilometer as meter", 80),
            "2.0 kilometer as meter"
        );
        // It binds below arithmetic, so the whole quotient converts: `a / b as u` is `(a / b) as u`.
        assert_eq!(
            assert_roundtrip("240.0 meter / 8.0 second as meter", 80),
            "240.0 meter / 8.0 second as meter"
        );
        // …and threads into a pipeline as one converted value.
        assert_eq!(assert_roundtrip("q as meter |> f", 80), "q as meter |> f");
        // Left-associative: a chained conversion groups left and needs no parens.
        assert_eq!(
            assert_roundtrip("q as meter as foot", 80),
            "q as meter as foot"
        );
        // As a call argument it needs no parens (an argument parses at the loosest precedence), but as
        // the OPERAND of a tighter context (member access) it does — member/app binds tighter than `as`.
        assert_eq!(assert_roundtrip("f(q as meter)", 80), "f(q as meter)");
        assert_eq!(
            assert_roundtrip("(q as meter).value", 80),
            "(q as meter).value"
        );
        // The s-expr oracle: the canonical `(Unit.in (Unit.of …) …)` prints as the concise `as` surface.
        let a = sexpr::read(r#"(Unit.in (Unit.of #"meter") (Qty.of 2.0 (Unit.of #"kilometer")))"#)
            .unwrap();
        assert_eq!(print(&a, 80), "2.0 kilometer as meter");
        // A COMPOUND target has no bare-name surface, so it falls back to the `Unit.in(target, value)`
        // call form — a faithful round-trip either way.
        let compound =
            sexpr::read(r#"(Unit.in (Unit./ (Unit.of #"meter") (Unit.of #"hour")) q)"#).unwrap();
        assert_eq!(
            print(&compound, 80),
            "Unit.in(Unit.of(#meter) / Unit.of(#hour), q)"
        );
        // A non-bare-safe target name likewise keeps the call form.
        let odd = sexpr::read(r#"(Unit.in (Unit.of #"foo bar") q)"#).unwrap();
        assert_eq!(print(&odd, 80), "Unit.in(Unit.of(#\"foo bar\"), q)");
    }

    #[test]
    fn positional_member_access() {
        // `(. obj N)` (positional tuple access) renders `obj.N`, the numeric sibling of `obj.field`.
        assert_eq!(print(&sexpr::read("(. p 0)").unwrap(), 80), "p.0");
        assert_eq!(
            print(&sexpr::read("(. (tuple 1 2 3) 0)").unwrap(), 80),
            "(1, 2, 3).0"
        );
        // `.0` re-reads to the same `(. p 0)` head form.
        assert_eq!(assert_roundtrip("p.0", 80), "p.0");
        assert_eq!(assert_roundtrip("p.field", 80), "p.field");
        // a name key with a numeric-looking segment stays a name (only a real Int key is an index).
        let a = parser::read_ml("p.0");
        assert_eq!(sexpr::print(&a.arenas), "(. p 0)");
    }

    #[test]
    fn positional_member_operand_parenthesized_when_digit_adjacent() {
        // A numeric key must not abut an operand ending in a decimal digit, or it re-lexes as a float.
        // `(. 5 0)` → `(5).0` (not `5.0`); a chained `(. (. x 0) 1)` → `(x.0).1` (not `x.0.1`).
        assert_eq!(assert_roundtrip("(5).0", 80), "(5).0");
        assert_eq!(assert_roundtrip("(x.0).1", 80), "(x.0).1");
        assert_eq!(print(&sexpr::read("(. 5 0)").unwrap(), 80), "(5).0");
        assert_eq!(print(&sexpr::read("(. (. x 0) 1)").unwrap(), 80), "(x.0).1");
        // both re-read to the canonical head form.
        assert_eq!(sexpr::print(&parser::read_ml("(5).0").arenas), "(. 5 0)");
        assert_eq!(
            sexpr::print(&parser::read_ml("(x.0).1").arenas),
            "(. (. x 0) 1)"
        );
        // a NAME operand or a `)`-terminated operand needs no parens.
        assert_eq!(print(&sexpr::read("(. x 0)").unwrap(), 80), "x.0");
        assert_eq!(print(&sexpr::read("(. (f a) 0)").unwrap(), 80), "f(a).0");
    }

    #[test]
    fn annotation_sigil_round_trips() {
        // `@name form` is the general-purpose annotation sigil, canonically `(@ name form)` — the same
        // sigil↔head pattern as `.` member-access and `,@` unquote-splicing. The annotation prints on its
        // OWN line, ABOVE the form it modifies (the Rust `#[attr]\nfn …` convention), and re-reads to the
        // same head form. `inline-never`/`inline-always` are the two names the compiler consumes today; the
        // surface is name-agnostic.
        assert_eq!(
            assert_roundtrip("@inline-never def big(x) = x * 7", 80),
            "@inline-never\ndef big(x) = x * 7"
        );
        // ML surface desugars to the `@`-headed canonical form; the s-expr surface reads it directly. The
        // parser reads `@name` and the form below it across the line break (the form parses in prefix
        // position), so the on-its-own-line print round-trips.
        assert_eq!(
            sexpr::print(&parser::read_ml("@inline-never\ndef big(x) = x * 7").arenas),
            "(@ inline-never (def (big x) (* x 7)))"
        );
        assert_eq!(
            print(
                &sexpr::read("(@ inline-always (def (f x) (+ x 1)))").unwrap(),
                80
            ),
            "@inline-always\ndef f(x) = x + 1"
        );
        // A name-agnostic annotation (not an inline-policy name) round-trips just the same.
        assert_eq!(
            assert_roundtrip("@deprecated def old(x) = x", 80),
            "@deprecated\ndef old(x) = x"
        );
        // Stacked annotations each get their own line, since the annotated form parses in prefix position.
        assert_eq!(
            sexpr::print(&parser::read_ml("@a\n@b\ndef f(x) = x").arenas),
            "(@ a (@ b (def (f x) x)))"
        );
        assert_eq!(
            assert_roundtrip("@a @b def f(x) = x", 80),
            "@a\n@b\ndef f(x) = x"
        );
    }

    #[test]
    fn parameterized_annotation_round_trips() {
        // `@tag("slow")` — a CALL-STYLE annotation ARGUMENT (the operator's `@tag` surface): the `@`
        // name slot is an APPLICATION glued to the name, canonically `(@ (tag "slow") form)`. It prints
        // on its own line above the form, like a bare annotation, and re-reads to the same head form.
        assert_eq!(
            sexpr::print(&parser::read_ml("@tag(\"slow\")\ndef f() = 1").arenas),
            "(@ (tag \"slow\") (def (f) 1))"
        );
        assert_eq!(
            assert_roundtrip("@tag(\"slow\") def f() = 1", 80),
            "@tag(\"slow\")\ndef f() = 1"
        );
        // The canonical s-expr application-name shape prints back to the `@name(arg)` surface.
        assert_eq!(
            print(&sexpr::read("(@ (tag \"slow\") (def (f) 1))").unwrap(), 80),
            "@tag(\"slow\")\ndef f() = 1"
        );
        // A parameterized annotation STACKS with a bare one, each on its own line.
        assert_eq!(
            sexpr::print(&parser::read_ml("@test\n@tag(\"foo\")\ndef f() = 1").arenas),
            "(@ test (@ (tag \"foo\") (def (f) 1)))"
        );
        assert_eq!(
            assert_roundtrip("@test @tag(\"foo\") def f() = 1", 80),
            "@test\n@tag(\"foo\")\ndef f() = 1"
        );
        // Multiple args round-trip too (the name slot is a general application).
        assert_eq!(
            assert_roundtrip("@cfg(\"a\", \"b\") def f() = 1", 80),
            "@cfg(\"a\", \"b\")\ndef f() = 1"
        );
    }

    #[test]
    fn attr_renders_on_its_own_line_above_the_def_operator_16() {
        // OPERATOR #16 (ATTR-ABOVE): a `@test` / `@tag("…")` annotation on a def renders on its OWN LINE
        // ABOVE the def (the Rust `#[attr]\nfn …` convention), NEVER inline. This absorbs the invariant the
        // guide's `check-examples.mjs` pinned as a safety net — that check is being removed (operator: guide
        // off node checks), and ATTR-ABOVE is THIS ML printer's contract, so it belongs in the printer suite.
        // The general shape is also covered by `annotation_sigil_round_trips` /
        // `parameterized_annotation_round_trips`; this pins the guide's EXACT `@test`/`@tag` cases + the
        // never-inline anti-case under the operator-numbered name so the contract is unmistakable.
        // Bare `@test`.
        assert_eq!(
            assert_roundtrip("@test def f() = 1", 80),
            "@test\ndef f() = 1"
        );
        // Parameterized `@tag("slow")`.
        assert_eq!(
            assert_roundtrip("@tag(\"slow\") def f() = 1", 80),
            "@tag(\"slow\")\ndef f() = 1"
        );
        // Already-canonical (own-line) input is idempotent — the `@` line stays immediately above `def`.
        assert_eq!(
            assert_roundtrip("@test\ndef f() = 1", 80),
            "@test\ndef f() = 1"
        );
        // The never-inline anti-case, explicit: the printed form STARTS with `@test\n` and never renders
        // `@test def` on one line (the exact regression the guide's check guarded).
        let out = assert_roundtrip("@test def f() = 1", 80);
        assert!(
            out.starts_with("@test\n") && !out.contains("@test def"),
            "a @-annotation must render on its own line above the def (OPERATOR #16), got {out:?}"
        );
    }

    #[test]
    fn the_test_annotation_example_round_trips_both_directions_and_never_prints_the_backtick_at_form()
     {
        // REGRESSION for the operator's thrice-reported high-viz `@test` bug (concierge issue): a
        // `(@ test (def …))` annotation on the ML surface must PRINT as `@test` above the def and PARSE
        // that back to the same `(@ test …)` head — NOT the malformed `` `@`(test, <def>) `` backtick-quoted-
        // symbol CALL that was reported (which broke every `@test`/`@property` example in both directions).
        // Pin the operator's EXACT example (a `@test` def with an `assert-eq` body) so the specific case can
        // never regress; the general annotation surface is covered above, but this thrice-reported case
        // deserves its own witness. Both directions + a NEGATIVE assertion against the malformed form.
        let sx =
            r#"(@ test (def (two-plus-two-is-four) (assert-eq (+ 2 2) 4 "arithmetic is broken")))"#;
        // s-expr → ML: prints `@test` on its own line above the def, no backtick-`@`-call.
        let ml = print(&sexpr::read(sx).unwrap(), 80);
        assert_eq!(
            ml, "@test\ndef two-plus-two-is-four() = assert-eq(2 + 2, 4, \"arithmetic is broken\")",
            "the @test annotation must print above the def, not as a backtick-@ call"
        );
        // The regression signature: the malformed output was a backtick-quoted `@` applied as a call.
        assert!(
            !ml.contains("`@`"),
            "must not print the annotation as a backtick-quoted `@` symbol call: {ml}"
        );
        // ML → s-expr: `@test`-above-the-def parses back to the canonical `(@ test …)` head.
        let back = parser::read_ml(&ml);
        assert!(
            back.ok(),
            "the printed @test form must re-parse: {:?}",
            back.errors
        );
        assert_eq!(
            sexpr::print(&back.arenas),
            sx,
            "the ML @test form must re-read to the same (@ test …) head"
        );
        // The full s-expr↔ML round-trip is structurally faithful (what the guide/testing page relies on).
        assert!(
            sexpr::read(sx)
                .unwrap()
                .structurally_eq(&parser::read_ml(&ml).arenas),
            "the operator's @test example must round-trip s-expr ⇄ ML structurally"
        );
        // `@property` — the other broken example — behaves the same way.
        let prop = "(@ property (def (p x) (assert-eq x x \"reflexive\")))";
        let prop_ml = print(&sexpr::read(prop).unwrap(), 80);
        assert!(prop_ml.starts_with("@property\n"), "{prop_ml}");
        assert_eq!(sexpr::print(&parser::read_ml(&prop_ml).arenas), prop);
    }

    #[test]
    fn annotation_on_a_compound_expression_parenthesizes_so_it_round_trips() {
        // The parser re-reads an annotated form in PREFIX position, so an annotated INFIX / APPLICATION /
        // MEMBER-access form (the shapes the Pratt/postfix loops build AFTER the prefix atom) must be
        // PARENTHESIZED by the printer — else `@inline a + 1` re-reads as `(+ (@ inline a) 1)` (the `@`
        // binding only the leading atom), NOT the intended `(@ inline (+ a 1))`. Pin each post-prefix shape.
        for form_sx in [
            "(+ a 1)",           // infix
            "(f x)",             // application (call)
            "((g y) x)",         // computed-callee application
            "(. a b)",           // member access
            "(. (. a b) c)",     // member chain
            "(+ (* a b) (f x))", // nested infix + call
        ] {
            let sx = format!("(def (main) (@ inline {form_sx}))");
            let a = sexpr::read(&sx).unwrap();
            let ml = print(&a, 80);
            // The annotated form prints parenthesized (a `(` on the line below the `@inline`).
            let back = parser::read_ml(&ml);
            assert!(
                back.ok(),
                "annotated compound re-parses: {ml:?} errs={:?}",
                back.errors
            );
            assert!(
                back.arenas.structurally_eq(&a),
                "annotated compound round-trips (parens keep the `@` on the WHOLE form):\n  ml: {ml}\n  back: {}",
                sexpr::print(&back.arenas)
            );
        }
        // A SELF-DELIMITING annotated form (keyword form, bracketed literal, nested `@`) is NOT
        // parenthesized — it round-trips bare (and parens would be ugly/needless).
        for (form_sx, want_no_paren) in [
            ("(if a b c)", "@inline\nif a then b else c"),
            ("#list(1 2)", "@inline\n[1, 2]"),
            ("(def (f) 1)", "@inline\ndef f() = 1"),
        ] {
            let sx = format!("(def (main) (@ inline {form_sx}))");
            // For the def case the wrapper is a module-ish do; just assert the annotated-form render + round-trip.
            let a = sexpr::read(&format!("(@ inline {form_sx})")).unwrap();
            let ml = print(&a, 80);
            assert_eq!(
                ml, want_no_paren,
                "self-delimiting annotated form must not be parenthesized"
            );
            assert!(
                parser::read_ml(&ml).arenas.structurally_eq(&a),
                "self-delimiting annotated form round-trips: {ml}"
            );
            let _ = sx;
        }
        // A PARAMETERIZED annotation on a compound form round-trips too — the `@tag("t")` name-call takes
        // ONLY its glued arg, leaving the parenthesized form as the annotated target.
        let a = sexpr::read(r#"(def (main) (@ (tag "t") (+ a 1)))"#).unwrap();
        let ml = print(&a, 80);
        assert!(
            parser::read_ml(&ml).arenas.structurally_eq(&a),
            "parameterized annotation on a compound form round-trips:\n  ml: {ml}\n  back: {}",
            sexpr::print(&parser::read_ml(&ml).arenas)
        );
    }

    #[test]
    fn annotation_in_an_operand_position_parenthesizes_the_whole_annotation() {
        // An `@name` annotation prints on its OWN line above the form — safe only at a STATEMENT/body
        // position, where the next surface token is a fresh statement. In an OPERAND position (an infix/
        // ascription operand, a `match` scrutinee), a trailing operator would bind to the annotated form's
        // LAST line, not the whole `(@ …)`: `(: (@ test (if a b c)) T)` printed `@test\n if … c : T`,
        // which re-read as `(@ test (if a b (: c T)))` — the `: T` swallowed by the `if`'s else-branch, a
        // round-trip BREAK. In operand position the whole annotation is now parenthesized.
        use crate::sexpr;
        // Ascription of an annotation, as a match scrutinee (the reported break) — round-trips now.
        for sx in [
            r#"(def (main) (match (: (@ test (if a b c)) (-> Int64 Bool)) (3 x)))"#,
            r#"(def (main) (match (: (@ test (if a b c)) Int64) (3 x)))"#,
            // annotation as an infix operand.
            r#"(def (main) (+ (@ test (if a b c)) 1))"#,
            r#"(def (main) (+ (@ test x) 1))"#,
        ] {
            let a = sexpr::read(sx).unwrap();
            let ml = print(&a, 80);
            let back = parser::read_ml(&ml);
            assert!(
                back.ok() && back.arenas.structurally_eq(&a),
                "annotation in operand position round-trips:\n  sx: {sx}\n  ml: {ml}\n  back: {}",
                sexpr::print(&back.arenas)
            );
        }
        // A STATEMENT-position annotation must still print WITHOUT a wrapping paren (`@name` on its own
        // line above the form) — the parenthesization is operand-position ONLY.
        let stmt = print(&sexpr::read("(def (main) (@ inline (+ a 1)))").unwrap(), 80);
        assert!(
            stmt.contains("@inline\n") && !stmt.contains("(@inline"),
            "a statement-position annotation must not be parenthesized:\n{stmt}"
        );
    }

    #[test]
    fn at_bang_param_pragma_prints_the_module_directive_surface() {
        // `(pragma param (param <kv>…) (: name Type))` -> `@!param(k: v, …) name : Type` (the operator's
        // module-level `@param`). Print + re-read must be structurally faithful, and the surface must be the
        // `@!param` sugar, not a generic `pragma(param, …)` call.
        for (sx, want) in [
            (
                "(pragma param (param (: widget slider)) (: width Int64))",
                "@!param(widget: slider) width : Int64",
            ),
            (
                r#"(pragma param (param (: widget slider) (: range #tuple(1 10))) (: width Int64))"#,
                "@!param(widget: slider, range: (1, 10)) width : Int64",
            ),
            // empty config -> no `()`
            (
                "(pragma param (param) (: width Int64))",
                "@!param width : Int64",
            ),
            // function-typed param
            (
                "(pragma param (param (: widget stepper)) (: transform (-> Int64 Int64)))",
                "@!param(widget: stepper) transform : Int64 -> Int64",
            ),
        ] {
            let a = sexpr::read(sx).unwrap();
            let ml = print(&a, 80);
            assert_eq!(ml, want, "@!param surface");
            assert!(
                parser::read_ml(&ml).arenas.structurally_eq(&a),
                "@!param round-trips: {ml}"
            );
        }
        // A non-`param` pragma still prints the plain `@!key arg` form (unchanged).
        assert_eq!(
            print(
                &sexpr::read("(pragma default-fraction Rational)").unwrap(),
                80
            ),
            "@!default-fraction Rational"
        );
    }

    #[test]
    fn tagged_template_round_trips_hole_free() {
        // B1: a hole-free tagged template `tag"…"` reads to `(tagged-template <tag> (chunks <str>)
        // (holes))` and prints back to the glued `tag"…"` surface.
        assert_eq!(
            sexpr::print(&parser::read_ml("def m() = jsx\"hello world\"").arenas),
            "(def (m) (tagged-template jsx (chunks \"hello world\") (holes)))"
        );
        assert_eq!(
            assert_roundtrip("def m() = jsx\"hello world\"", 80),
            "def m() = jsx\"hello world\""
        );
        // The canonical s-expr node prints back to the sugar.
        assert_eq!(
            print(
                &sexpr::read("(def (m) (tagged-template id (chunks \"hi\") (holes)))").unwrap(),
                80
            ),
            "def m() = id\"hi\""
        );
        // Escapes in the body survive (escape_string ∘ unescape_string).
        assert_eq!(
            assert_roundtrip("def m() = id\"a\\nb\\\"c\"", 80),
            "def m() = id\"a\\nb\\\"c\""
        );
        // A tag glued to an EMPTY string is a valid (empty single chunk) template.
        assert_eq!(
            sexpr::print(&parser::read_ml("def m() = e\"\"").arenas),
            "(def (m) (tagged-template e (chunks \"\") (holes)))"
        );
    }

    #[test]
    fn tagged_template_non_bare_tag_falls_back_to_call_form() {
        // PR #405: the `tag"…"` sugar glues the tag directly before the quote, and the lexer only
        // re-lexes a BARE ident (not a backtick-escaped name) glued to `"`. A non-bare-safe tag (here
        // `a+b`, which `emit_name` would backtick-quote) must NOT sugar — `` `a+b`"…" `` would not
        // re-lex, a garbage render. It falls back to the generic `(tagged-template …)` call form, which
        // round-trips (the tag is backtick-escaped in call-head position and re-reads as a name).
        let a = sexpr::read("(tagged-template a+b (chunks \"hi\") (holes))").unwrap();
        let ml = print(&a, 80);
        assert!(
            !ml.starts_with('`') && ml.contains("tagged-template("),
            "a non-bare tag must print as the generic call form, not garbage sugar; got {ml:?}"
        );
        // And it round-trips: re-reading the ML yields the same tree.
        assert!(
            parser::read_ml(&ml).arenas.structurally_eq(&a),
            "the call-form fallback must round-trip; ml = {ml:?}"
        );
        // A BARE-safe tag still sugars to `tag"…"`.
        assert_eq!(
            print(
                &sexpr::read("(tagged-template jsx (chunks \"hi\") (holes))").unwrap(),
                80
            ),
            "jsx\"hi\""
        );
    }

    #[test]
    fn tagged_template_holes_round_trip() {
        // B2: `{expr}` interpolation holes. The body splits into chunks at hole boundaries — a body
        // with N holes has N+1 chunks (some empty) — and each hole is an ordinary parsed expression.
        assert_eq!(
            sexpr::print(&parser::read_ml("def m(x) = jsx\"a{x}b\"").arenas),
            "(def (m x) (tagged-template jsx (chunks \"a\" \"b\") (holes x)))"
        );
        assert_eq!(
            assert_roundtrip("def m(x) = jsx\"a{x}b\"", 80),
            "def m(x) = jsx\"a{x}b\""
        );
        // Leading/trailing empty chunks (a hole at each edge) → chunks ["", "+", ""].
        assert_eq!(
            sexpr::print(&parser::read_ml("def m(x, y) = t\"{x}+{y}\"").arenas),
            "(def (m x y) (tagged-template t (chunks \"\" \"+\" \"\") (holes x y)))"
        );
        assert_eq!(
            assert_roundtrip("def m(x, y) = t\"{x}+{y}\"", 80),
            "def m(x, y) = t\"{x}+{y}\""
        );
        // A hole holds ANY expression (parsed by the full ML reader).
        assert_eq!(
            sexpr::print(&parser::read_ml("def m(a, b) = t\"sum={a + b * 2}!\"").arenas),
            "(def (m a b) (tagged-template t (chunks \"sum=\" \"!\") (holes (+ a (* b 2)))))"
        );
        assert_eq!(
            assert_roundtrip("def m(a, b) = t\"sum={a + b * 2}!\"", 80),
            "def m(a, b) = t\"sum={a + b * 2}!\""
        );
        // `{{` / `}}` are LITERAL braces in a chunk, not holes — they round-trip (chunk holds `{`/`}`).
        assert_eq!(
            sexpr::print(&parser::read_ml("def m(x) = t\"lit {{brace}} {x} end\"").arenas),
            "(def (m x) (tagged-template t (chunks \"lit {brace} \" \" end\") (holes x)))"
        );
        assert_eq!(
            assert_roundtrip("def m(x) = t\"lit {{brace}} {x} end\"", 80),
            "def m(x) = t\"lit {{brace}} {x} end\""
        );
        // A hole may contain a STRING literal with braces — a raw `"` inside the hole opens/closes it, so
        // its braces don't miscount (`g("}")` is one hole holding the string `"}"`).
        assert_eq!(
            sexpr::print(&parser::read_ml("def m() = t\"x{g(\"}\")}y\"").arenas),
            "(def (m) (tagged-template t (chunks \"x\" \"y\") (holes (g \"}\"))))"
        );
        // A hole's string may contain an ESCAPED quote — `\"` must NOT toggle the hole's string-mode
        // (PR #409): `g("\"}")` is one hole holding the string `"}` (an escaped-quote char then a brace);
        // the `}` inside that string must not close the hole. (Source `t"x{g("\"}")}y"`.)
        assert_eq!(
            sexpr::print(&parser::read_ml("def m() = t\"x{g(\"\\\"}\")}y\"").arenas),
            "(def (m) (tagged-template t (chunks \"x\" \"y\") (holes (g \"\\\"}\"))))"
        );
        // A backslash-escaped brace `\{` / `\}` is an ALTERNATE spelling of a literal brace: it reads to
        // the SAME chunk as the `{{` / `}}` doubling (chunk holds `{`/`}`), and the printer CANONICALIZES
        // it to the doubled form. So `\{`/`\}` input is not byte-preserved — it normalizes to `{{`/`}}` —
        // but the normalization is a FIXED POINT (re-reading the output re-prints identically). Pin both:
        // the two spellings read to one arena, and the output is idempotent.
        assert_eq!(
            sexpr::print(&parser::read_ml("t\"a\\{b\\}c\"").arenas),
            sexpr::print(&parser::read_ml("t\"a{{b}}c\"").arenas),
            "backslash-escaped and doubled braces read to the same literal-brace chunk"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("t\"a\\{b\\}c\"").arenas),
            "(tagged-template t (chunks \"a{b}c\") (holes))"
        );
        // The escaped spelling canonicalizes to the doubled form, which is then a fixed point.
        assert_eq!(assert_roundtrip("t\"a\\{b\\}c\"", 80), "t\"a{{b}}c\"");
        assert_eq!(assert_roundtrip("t\"a{{b}}c\"", 80), "t\"a{{b}}c\"");
    }

    #[test]
    fn pragma_sugar_round_trips() {
        // `@!key arg` is the PRAGMA sugar — the inner-attribute twin of `@` (Rust's `#![…]`). It desugars
        // to `(pragma key arg)`, byte-identical to a written pragma, so it flows through the SAME registry
        // with no new downstream case. A bare-name argument (`Float32`) is the common shape.
        assert_eq!(
            assert_roundtrip("@!default-float Float32", 80),
            "@!default-float Float32"
        );
        // ML surface → the canonical `(pragma …)` head; the s-expr surface reads it directly, so a written
        // `(pragma …)` prints as `@!`. The two surfaces agree on one tree.
        assert_eq!(
            sexpr::print(&parser::read_ml("@!default-fraction Rational").arenas),
            "(pragma default-fraction Rational)"
        );
        assert_eq!(
            print(&sexpr::read("(pragma default-integer Int64)").unwrap(), 80),
            "@!default-integer Int64"
        );
        // The argument parses in prefix+POSTFIX position, so a constructor APPLICATION `Int(8)` is the
        // single argument (`(Int 8)`), not `(pragma … Int)` applied to `8` — and it round-trips.
        assert_eq!(
            assert_roundtrip("@!default-integer Int(8)", 80),
            "@!default-integer Int(8)"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("@!default-integer Int(8)").arenas),
            "(pragma default-integer (Int 8))"
        );
        // In a MODULE, the pragma sits above the members it governs and the following `def` is NOT
        // swallowed as an argument (a member does not begin with `.`/`(`); the whole module round-trips.
        assert_eq!(
            sexpr::print(
                &parser::read_ml("module m {\n  @!default-float Float32\n  def x() = 0.5\n}")
                    .arenas
            ),
            "(module m (pragma default-float Float32) (def (x) 0.5))"
        );
    }

    #[test]
    fn function_definition() {
        // a named def uses `def`; an anonymous lambda uses `fn` — distinct surfaces.
        assert_eq!(
            assert_roundtrip("def add(a, b) = a + b", 80),
            "def add(a, b) = a + b"
        );
        assert_eq!(assert_roundtrip("def main() = 42", 80), "def main() = 42");
        assert_eq!(assert_roundtrip("fn(x) => x * 2", 80), "fn(x) => x * 2");
    }

    #[test]
    fn an_annotated_def_juxtaposes_without_a_spurious_semicolon() {
        // A top-level `@name def …` FOLLOWED by another form must NOT get a trailing `;`: an annotation is
        // a self-delimiting form boundary (an `@` only ever begins a fresh annotation), so the next form
        // juxtaposes, exactly as after a bare `def`. Before, the root-form printer treated the following
        // `@`-form as "open" and appended a `;` (`@test def a() = unit;`), which then FAILED to re-parse
        // ("a do block must end in a value form"). `assert_roundtrip` re-parses the printed text, so it
        // would panic on that breakage — this pins the fix. Two annotated defs in a row is the exact case.
        let printed = assert_roundtrip("@test def a() = unit\n@test def b() = unit", 80);
        assert!(
            !printed.contains(';'),
            "an annotated def must not gain a trailing `;`: {printed:?}"
        );
        // The inline-policy annotation (the original `@` user) juxtaposes the same way.
        assert!(
            !assert_roundtrip("@inline-never def h() = 1\ndef m() = h()", 80).contains("1;"),
            "an @inline-never def must not gain a trailing `;`"
        );
    }

    #[test]
    fn value_definition() {
        // `def name = value` is a value definition (hoisting, so `def` not `let`).
        assert_eq!(assert_roundtrip("def x = 5", 80), "def x = 5");
        assert_eq!(
            assert_roundtrip("def pt = { x = 1, y = 2 }", 80),
            "def pt = { x = 1, y = 2 }"
        );
        // the underlying AST is `(def name value)` — a name atom, not a signature list.
        let a = sexpr::read("(def x 5)").unwrap();
        assert_eq!(print(&a, 80), "def x = 5");
    }

    #[test]
    fn sum_type_declaration() {
        // Each variant on its own line, led by `| ` (including the first) — symmetric with a match's
        // `|`-arms. Nullary = a bare ctor, a payload variant = `Ctor(T, …)`. The `|` is a surface
        // separator, never a tree atom.
        assert_eq!(
            assert_roundtrip("type N = | I(Int64) | J(Int64)", 80),
            "type N =\n  | I(Int64)\n  | J(Int64)"
        );
        assert_eq!(
            assert_roundtrip("type Sign = | Neg | Zero | Pos", 80),
            "type Sign =\n  | Neg\n  | Zero\n  | Pos"
        );
        // nullary + payload mix.
        assert_eq!(
            assert_roundtrip("type FL = | FNil | FCons(Tuple(Int64, FL))", 80),
            "type FL =\n  | FNil\n  | FCons(Tuple(Int64, FL))"
        );
        // the arena is `(type NAME variant…)` — nullary = a bare name, payload = a `(Ctor T…)` list.
        let a = sexpr::read("(type N (I Int64) (J Int64))").unwrap();
        assert_eq!(print(&a, 80), "type N =\n  | I(Int64)\n  | J(Int64)");
        let a = sexpr::read("(type FL FNil (FCons (Tuple Int64 FL)))").unwrap();
        assert_eq!(
            print(&a, 80),
            "type FL =\n  | FNil\n  | FCons(Tuple(Int64, FL))"
        );
    }

    #[test]
    fn a_multiline_type_doc_header_prints_every_line_flush_not_indented() {
        // A `///` doc header on a `type` decl must print EVERY line at the type's own column — line 1 AND
        // its continuation lines. Regression: `print_type` opened `cbox(INDENT)` BEFORE the docs, so the
        // first `hardbreak` reflowed line 2+ to the box's INDENT while line 1 stayed at column 0 — an
        // inconsistent per-line indent within one doc block (v-cdz-tooling flagged: multi-line `type` doc
        // headers went `///`\n`  ///`). Docs now print outside the variant-indent box, like `print_def`.
        let a = sexpr::read(r#"(type T (doc "line one") (doc "line two") (doc "line three") A B)"#)
            .unwrap();
        assert_eq!(
            print(&a, 80),
            "/// line one\n/// line two\n/// line three\ntype T =\n  | A\n  | B",
            "every doc-header line flush at column 0; variants indented under `type`"
        );
        // Round-trips + idempotent (the `///` header re-reads to the same `(doc …)` nodes).
        assert_eq!(
            assert_roundtrip("/// one\n/// two\ntype T = | A | B", 80),
            "/// one\n/// two\ntype T =\n  | A\n  | B"
        );
    }

    #[test]
    fn a_multiline_effect_doc_header_prints_every_line_flush_not_indented() {
        // Same layout invariant as the `type` doc header, for an `effect` decl: a multi-line `///` header
        // must print EVERY line at the effect's own column. Regression sibling to `print_type` — before
        // the fix `print_effect` also opened `cbox(INDENT)` BEFORE the docs, so line 2+ reflowed to the
        // box's INDENT (`///`\n`  ///`) while line 1 stayed flush. Docs now print outside the `|`-led
        // operations' indent box, like `print_type`/`print_def`.
        let a = sexpr::read(
            r#"(effect E (doc "line one") (doc "line two") (doc "line three") (op emit (-> Unit)))"#,
        )
        .unwrap();
        assert_eq!(
            print(&a, 80),
            "/// line one\n/// line two\n/// line three\neffect E =\n  | emit : -> Unit",
            "every doc-header line flush at column 0; operations indented under `effect`"
        );
        // Round-trips + idempotent (the `///` header re-reads to the same `(doc …)` nodes).
        assert_eq!(
            assert_roundtrip("/// one\n/// two\neffect E = | emit : -> Unit", 80),
            "/// one\n/// two\neffect E =\n  | emit : -> Unit"
        );
    }

    #[test]
    fn a_malformed_empty_op_effect_clause_degrades_to_generic_form_without_panic() {
        // REGRESSION (reported by v-wasmtime-migration, hit delanguaging a CDZ0201 reject case): a
        // malformed `(effect E (op))` — a bare `(op)` with ZERO children (no name/type) — must NOT panic
        // the ML printer. `is_effect_shape` indexed `o[0]` BEFORE its `o.len()` gate, so the empty op's
        // `o == []` panicked index-out-of-bounds on print. With the length gate reordered to short-circuit
        // first, the malformed op fails the shape check and the whole effect degrades to the generic call
        // form — the printer-totality guarantee (`print` never panics on a well-formed arena, whatever the
        // surface shape).
        let a = sexpr::read("(effect E (op))").expect("reads the malformed effect node");
        let printed = print(&a, 80);
        assert!(
            !printed.is_empty(),
            "degrades to a non-empty generic form: {printed:?}"
        );
        // NOT the `effect Name = | …` surface (the path that panicked) — the generic call form instead.
        assert!(
            !printed.contains("effect E ="),
            "a malformed op does NOT use the |-led effect surface: {printed:?}"
        );
        // Totality: the printed generic form re-reads to a valid arena (round-trip stays total).
        let _ = parser::read_ml(&printed);
    }

    #[test]
    fn a_leading_module_doc_prints_above_the_keyword_and_round_trips() {
        // A `///` header ABOVE `module M {` documents the MODULE — the reader attaches it as a LEADING
        // `(doc …)` MEMBER: `(module M (doc …) (def …))`. This is a DISTINCT tree from a doc INSIDE the
        // braces, which attaches to the def it precedes: `(module M (def (x) (doc …) …))`. Regression:
        // `print_module` rendered the leading module-doc member as an in-body `///` line, so it re-read
        // as a doc on the FIRST member — silently MIGRATING the module-doc onto that member (a round-trip
        // break in my core invariant). The leading doc run now prints ABOVE the `module` keyword.
        // A leading module-doc member prints above `module`, and re-reads to the SAME module-doc member
        // (NOT migrated into the def) — the round-trip witness.
        let a = sexpr::read(r#"(module M (doc "one") (doc "two") (def (y) (: 1 Int64)))"#).unwrap();
        let printed = print(&a, 80);
        assert_eq!(
            printed, "/// one\n/// two\nmodule M {\n  def y() -> Int64 = 1\n}",
            "leading module-docs print flush above the `module` keyword"
        );
        let b = parser::read_ml(&printed);
        assert!(b.ok(), "reparse: {:?}", b.errors);
        assert_eq!(
            sexpr::print(&b.arenas),
            "(module M (doc \"one\") (doc \"two\") (def (y) (: 1 Int64)))",
            "docs stay MODULE members across the round-trip, not migrated onto the def"
        );
        // A doc INSIDE the braces (before a def) is the DISTINCT tree and must be UNCHANGED — it stays a
        // doc on that def, printed indented in the body (the contrast that makes the fix correct, not a
        // blanket hoist).
        let body = parser::read_ml("module M {\n  /// on def\n  def y() -> Int64 = 1\n}");
        assert_eq!(
            sexpr::print(&body.arenas),
            "(module M (def (y) (doc \"on def\") (: 1 Int64)))",
            "an in-body doc attaches to the def, not the module"
        );
        // A doc-only module (no members) also round-trips: the leading doc prints above the empty braces.
        assert_eq!(
            sexpr::print(
                &parser::read_ml(&print(
                    &sexpr::read(r#"(module M (doc "only"))"#).unwrap(),
                    80
                ))
                .arenas
            ),
            "(module M (doc \"only\"))"
        );
    }

    #[test]
    fn a_trailing_doc_or_comment_before_a_modules_close_brace_is_preserved_not_dropped() {
        // A `///` doc or `//` comment on the last line(s) INSIDE a module body, before the closing `}`,
        // has no following member — it sits in the `}` token's leading slot, which the member loop never
        // drains, so it used to be DROPPED (a genuine content LOSS: `cdz fmt` then REFUSED the whole file
        // via the comment-drop guard, so the module could not be formatted). module_expr now drains that
        // slot after the loop (mirroring the top-level `program()` trailing handler).
        //
        // A trailing `///` doc → a `(doc …)` MODULE MEMBER at the end of the body (docs are members here).
        assert_eq!(
            sexpr::print(
                &parser::read_ml("module M {\n  def a() -> Int64 = 1\n  /// trailing doc\n}")
                    .arenas
            ),
            "(module M (def (a) (: 1 Int64)) (doc \"trailing doc\"))",
            "a trailing `///` before the close brace becomes a doc member, not dropped"
        );
        // It round-trips (re-reading the printed form yields the same trailing doc member).
        let printed = print(
            &parser::read_ml("module M {\n  def a() -> Int64 = 1\n  /// trailing doc\n}").arenas,
            80,
        );
        assert_eq!(
            sexpr::print(&parser::read_ml(&printed).arenas),
            "(module M (def (a) (: 1 Int64)) (doc \"trailing doc\"))",
            "trailing module doc round-trips"
        );
        // A trailing `//` comment → wraps the LAST member as `(comment …)`, preserved (not dropped).
        assert_eq!(
            sexpr::print(
                &parser::read_ml("module M {\n  def a() -> Int64 = 1\n  // trailing comment\n}")
                    .arenas
            ),
            "(module M (comment \"trailing comment\" (def (a) (: 1 Int64))))",
            "a trailing `//` before the close brace wraps the last member, not dropped"
        );
        // A trailing doc in an EMPTY module body is also preserved (a `(doc …)` stands alone).
        assert_eq!(
            sexpr::print(&parser::read_ml("module M {\n  /// only doc\n}").arenas),
            "(module M (doc \"only doc\"))",
            "a trailing doc in an empty module body is preserved"
        );
        // A `//` comment in an EMPTY body has no member to wrap and no clean standalone carrier — it must
        // NOT corrupt the module name (the earlier `items.pop()` bug wrapped the name → `(module (comment
        // … M))`). The module reads as `(module M)`; the comment stays in the slot for the drop-guard to
        // catch (`cdz fmt` refuses, byte-identical) — the pre-existing safe behavior, no corruption.
        assert_eq!(
            sexpr::print(&parser::read_ml("module M {\n  // only comment\n}").arenas),
            "(module M)",
            "a comment-only empty module keeps its name intact (no phantom member, no name corruption)"
        );
    }

    #[test]
    fn open_sum_row_variable_round_trips() {
        // OPEN SUM (open-sums OS1): a trailing `.. r` row-variable marker after the last variant. It is
        // the flat two-sibling convention — `Name("..")` then a lowercase `Name` — as the type list's
        // final two children, NOT a wrapper node (matches `db.rs::scan_type_decl` + the s-expr corpus).
        // It prints on its own line as `.. r` (NOT as spurious `| ..` / `| r` variants).
        assert_eq!(
            sexpr::print(
                &parser::read_ml("type T =\n  | Known(Int64)\n  | Unknown(String)\n  .. r").arenas
            ),
            "(type T (Known Int64) (Unknown String) .. r)"
        );
        assert_eq!(
            assert_roundtrip("type T = | Known(Int64) | Unknown(String) .. r", 80),
            "type T =\n  | Known(Int64)\n  | Unknown(String)\n  .. r"
        );
        // The canonical s-expr open sum prints back to the `.. r` ML surface.
        assert_eq!(
            print(
                &sexpr::read("(type Vocab (Known Unit) (Unknown Unit) .. r)").unwrap(),
                80
            ),
            "type Vocab =\n  | Known(Unit)\n  | Unknown(Unit)\n  .. r"
        );
        // A CLOSED sum (no `.. r`) is unchanged — a trailing lowercase would be a nullary variant, but
        // there is none here; the two-sibling peel only fires on a `..` second-to-last atom.
        assert_eq!(
            assert_roundtrip("type C = | A | B(Int64)", 80),
            "type C =\n  | A\n  | B(Int64)"
        );
        // A ZERO-named-variant open sum (`type Opaque = .. r`) — the body is ONLY a row tail, no
        // variants. The parser must not require a leading variant (it skips the variant loop at a `..`
        // head), and the printer emits `.. r` on its own line. Round-trips through both ML and s-expr.
        // (Regression: `type_expr`'s variant loop used to call `variant()` unconditionally, so it hit
        // `..` expecting a name and failed to re-parse the printed form — a trunk-red round-trip gap.)
        assert_eq!(
            sexpr::print(&parser::read_ml("type Opaque =\n  .. r").arenas),
            "(type Opaque .. r)"
        );
        assert_eq!(
            assert_roundtrip("type Opaque = .. r", 80),
            "type Opaque =\n  .. r"
        );
        assert_eq!(
            print(&sexpr::read("(type Opaque .. r)").unwrap(), 80),
            "type Opaque =\n  .. r"
        );
    }

    #[test]
    fn nullary_variant_as_a_one_element_list_renders_as_a_type_decl_not_a_backtick_application() {
        // A nullary variant has TWO arena spellings: a bare atom `A` (from ML `A`) AND a 1-element list
        // `(A)` (from ML `A()`). `is_type_shape` used to require a LIST variant have len >= 2, so a type
        // with an `(A)` variant failed the shape check and rendered as the backtick-fallback application
        // `` `type`(T, A(), B(Int64)) `` — which does NOT round-trip under an `@invariant`/annotation
        // wrapper (the annotation re-binds to `type` as a value head). v-verification hit this on an
        // @invariant establish corpus case. Fix: accept a 1-elem list nullary + render `(A)` as `A()`
        // (the empty parens PRESERVED — see the next paragraph for why NOT bare `A`).
        //
        // The 1-elem-list `(A)` variant renders as a proper `type T = | A() | B(Int64)` — the `()`
        // PRESERVED (NOT bare `A`), because `(A)` (1-elem list) and `A` (atom) are DISTINCT arenas and
        // corpus_roundtrip requires read(ml(read(x))) == read(x) EXACTLY (no canonicalization). `A()`
        // re-reads to the 1-elem list `(A)`, so the exact shape survives.
        assert_eq!(
            print(&sexpr::read("(type T (A) (B Int64))").unwrap(), 80),
            "type T =\n  | A()\n  | B(Int64)"
        );
        // ...and re-reads BACK to the exact 1-elem-list `(A)` form (round-trip-preserving, NOT canonicalized).
        assert_eq!(
            sexpr::print(&parser::read_ml("type T =\n  | A()\n  | B(Int64)").arenas),
            "(type T (A) (B Int64))"
        );
        // The bare-atom nullary `A` (from ML `A`) is a SEPARATE shape and still renders/round-trips as `A`.
        assert_eq!(
            print(&sexpr::read("(type T A (B Int64))").unwrap(), 80),
            "type T =\n  | A\n  | B(Int64)"
        );
        // The full v-verification repro: an @invariant on a multi-variant type with a nullary variant
        // round-trips (the @invariant stays bound to the `(type …)`, not mis-bound to `type` as a value).
        let src = "(do (@ (invariant (match self (((. T A)) false) (((. T B) x) (> x 0)))) (type T (A) (B Int64))))";
        let ml = print(&sexpr::read(src).unwrap(), 80);
        assert!(
            ml.contains("type T =") && !ml.contains("`type`"),
            "the annotated type must render as a `type` decl, not a backtick application; got:\n{ml}"
        );
        // ml -> sexpr keeps the @invariant bound to the type declaration (arena-idempotent).
        let back = sexpr::print(&parser::read_ml(&ml).arenas);
        assert!(
            back.contains("(@ (invariant") && back.contains("(type T (A) (B Int64))"),
            "the @invariant must stay bound to the (type …) with the exact (A) shape preserved, not \
             (@ … type) as an application head; got:\n{back}"
        );
    }

    #[test]
    fn type_annotated_parameter() {
        // A `(: name Type)` binder in a signature prints as `name: Type` and round-trips.
        assert_eq!(
            assert_roundtrip("def annotated(a: Int64, b) = a + b", 80),
            "def annotated(a: Int64, b) = a + b"
        );
        // and in a lambda parameter list
        assert_eq!(assert_roundtrip("fn(x: Bool) => x", 80), "fn(x: Bool) => x");
        // the underlying AST is the binder-position annotation `(: a Int64)`
        let a = sexpr::read("(def (f (: a Int64)) a)").unwrap();
        assert_eq!(print(&a, 80), "def f(a: Int64) = a");
    }

    #[test]
    fn function_type_arrow() {
        // `(-> A B)` prints as the ML infix `A -> B` and round-trips (the inverse of the reader).
        assert_eq!(
            print(&sexpr::read("(-> Int64 Bool)").unwrap(), 80),
            "Int64 -> Bool"
        );
        // RIGHT-associative: `A -> B -> C` is `(-> A (-> B C))`, printed without inner parens.
        let a = sexpr::read("(-> Int64 (-> Int64 Bool))").unwrap();
        assert_eq!(print(&a, 80), "Int64 -> Int64 -> Bool");
        // A left-nested arrow (a function-typed ARGUMENT) parenthesizes: `(A -> B) -> C`.
        let a = sexpr::read("(-> (-> Int64 Int64) Bool)").unwrap();
        assert_eq!(print(&a, 80), "(Int64 -> Int64) -> Bool");
        // An arrow type in a parameter annotation round-trips.
        assert_eq!(
            assert_roundtrip("def apply(g: Int64 -> Bool, n: Int64) = g(n)", 80),
            "def apply(g: Int64 -> Bool, n: Int64) = g(n)"
        );
    }

    #[test]
    fn return_type_annotation() {
        // A def return type `-> R` desugars to a body ascription `(: body R)` and prints back in
        // signature position. It round-trips and the underlying AST is the ascription form.
        assert_eq!(
            assert_roundtrip("def add(x: Int64, y: Int64) -> Int64 = x + y", 80),
            "def add(x: Int64, y: Int64) -> Int64 = x + y"
        );
        let a = parser::read_ml("def add(x: Int64) -> Int64 = x + 1");
        assert_eq!(
            sexpr::print(&a.arenas),
            "(def (add (: x Int64)) (: (+ x 1) Int64))"
        );
        // A lambda return type behaves the same way.
        assert_eq!(
            assert_roundtrip("fn(x: Int64) -> Int64 => x * 2", 80),
            "fn(x: Int64) -> Int64 => x * 2"
        );
        // A return type that IS a function type (curried) reads as one arrow chain.
        assert_eq!(
            assert_roundtrip("def mk(k: Int64) -> Int64 -> Int64 = fn(x) => x + k", 80),
            "def mk(k: Int64) -> Int64 -> Int64 = fn(x) => x + k"
        );
        // A body written as a bare value ascription `(: e R)` canonicalizes to the return-type form —
        // one AST, printed in the cleaner spelling.
        let a = sexpr::read("(def (main) (: (f x) Int64))").unwrap();
        assert_eq!(print(&a, 80), "def main() -> Int64 = f(x)");
    }

    #[test]
    fn record_type_fields_print_as_colon_ascription_and_round_trip() {
        // DESIGN-record-type-syntax RT2 (operator PR #2794): the canonical record-TYPE field is the
        // `(: name T)` ascription node — the SAME node as a param binder / `e: T`, not a bespoke pair.
        // A `Record` type head prints each `(: field T)` child through the infix `:` path as
        // `field : T`, and the printed surface re-parses back to the identical ascription arena. This
        // pins the canonical-form behavior BEFORE Phase A migrates the ~141 head-app `(field T)` cases,
        // so the atomic RT1+RT3+RT4 land cannot silently regress the ascription round-trip.
        //
        // Arena -> surface: the ascription children print as `a : Int64` (infix `:`, spaced), never as
        // the head-app spelling `a(Int64)`.
        let a = sexpr::read("(type R (Record (: a Int64) (: b Bool)))").unwrap();
        let out = print(&a, 80);
        assert_eq!(out, "type R =\n  | Record(a : Int64, b : Bool)");
        assert!(
            !out.contains("a(Int64)"),
            "field must not print as head-app: {out:?}"
        );
        // Surface -> arena -> surface: the printed colon form re-parses to the same ascription arena
        // and reprints identically (round-trip fixed point).
        assert_eq!(
            assert_roundtrip("type R =\n  | Record(a : Int64, b : Bool)", 80),
            "type R =\n  | Record(a : Int64, b : Bool)"
        );
        // Single field, same shape.
        let a = sexpr::read("(type R (Record (: a Int64)))").unwrap();
        assert_eq!(print(&a, 80), "type R =\n  | Record(a : Int64)");
    }

    #[test]
    fn multi_statement_function_body_prints_bare() {
        // A `(do …)` FUNCTION body prints as a bare `;`-separated statement run under the `=` — the
        // exact surface the parser folds back into that `(do …)`. No wrapping parens.
        let a = sexpr::read("(def (f) (do (g 20) (g 5) (h)))").unwrap();
        assert_eq!(print(&a, 80), "def f() =\n  g(20);\n  g(5);\n  h()");
        // and it round-trips from that ML surface
        assert_eq!(
            assert_roundtrip("def f() =\n  g(20);\n  g(5);\n  h()", 80),
            "def f() =\n  g(20);\n  g(5);\n  h()"
        );
    }

    #[test]
    fn top_level_forms_have_no_semicolons_between_keyword_forms() {
        // An all-declaration program (every next form keyword-led) prints with NO `;` at all — `;` is
        // the within-body sequencer, not a top-level separator.
        let a = sexpr::read("(do (def (f) 1) (def (g) 2) (export f))").unwrap();
        assert_eq!(print(&a, 80), "def f() = 1\n\ndef g() = 2\n\nexport { f }");
    }

    #[test]
    fn bare_body_before_open_next_form_is_delimited() {
        // A def whose plain body is followed by a NON-keyword form (`f(9)`) parenthesizes the body, so
        // the trailing token cannot fuse with the next form (`1 f` would re-lex as a quantity literal).
        // The `def`'s greedy body cannot instead take a `;` (it would swallow it), hence body parens.
        let a = sexpr::read("(do (def (f n) (+ n 1)) (f 9))").unwrap();
        assert_eq!(print(&a, 80), "def f(n) = (n + 1)\n\nf(9)");
        assert_eq!(
            assert_roundtrip("def f(n) = (n + 1)\n\nf(9)", 80),
            "def f(n) = (n + 1)\n\nf(9)"
        );
    }

    #[test]
    fn bare_expression_before_open_next_form_takes_a_semicolon() {
        // Two bare top-level expressions where the second could fuse onto the first take a `;` — which
        // re-parses as a stmt-level `(do …)` the root splices flat, preserving the tree. (Parens would
        // be wrong: `(5)` then `(x)` re-lexes `)(` as application.)
        let a = sexpr::read("(do (def x 5) (+ x 1))").unwrap();
        assert_eq!(print(&a, 80), "def x = (5)\n\nx + 1");
        assert_eq!(
            assert_roundtrip("def x = (5)\n\nx + 1", 80),
            "def x = (5)\n\nx + 1"
        );
    }

    #[test]
    fn greedy_tailed_statement_in_a_sequence_is_parenthesized() {
        // A non-final `match` (greedy-tailed) inside a value-position bare sequence is wrapped so its
        // last arm body does not swallow the following `; rest`.
        let a = sexpr::read("(def (f) (do (match 0 (0 a) (_ 99)) (next)))").unwrap();
        let out = print(&a, 80);
        assert!(
            out.contains("| _ => 99);"),
            "the match wraps before the `;`, got: {out:?}"
        );
        // And a greedy-tailed form at the ROOT, before a non-keyword form, is wrapped whole (a `)`
        // closes its tail; no `;` — the wrapping alone prevents the swallow).
        let r = sexpr::read("(do (match 0 (0 a) (_ 99)) (next))").unwrap();
        let rout = print(&r, 80);
        assert!(
            rout.starts_with("(match 0 with") && rout.contains("| _ => 99)\n"),
            "root match wraps whole before `next()`, got: {rout:?}"
        );
    }

    #[test]
    fn module_block() {
        // Members are blank-line separated so a wall of defs reads with breathing room; the first
        // member still hugs the `{`.
        let out = assert_roundtrip(
            "module math { def add(a, b) = a + b def main() = add(2, 3) }",
            80,
        );
        assert_eq!(
            out,
            "module math {\n  def add(a, b) = a + b\n\n  def main() = add(2, 3)\n}"
        );
    }

    #[test]
    fn top_level_defs_are_blank_separated() {
        // Consecutive top-level definitions are JUXTAPOSED — blank-line separated, NO `;` between them
        // (`;` is the within-body sequencing operator, not a top-level separator) — and the layout
        // round-trips (blank lines are whitespace).
        assert_eq!(
            assert_roundtrip("def a = 1 def b = 2 def c = 3", 80),
            "def a = 1\n\ndef b = 2\n\ndef c = 3"
        );
    }

    #[test]
    fn doc_line_hugs_its_def_no_blank() {
        // A `///` doc line stays glued to the def it documents (single break), while distinct
        // definitions are still blank-separated — again with no `;` between the top-level forms.
        assert_eq!(
            assert_roundtrip("/// first\ndef a = 1\n/// second\ndef b = 2", 80),
            "/// first\ndef a = 1\n\n/// second\ndef b = 2"
        );
    }

    #[test]
    fn def_match_body_hugs_the_eq() {
        // A `match … with` body stays on the `=` line; its `|`-arms break one per line, indented one
        // level under the def.
        let out = assert_roundtrip("def describe(s) = match s with | A(_) => 1 | B(_) => 2", 80);
        assert_eq!(
            out,
            "def describe(s) = match s with\n  | A(_) => 1\n  | B(_) => 2"
        );
    }

    #[test]
    fn def_let_body_drops_flat_at_the_let_column() {
        // A non-brace body (let) is not hugged: it drops to an indented continuation line. The `let`
        // and its FINAL body share one indent — the body is FLAT at the `let` column, not nested a
        // level deeper (operator seq-86: "why is the last statement indented").
        let out = assert_roundtrip("def f(x) = let y = x + 1 in y * y", 80);
        assert_eq!(out, "def f(x) =\n  let y = x + 1 in\n  y * y");
    }

    #[test]
    fn single_expression_arm_body_stays_bare_no_unneeded_parens_seq101() {
        // Operator seq-101 (bug in seq-95): a SINGLE-EXPRESSION arm body (a call/var/literal) must stay
        // BARE after `=>` — NOT wrapped in parens. Only a body that genuinely needs grouping (a block
        // form: a let-in chain, a multi-line body) gets the seq-95 `=> ( … )` paren layout. Handle arms
        // exercise both: non-last call bodies are bare; the non-last `let` body is paren-wrapped.
        let out = assert_roundtrip(
            "def main(db0) = handle DbState(db0) with\n  | get-tcol(db) => resume(types-col(db), db)\n  | get-ty(id, db) => resume(require-ty(db, id), db)\n  | set-ty(pair, db) => let (id, t) = pair in resume(unit, fill-ty(db, id, t))\n  | get-resolved(id, db) => resume(require-resolved(db, id), db)\n  in run-program(db0)",
            100,
        );
        assert_eq!(
            out,
            "def main(db0) =\n  handle DbState(db0) with\n    | get-tcol(db) => resume(types-col(db), db)\n    | get-ty(id, db) => resume(require-ty(db, id), db)\n    | set-ty(pair, db) => (\n      let (id, t) = pair in\n      resume(unit, fill-ty(db, id, t))\n    )\n    | get-resolved(id, db) => resume(require-resolved(db, id), db)\n  in\n  run-program(db0)"
        );
    }

    #[test]
    fn match_arm_flush_is_conditional_on_own_line_seq96_97() {
        // Operator seq-96/97: match arms FLUSH with the `match` keyword when `match` starts its OWN line
        // (a let/if body-tail, a statement); they stay INDENTED one level when `match` is BOUND inline
        // to a preceding token (`def f = match …`, a call arg).

        // BOUND — `match` on the `def … =` line → arms INDENTED.
        assert_eq!(
            assert_roundtrip(
                "def t(id) = match g(id) with | S(t) => t | N(_) => c(id)",
                80
            ),
            "def t(id) = match g(id) with\n  | S(t) => t\n  | N(_) => c(id)"
        );
        // OWN-LINE — the `match` is a `let` in-body on its own line → arms FLUSH with `match`.
        assert_eq!(
            assert_roundtrip(
                "def f(x) = let y = p(x) in match y with | A => 1 | B => 2",
                80
            ),
            "def f(x) =\n  let y = p(x) in\n  match y with\n  | A => 1\n  | B => 2"
        );
        // BOUND — a `match` as a call ARG stays INDENTED (value position, not a statement).
        assert_eq!(
            assert_roundtrip("def f(x) = g(match x with | A => 1 | B => 2)", 40),
            "def f(x) =\n  g(match x with\n    | A => 1\n    | B => 2)"
        );
    }

    #[test]
    fn parenthesized_arm_body_opens_paren_on_the_arm_line_seq95() {
        // Operator seq-95: a PARENTHESIZED match-arm body puts the open `(` on the `=>` line, the body
        // indented one level under the arm, and the close `)` on its OWN line dedented to the arm indent
        // — not `(expr` bound tight nor a trailing `)` glued to the last body line.
        assert_eq!(
            assert_roundtrip(
                "def f() = match x with | A => (match x with | C => 1 | _ => 2) | B => 3",
                200,
            ),
            "def f() = match x with\n  | A => (\n    match x with\n    | C => 1\n    | _ => 2\n  )\n  | B => 3"
        );
    }

    #[test]
    fn def_and_fn_param_lists_wrap_one_per_line_when_they_overflow() {
        // Operator seq-92/93: a `def`/`fn` param list that does NOT fit goes one-param-per-line — open
        // `(` on the header, each param indented one level, close `)` on its own dedented line. A param
        // list that FITS stays inline.
        assert_eq!(
            assert_roundtrip(
                "def v3q(x: Qty(Rational, Unit.base(#meter)), y: Qty(Rational, Unit.base(#meter)), z: Qty(Rational, Unit.base(#meter))) = Vec3q.V3q(x, y, z)",
                100,
            ),
            "def v3q(\n  x: Qty(Rational, Unit.base(#meter)),\n  y: Qty(Rational, Unit.base(#meter)),\n  z: Qty(Rational, Unit.base(#meter))\n) = Vec3q.V3q(x, y, z)"
        );
        // Fits on one line → stays inline (no force-break).
        assert_eq!(
            assert_roundtrip("def f(x: Int64, y: Int64) = x + y", 80),
            "def f(x: Int64, y: Int64) = x + y"
        );
    }

    #[test]
    fn multi_line_match_arm_body_breaks_to_its_own_indented_line() {
        // Operator follow-on to seq-86/87/89: a MULTI-LINE match-arm body goes on a NEW line indented
        // one level under `=>` (a let-in chain / a body that wraps); a SINGLE-LINE body stays inline.
        let out = assert_roundtrip(
            "def profile-half-extent(p: Profile(Rational)) = match p with\n  | Profile.Rect(sz) => let Vec2.V2(w, h) = sz in Vec2.V2(rhalf(w), rhalf(h))\n  | Profile.Circle(r) => Vec2.V2(r, r)\n  | Profile.PathProfile(pth) => path-half-extent(pth)",
            100,
        );
        assert_eq!(
            out,
            "def profile-half-extent(p: Profile(Rational)) = match p with\n  | Profile.Rect(sz) =>\n    let Vec2.V2(w, h) = sz in\n    Vec2.V2(rhalf(w), rhalf(h))\n  | Profile.Circle(r) => Vec2.V2(r, r)\n  | Profile.PathProfile(pth) => path-half-extent(pth)"
        );
    }

    #[test]
    fn compound_body_indentation_is_coherent_operator_seq_86_87_89() {
        // The operator flagged THREE examples of "funky" compound-body indentation; the coherent model:
        // a construct's BODY indents ONE level under its header, and a flat CHAIN (let-in / else-if
        // ladder) stays at ONE level (no per-rung deepening). Pin all three shapes.

        // seq-86: a let-in CHAIN's final body is FLAT at the chain indent (not nested a level deeper).
        assert_eq!(
            assert_roundtrip("def f() = let a = 1 in let b = 2 in a + b", 80),
            "def f() =\n  let a = 1 in\n  let b = 2 in\n  a + b"
        );

        // seq-87: a let that IS a match-arm body indents ONE level UNDER the `=>` arm (not flush with
        // the `|` markers) — here forced to its own line by the `// MISS` comment trailing `=>`. The
        // `//` comments round-trip in place.
        let seq87 = assert_roundtrip(
            "def demand-typed-leaf(db: Db, id: Int64) = match require-ty(db, id) with\n  | Option.Some(fact) => (db, fact) // memo HIT — no recompute\n  | Option.None(_) => // MISS — compute from source, fill, thread\n  let ty = compute-leaf-type(db-tree(db), id) in\n  (fill-ty(db, id, ty), ty)",
            100,
        );
        assert_eq!(
            seq87,
            "def demand-typed-leaf(db: Db, id: Int64) = match require-ty(db, id) with\n  | Option.Some(fact) => (db, fact) // memo HIT — no recompute\n  | Option.None(_) => // MISS — compute from source, fill, thread\n    let ty = compute-leaf-type(db-tree(db), id) in\n    (fill-ty(db, id, ty), ty)"
        );

        // seq-89: mixed let-in + if-then-else — the let-in body (`if`) is FLAT at the let indent; each
        // then/else BRANCH body indents one level under its `if … then` / `else` header; `else`
        // dedents to its `if` column. The leading `//` comment round-trips.
        let seq89 = assert_roundtrip(
            "@test\ndef dd-miss-fills-and-returns() =\n  // a demand on an ABSENT slot computes the fact, fills the column, returns the fact.\n  let (db1, fact) = demand-typed(sample-db(), 0, mk-int(true, 64)) in\n  if is-int-ty(fact) then\n  let (s, w) = int-parts-of(fact) in\n  if s and w == 64 then\n  if typed-filled(db1) == 1 then unit else trap(\"filled 1\")\n  else trap(\"Int64\")\n  else trap(\"demand returns the fact\")",
            100,
        );
        assert_eq!(
            seq89,
            "@test\ndef dd-miss-fills-and-returns() =\n  // a demand on an ABSENT slot computes the fact, fills the column, returns the fact.\n  let (db1, fact) = demand-typed(sample-db(), 0, mk-int(true, 64)) in\n  if is-int-ty(fact) then\n    let (s, w) = int-parts-of(fact) in\n    if s and w == 64 then\n      if typed-filled(db1) == 1 then unit else trap(\"filled 1\")\n    else\n      trap(\"Int64\")\n  else\n    trap(\"demand returns the fact\")"
        );
    }

    #[test]
    fn def_record_body_hugs_the_eq() {
        // A literal body (record) hugs the `=` too: `{` stays on the line, fields indent one level.
        let out = assert_roundtrip("def point() = { x = 1, y = 2, z = 3 }", 20);
        assert_eq!(out, "def point() = {\n  x = 1,\n  y = 2,\n  z = 3\n}");
        // and inline when it fits
        assert_eq!(
            assert_roundtrip("def point() = { x = 1 }", 80),
            "def point() = { x = 1 }"
        );
    }

    #[test]
    fn last_arg_lambda_hugs() {
        // A trailing lambda stays on the call line, breaking only its own body — head args inline.
        let out = assert_roundtrip(
            "fold(xs, zero, fn(acc, x) => match x with | Some(v) => acc + v | None(_) => acc)",
            80,
        );
        assert_eq!(
            out,
            "fold(xs, zero, fn(acc, x) => match x with\n  | Some(v) => acc + v\n  | None(_) => acc)"
        );
    }

    #[test]
    fn last_arg_hug_fits_inline() {
        // When the whole call fits, hugging is invisible — it stays on one line.
        assert_eq!(
            assert_roundtrip("map(items, fn(x) => x + 1)", 80),
            "map(items, fn(x) => x + 1)"
        );
    }

    #[test]
    fn infix_chain_breaks_at_one_indent() {
        // A same-precedence chain flattens: operators break at ONE consistent 2-space indent, each
        // leading its continuation line; tighter sub-terms (`*`) stay intact.
        let out = assert_roundtrip("aaaa * bbbb + cccc * dddd", 15);
        assert_eq!(out, "aaaa * bbbb\n  + cccc * dddd");
    }

    #[test]
    fn plain_call_all_or_nothing_when_wide() {
        // A call with no block-like last arg breaks all args one per line when it overflows.
        let out = assert_roundtrip("some-function(alpha, beta, gamma, delta)", 20);
        assert_eq!(
            out,
            "some-function(\n  alpha,\n  beta,\n  gamma,\n  delta\n)"
        );
    }

    #[test]
    fn literals_render_and_round_trip() {
        assert_eq!(assert_roundtrip("{ x = 1, y = 2 }", 80), "{ x = 1, y = 2 }");
        assert_eq!(assert_roundtrip("[1, 2, 3]", 80), "[1, 2, 3]");
        assert_eq!(assert_roundtrip("(1, 2)", 80), "(1, 2)");
        assert_eq!(assert_roundtrip("(1, 2, 3)", 80), "(1, 2, 3)");
        // maps use `=` (like records); the `#` sigil is what distinguishes them.
        assert_eq!(
            assert_roundtrip("#{ \"a\" = 1, \"b\" = 2 }", 80),
            "#{ \"a\" = 1, \"b\" = 2 }"
        );
        assert_eq!(assert_roundtrip("#{ 1 = 10 }", 80), "#{ 1 = 10 }");
    }

    #[test]
    fn a_same_line_trailing_comment_on_the_last_record_or_map_field_is_preserved() {
        // The record/map siblings of the list/tuple/set trailing-comment fix. A field/entry is a
        // `(name value)` PAIR (not a bare value), so the `(comment-after "text" (pair))` wrapper is
        // unwrapped by the shape-guards (`is_pairs`/`is_record_shape`) AND the printer
        // (`bracketed_pairs_comment_aware`), which re-emits ` // text` after the field and forces `}` to
        // its own line. Captured only on the LAST field (PR#758 gate); a mid-field comment is left to the
        // drop-guard. strip_comments peels it — a record with a trailing comment compiles.
        assert_eq!(
            sexpr::print(&parser::read_ml("def r() -> Int64 = { a = 1, b = 2 // last\n}").arenas),
            "(def (r) (: #record((= a 1) (comment-after \"last\" (= b 2))) Int64))",
            "a trailing comment on the last RECORD field is captured, not dropped"
        );
        assert_eq!(
            assert_roundtrip("{ a = 1, b = 2 // last\n}", 80),
            "{\n  a = 1,\n  b = 2 // last\n}"
        );
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def m() -> Int64 = #{ 1 = 10, 2 = 20 // last\n}").arenas
            ),
            "(def (m) (: #map((= 1 10) (comment-after \"last\" (= 2 20))) Int64))",
            "a trailing comment on the last MAP entry is captured, not dropped"
        );
        assert_eq!(
            assert_roundtrip("#{ 1 = 10, 2 = 20 // last\n}", 80),
            "#{\n  1 = 10,\n  2 = 20 // last\n}"
        );
        // Clean record/map (incl. field-shorthand pun) keep their ordinary flat layout — no forced break.
        assert_eq!(assert_roundtrip("{ x = 1, y = 2 }", 80), "{ x = 1, y = 2 }");
        assert_eq!(assert_roundtrip("{ x, y }", 80), "{ x, y }");
        assert_eq!(assert_roundtrip("#{ 1 = 10 }", 80), "#{ 1 = 10 }");
    }

    #[test]
    fn an_own_line_comment_before_a_record_or_map_field_is_preserved_not_dropped() {
        // The record/map own-line interior sibling. A field/entry is a `(name value)` PAIR, so an own-line
        // leading `//` wraps it as `(comment "text" (name value))` — which the shape-guards (`is_pairs`/
        // `is_record_shape` via `strip_field_comments`) unwrap, and the printer (`bracketed_pairs_comment_
        // aware`) renders as a `// …` line ABOVE the field, forcing the container to break. Distinct from
        // the same-line trailing `(comment-after …)`. Own-line has no swallow hazard → works at any field.
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def r() -> Int64 = {\n  // lead\n  a = 1, b = 2 }").arenas
            ),
            "(def (r) (: #record((comment \"lead\" (= a 1)) (= b 2)) Int64))",
            "own-line comment before the first record field is captured, printer renders it above"
        );
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def r() -> Int64 = { a = 1,\n  // mid\n  b = 2 }").arenas
            ),
            "(def (r) (: #record((= a 1) (comment \"mid\" (= b 2))) Int64))",
            "own-line comment before a non-first record field is captured (no swallow hazard)"
        );
        assert_eq!(
            assert_roundtrip("{\n  // lead\n  a = 1, b = 2 }", 80),
            "{\n  // lead\n  a = 1,\n  b = 2\n}"
        );
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def m() -> Int64 = #{\n  // lead\n  1 = 10, 2 = 20 }").arenas
            ),
            "(def (m) (: #map((comment \"lead\" (= 1 10)) (= 2 20)) Int64))",
            "own-line comment before a map entry is captured"
        );
        // Leading own-line + trailing same-line (last field) compose.
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def r() -> Int64 = {\n  // lead\n  a = 1, b = 2 // last\n}")
                    .arenas
            ),
            "(def (r) (: #record((comment \"lead\" (= a 1)) (comment-after \"last\" (= b 2))) Int64))",
            "leading own-line + trailing same-line record comments compose"
        );
        // Clean record/map (incl. pun) keep their flat layout.
        assert_eq!(assert_roundtrip("{ x = 1, y = 2 }", 80), "{ x = 1, y = 2 }");
        assert_eq!(assert_roundtrip("{ x, y }", 80), "{ x, y }");
    }

    #[test]
    fn multiple_comment_wrappers_on_a_record_field_all_print_not_just_the_first() {
        // PR#768 (Copilot): `emit_field` peeled only ONE leading `(comment …)` (a single `if let`), so a
        // field with MORE than one comment wrapper — only from a decoded / metaprogramming-built AST, but
        // the printer must be TOTAL (PR#763-class) — mis-printed: the inner `(comment c2 (pair))` was
        // handed to `emit` as if `comment` were the field name (`// c1` then a spurious `comment = "c2"`).
        // The peel is now a LOOP over both leading `(comment …)` and trailing `(comment-after …)` layers.
        // TWO leading comments: both print as `// …` lines above the field; round-trips.
        let a = sexpr::read(
            r#"(def (r) (: ("record" (comment "c1" (comment "c2" (= a 1))) (= b 2)) _))"#,
        )
        .unwrap();
        let printed = print(&a, 80);
        assert_eq!(
            printed, "def r() -> _ = {\n  // c1\n  // c2\n  a = 1,\n  b = 2\n}",
            "both leading comments print, not just the first (no spurious `comment = ...` field)"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml(&printed).arenas),
            r#"(def (r) (: #record((comment "c1" (comment "c2" (= a 1))) (= b 2)) _))"#,
            "a doubly-commented field round-trips"
        );
        // A leading + trailing combo on one field normalizes to a stable (idempotent) nesting — nothing
        // dropped or mis-attributed; re-printing is a fixed point.
        let combo = sexpr::read(
            r#"(def (r) (: ("record" (= a 1) (comment "lead" (comment-after "trail" (= b 2)))) _))"#,
        )
        .unwrap();
        let p1 = print(&combo, 80);
        let p2 = print(&parser::read_ml(&p1).arenas, 80);
        assert_eq!(
            p1, p2,
            "leading+trailing on one field is idempotent (stable after normalization)"
        );
    }

    #[test]
    fn a_same_line_trailing_comment_on_a_list_elem_is_preserved_not_dropped() {
        // A `//` comment trailing a list element on the SAME source line (`[…, x // note]`) used to be
        // DROPPED (it sat in the next token's / the `]`'s leading slot, which the element loop never
        // drained), so `cdz fmt` REFUSED the whole file via the comment-drop guard. list_literal now
        // captures it as a `(comment-after "text" elem)` wrapper (mirroring the sum-type variant loop);
        // strip_comments peels it, so the compiler is unaffected.
        let src = "def l() -> List(Int64) = [1, 2 // last\n]";
        let tree = sexpr::print(&parser::read_ml(src).arenas);
        assert_eq!(
            tree, "(def (l) (: #list(1 (comment-after \"last\" 2)) (List Int64)))",
            "the same-line trailing `//` on the last elem is captured, not dropped"
        );
        // The printer re-emits it SAME-LINE and — crucially — forces the closing `]` onto the NEXT line,
        // else `]` is swallowed into the `// last` comment and the printed form fails to re-parse. Verify
        // the exact layout AND that it round-trips (re-reading the printed form yields the same tree).
        let printed = print(&parser::read_ml(src).arenas, 80);
        assert_eq!(
            printed, "def l() -> List(Int64) = [\n  1,\n  2 // last\n]",
            "trailing comment prints same-line; `]` breaks to its own line"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml(&printed).arenas),
            "(def (l) (: #list(1 (comment-after \"last\" 2)) (List Int64)))",
            "the trailing list comment round-trips"
        );
        // A clean list with no trailing comment keeps the ordinary flat layout (no forced break).
        assert_eq!(assert_roundtrip("[1, 2, 3]", 80), "[1, 2, 3]");
        assert_eq!(assert_roundtrip("[]", 80), "[]");
    }

    #[test]
    fn a_nonlast_comment_after_in_a_decoded_ast_falls_back_and_round_trips_not_swallowed() {
        // PR#763 (Copilot): a printer shape/render guard must be correct on ANY AST, not just the reader's
        // (the reader gates same-line-comment capture to the LAST element). A decoded / metaprogramming-
        // built AST can carry a `(comment-after …)` on a NON-last element. Rendering it inline would emit
        // `elem // text , next` — the `, next` swallowed into the comment line → invalid re-parse (the
        // printer-side PR#758 break). Every collection literal now DECLINES its sugared surface when a
        // non-last element is comment-after-wrapped, falling back to the generic call form, which
        // round-trips `comment-after(...)` faithfully. Pin round-trip for all containers.
        // PR#781 (Copilot): `print_bin` (b[…]) reached `bracketed_comment_aware` UNGUARDED (no dispatch
        // guard like list/tuple/record/map/set) — so `bin` was added here after `bracketed_comment_aware`
        // itself gained the `has_nonlast_comment_after` self-guard (fixing every caller, present + future).
        for (label, sx) in [
            (
                "list",
                r#"(def (l) (: ("list" (comment-after "mid" 1) 2) _))"#,
            ),
            (
                "tuple",
                r#"(def (t) (: ("tuple" (comment-after "mid" 1) 2) _))"#,
            ),
            (
                "record",
                r#"(def (r) (: ("record" (comment-after "mid" (a 1)) (b 2)) _))"#,
            ),
            (
                "map",
                r#"(def (m) (: ("map" (comment-after "mid" (1 10)) (2 20)) _))"#,
            ),
            (
                "set",
                r#"(def (s) (: ((. Set of) ("list" (comment-after "mid" 1) 2)) _))"#,
            ),
            (
                "bin",
                r#"(def (b) (: (bin (comment-after "mid" (u8 1)) (u8 2)) Bytes))"#,
            ),
        ] {
            let a = sexpr::read(sx).unwrap();
            let printed = print(&a, 80);
            let reparsed = sexpr::print(&parser::read_ml(&printed).arenas);
            assert_eq!(
                reparsed, sx,
                "{label}: a non-last comment-after must round-trip via call-form fallback, not swallow the separator (printed: {printed})"
            );
        }
    }

    #[test]
    fn an_own_line_comment_before_a_call_argument_is_preserved_not_dropped() {
        // An OWN-LINE `//` comment LEADING a call argument (`g(\n // note\n 1, 2)` or between args) used to
        // be DROPPED — `arg_exprs` parses each argument via `expr`, which does not drain the argument's
        // leading-comment slot. `arg_exprs` now captures it as a leading `(comment "text" arg)`; the call
        // printer already renders a leading `(comment …)` on its own line above the arg. Own-line has no
        // swallow hazard → not gated to the last arg. (Same-line trailing call-arg comments are a separate
        // follow-up — the two-path call printer needs work to render `arg // text` + force the `)` break.)
        assert_eq!(
            sexpr::print(&parser::read_ml("def f() -> Int64 = g(\n  // lead\n  1, 2)").arenas),
            "(def (f) (: (g (comment \"lead\" 1) 2) Int64))",
            "own-line comment before the first call arg is captured, not dropped"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("def f() -> Int64 = g(1,\n  // mid\n  2)").arenas),
            "(def (f) (: (g 1 (comment \"mid\" 2)) Int64))",
            "own-line comment before a non-first call arg is captured"
        );
        assert_eq!(
            assert_roundtrip("g(\n  // lead\n  1, 2)", 80),
            "g(\n  // lead\n  1,\n  2\n)"
        );
        // A clean call keeps its ordinary layout.
        assert_eq!(assert_roundtrip("g(1, 2, 3)", 80), "g(1, 2, 3)");
    }

    #[test]
    fn a_same_line_trailing_comment_on_the_last_call_argument_is_preserved_not_dropped() {
        // A same-line `//` trailing the LAST call argument (`g(1, 2 // note)`) used to be DROPPED (it sat
        // in the `)` leading slot, which `arg_exprs` didn't drain) → `cdz fmt` refused. `arg_exprs` now
        // captures it as `(comment-after …)` (gated on `at(RParen)`), and `call_args` routes a last-arg
        // comment-after to `plain_call_comment_aware`, which renders `arg // text` same-line and forces `)`
        // onto its own line so it isn't swallowed. `strip_comments` peels it; compiles to wasm.
        let src = "def f() -> Int64 = g(1, 2 // note\n)";
        assert_eq!(
            sexpr::print(&parser::read_ml(src).arenas),
            "(def (f) (: (g 1 (comment-after \"note\" 2)) Int64))",
            "the same-line trailing comment on the last call arg is captured, not dropped"
        );
        assert_eq!(
            print(&parser::read_ml(src).arenas, 80),
            "def f() -> Int64 =\n  g(\n    1,\n    2 // note\n  )",
            "trailing comment prints same-line; `)` breaks to its own line"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml(&print(&parser::read_ml(src).arenas, 80)).arenas),
            "(def (f) (: (g 1 (comment-after \"note\" 2)) Int64))",
            "the trailing call-arg comment round-trips"
        );
        // A `(comment-after …)` wrapping a would-be-huggable last arg (a `fn`/`match`) is NOT huggable
        // (its head is `comment-after`), so it routes to the comment-aware plain path and round-trips.
        let hug =
            sexpr::read(r#"(def (f) (: (map xs (comment-after "note" (fn (x) (+ x 1)))) _))"#)
                .unwrap();
        assert_eq!(
            sexpr::print(&parser::read_ml(&print(&hug, 80)).arenas),
            r#"(def (f) (: (map xs (comment-after "note" (fn (x) (+ x 1)))) _))"#,
            "a comment-after on a would-be-hugged last arg round-trips via the plain comment-aware path"
        );
        // A clean call keeps its flat layout (no forced break).
        assert_eq!(assert_roundtrip("g(1, 2, 3)", 80), "g(1, 2, 3)");
    }

    #[test]
    fn an_own_line_comment_before_a_list_element_is_preserved_not_dropped() {
        // An OWN-LINE `//` comment LEADING a list element (`[\n // note\n 1, …]` or between elements) used
        // to be DROPPED — `list_literal` parses each element via `expr`, which (unlike `stmt`/`body_expr`)
        // does not drain the element's leading-comment slot, so the comment was stranded (→ `cdz fmt`
        // refused the file). list_literal now captures it via `take_comments_here` + wraps the element in a
        // LEADING `(comment "text" elem)` (distinct from the same-line trailing `(comment-after …)`). The
        // printer already renders a leading `(comment …)` as a `// …` line above the element; strip_comments
        // peels it (compiles to wasm). This is the interior (own-line) half of the collection-comment gap.
        //
        // Before the FIRST element:
        assert_eq!(
            sexpr::print(&parser::read_ml("def l() -> List(Int64) = [\n  // lead\n  1, 2]").arenas),
            "(def (l) (: #list((comment \"lead\" 1) 2) (List Int64)))",
            "an own-line comment before the first element is captured, not dropped"
        );
        // BETWEEN elements (own-line before a non-first element) — safe (no swallow hazard, unlike a
        // same-line trailing mid-element comment which stays refused):
        assert_eq!(
            sexpr::print(&parser::read_ml("def l() -> List(Int64) = [1,\n  // mid\n  2]").arenas),
            "(def (l) (: #list(1 (comment \"mid\" 2)) (List Int64)))",
            "an own-line comment between elements is captured, not dropped"
        );
        // Round-trips (leading `//` prints on its own line above the element).
        assert_eq!(
            assert_roundtrip("[\n  // lead\n  1, 2]", 80),
            "[\n  // lead\n  1,\n  2\n]"
        );
        // Leading (own-line) AND trailing (same-line, last element) compose.
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def l() -> List(Int64) = [\n  // lead\n  1, 2 // last\n]").arenas
            ),
            "(def (l) (: #list((comment \"lead\" 1) (comment-after \"last\" 2)) (List Int64)))",
            "leading own-line + trailing same-line comments compose"
        );
    }

    #[test]
    fn an_own_line_comment_before_a_tuple_or_set_element_is_preserved_not_dropped() {
        // The tuple + set siblings of the list own-line interior fix (reader-only: the printer already
        // renders a leading `(comment …)` above the element). `(\n // note\n 1, 2)` / `#(\n // note\n 1, 2)`
        // used to DROP the comment (element parsed via `expr`, which doesn't drain the leading slot).
        // Own-line comments are NOT gated to the last element — no swallow hazard.
        // Tuple, before first + between elements:
        assert_eq!(
            sexpr::print(&parser::read_ml("def t() -> Int64 = (\n  // lead\n  1, 2)").arenas),
            "(def (t) (: #tuple((comment \"lead\" 1) 2) Int64))",
            "own-line comment before the first tuple element is captured"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("def t() -> Int64 = (1,\n  // mid\n  2)").arenas),
            "(def (t) (: #tuple(1 (comment \"mid\" 2)) Int64))",
            "own-line comment between tuple elements is captured"
        );
        assert_eq!(
            assert_roundtrip("(\n  // lead\n  1, 2)", 80),
            "(\n  // lead\n  1,\n  2\n)"
        );
        // Set:
        assert_eq!(
            sexpr::print(&parser::read_ml("def s() -> Int64 = #(\n  // lead\n  1, 2)").arenas),
            "(def (s) (: #set((comment \"lead\" 1) 2) Int64))",
            "own-line comment before the first set element is captured"
        );
        // A grouped (non-tuple) parenthesized expr with a leading own-line comment also round-trips (the
        // `first`-element leading capture covers the transparent-grouping outcome, not just tuples).
        assert_eq!(
            sexpr::print(&parser::read_ml("def g() -> Int64 = (\n  // c\n  1)").arenas),
            "(def (g) (: (comment \"c\" 1) Int64))",
            "own-line comment before a grouped expr is captured"
        );
        // Clean tuple/set keep their flat layout.
        assert_eq!(assert_roundtrip("(1, 2, 3)", 80), "(1, 2, 3)");
        assert_eq!(assert_roundtrip("#(1, 2)", 80), "#(1, 2)");
    }

    #[test]
    fn a_same_line_trailing_comment_on_a_tuple_elem_is_preserved_not_dropped() {
        // The tuple sibling of the list trailing-comment fix (shared `bracketed_comment_aware` +
        // `print_elem_maybe_commented`). `(…, x // note)` used to DROP the `//` (→ `cdz fmt` refused the
        // file); the tuple parse loop now captures it as `(comment-after …)` and the printer re-emits it
        // same-line, forcing `)` onto its own line so it is not swallowed into the comment.
        let src = "def t() -> Int64 = (1, 2 // last\n)";
        let tree = sexpr::print(&parser::read_ml(src).arenas);
        assert_eq!(
            tree, "(def (t) (: #tuple(1 (comment-after \"last\" 2)) Int64))",
            "the same-line trailing `//` on the last tuple elem is captured, not dropped"
        );
        let printed = print(&parser::read_ml(src).arenas, 80);
        assert_eq!(
            printed, "def t() -> Int64 = (\n  1,\n  2 // last\n)",
            "trailing comment prints same-line; `)` breaks to its own line"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml(&printed).arenas),
            "(def (t) (: #tuple(1 (comment-after \"last\" 2)) Int64))",
            "the trailing tuple comment round-trips"
        );
        // Clean tuples (incl. the 1-tuple `(e,)` and grouping `(e)`) keep their ordinary layout.
        assert_eq!(assert_roundtrip("(1, 2, 3)", 80), "(1, 2, 3)");
        assert_eq!(assert_roundtrip("(42,)", 80), "(42,)");
    }

    #[test]
    fn a_same_line_comment_on_a_non_last_collection_elem_is_not_captured_no_round_trip_break() {
        // PR#758 (Copilot): the list/tuple same-line trailing-comment capture must fire ONLY on the LAST
        // element (gated on the closer being next). A comment on a NON-last element (`[1 // note, 2]`)
        // sits in the `,` token's slot — capturing it there would print `1 // note, 2` with the `, 2`
        // swallowed into the comment line → an INVALID re-parse (a round-trip BREAK, worse than a drop).
        // So a non-last comment is NOT captured; the element is bare and the stranded comment trips the
        // comment-drop guard (fmt refuses — no corruption). Pin that the TREE carries no comment node here
        // (the witness the capture didn't fire) and that the bare elements are intact.
        assert_eq!(
            sexpr::print(&parser::read_ml("def l() -> List(Int64) = [1 // note\n, 2]").arenas),
            "(def (l) (: #list(1 2) (List Int64)))",
            "a comment on a non-last LIST element is not captured (no swallow, no corruption)"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("def t() -> Int64 = (1 // note\n, 2)").arenas),
            "(def (t) (: #tuple(1 2) Int64))",
            "a comment on a non-last TUPLE element is not captured (no swallow, no corruption)"
        );
        // The LAST-element case still IS captured (the two fixes above remain in force).
        assert_eq!(
            sexpr::print(&parser::read_ml("def l() -> List(Int64) = [1, 2 // last\n]").arenas),
            "(def (l) (: #list(1 (comment-after \"last\" 2)) (List Int64)))",
            "a comment on the LAST list element is still captured"
        );
    }

    #[test]
    fn set_literal_round_trips() {
        // `#(…)` is the native set ctor literal `#set(…)` — the third built-in collection surface. It
        // round-trips, and the LEGACY `((. Set of) (list …))` member form still prints back to `#(…)`
        // too (dual-support during the corpus migration; see the oracle assertion below).
        assert_eq!(assert_roundtrip("#(1, 2, 3)", 80), "#(1, 2, 3)");
        assert_eq!(assert_roundtrip("#(x)", 80), "#(x)");
        // Empty set: `#()` is the empty `#set()`, distinct from the empty map `#{}` / list `[]`.
        assert_eq!(assert_roundtrip("#()", 80), "#()");
        // The oracle: the desugared member-access application prints as the `#(…)` surface.
        let a = sexpr::read("((. Set of) (list 1 2 3))").unwrap();
        assert_eq!(print(&a, 80), "#(1, 2, 3)");
        assert_eq!(
            print(&sexpr::read("((. Set of) (list))").unwrap(), 80),
            "#()"
        );
        // Wide sets break all-or-nothing like a list.
        assert_eq!(
            assert_roundtrip("#(1000, 2000, 3000, 4000)", 20),
            "#(\n  1000,\n  2000,\n  3000,\n  4000\n)"
        );
        // A same-line trailing `//` on the LAST set element is preserved (a set `#(…)` desugars to
        // `Set.of([…])`, so its elements are list elements rendered via the shared comment-aware path).
        // Captured as `(comment-after …)`, printed same-line, `)` forced to its own line; round-trips.
        assert_eq!(
            sexpr::print(&parser::read_ml("def s() -> Int64 = #(1, 2 // last\n)").arenas),
            "(def (s) (: #set(1 (comment-after \"last\" 2)) Int64))",
            "a same-line trailing comment on the last set element is captured, not dropped"
        );
        assert_eq!(
            assert_roundtrip("#(1, 2 // last\n)", 80),
            "#(\n  1,\n  2 // last\n)"
        );
    }

    #[test]
    fn set_literal_falls_back_to_call_form() {
        // A shadowed `Set` (or a `Set.of` applied to a non-`list`-literal argument) is NOT a set
        // literal — it renders as the ordinary `Set.of(…)` member call, which round-trips faithfully.
        // A `Set.of` over a computed list (a bare `xs`, not a `list` literal) stays a call.
        assert_eq!(assert_roundtrip("Set.of(xs)", 80), "Set.of(xs)");
        // A shadowed inner `list` alias keeps the call form (the literal gate is `literal_ctor`).
        let shadowed = "let list = f in Set.of(list(1, 2))";
        let out = assert_roundtrip(shadowed, 80);
        assert!(
            out.contains("Set.of(list(1, 2))"),
            "shadowed `list` must stay a call, got {out:?}"
        );
        // A `Set.of` over a `.. rest` spread list has no `#(…)` surface — stays the call form.
        assert_eq!(
            assert_roundtrip("Set.of([1, .. rest])", 80),
            "Set.of([1, .. rest])"
        );
    }

    #[test]
    fn bin_literal_round_trips() {
        // `b[…]` is the surface for the `(bin …)` grammar form — the structured sibling of `b"…"`. It
        // round-trips in both construction and pattern position, and the s-expr oracle sugars `(bin …)`
        // back to `b[…]`.
        assert_eq!(
            assert_roundtrip("b[u16(258), u8(1)]", 80),
            "b[u16(258), u8(1)]"
        );
        assert_eq!(
            assert_roundtrip("b[bits(1, 1), bits(2, 3)]", 80),
            "b[bits(1, 1), bits(2, 3)]"
        );
        assert_eq!(assert_roundtrip("b[]", 80), "b[]");
        // The oracle: a hand-authored `(bin …)` prints as the `b[…]` surface.
        assert_eq!(
            print(&sexpr::read("(bin (u16 258) (u8 1))").unwrap(), 80),
            "b[u16(258), u8(1)]"
        );
        assert_eq!(print(&sexpr::read("(bin)").unwrap(), 80), "b[]");
        // Pattern position: `b[…]` segments are sub-patterns (`u16(n)` binds `n`), and it round-trips.
        assert_eq!(
            assert_roundtrip("match x with | b[u16(n), bytes(rest)] => n", 80),
            "match x with\n  | b[u16(n), bytes(rest)] => n"
        );
        assert_eq!(
            print(&sexpr::read("(match x ((bin (u16 n)) n))").unwrap(), 80),
            "match x with\n  | b[u16(n)] => n"
        );
    }

    #[test]
    fn a_comment_on_a_binary_segment_is_preserved_not_dropped() {
        // A comment on a `b[…]` CONSTRUCTION segment — own-line leading (`b[\n // seg\n u8(1), …]`) or a
        // same-line trailing on the LAST segment (`b[…, u8(2) // n]`) — used to be DROPPED (segments are
        // parsed via `expr`, which doesn't drain the leading slot; the last-segment trailing sat in the
        // `]` slot). bin_form now captures both (leading `(comment …)`, last-segment `(comment-after …)`
        // gated on `at(RBracket)`), and print_bin uses the shared `bracketed_comment_aware`. Same as the
        // list literal. `strip_comments` peels them; compiles to wasm.
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def b() -> Bytes = b[\n  // seg\n  u8(1), u8(2)]").arenas
            ),
            "(def (b) (: (bin (comment \"seg\" (u8 1)) (u8 2)) Bytes))",
            "own-line comment before the first bin segment is captured"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("def b() -> Bytes = b[u8(1), u8(2) // last\n]").arenas),
            "(def (b) (: (bin (u8 1) (comment-after \"last\" (u8 2))) Bytes))",
            "same-line trailing comment on the last bin segment is captured"
        );
        assert_eq!(
            assert_roundtrip("b[\n  // seg\n  u8(1), u8(2)]", 80),
            "b[\n  // seg\n  u8(1),\n  u8(2)\n]"
        );
        // Clean binary literals keep their flat layout.
        assert_eq!(
            assert_roundtrip("b[u16(258), u8(1)]", 80),
            "b[u16(258), u8(1)]"
        );
        assert_eq!(assert_roundtrip("b[]", 80), "b[]");
    }

    #[test]
    fn an_own_line_comment_leading_a_let_binding_is_preserved_not_dropped() {
        // An own-line `//` above a `let` binding (`let\n // note\n x = 1 in …`, or before a `,`-separated
        // later binding) used to be DROPPED — the binding is a `(binder value)` pair, and the leading slot
        // sat unfrained, so `is_let_shape` rejected the `(comment … (n e))`-wrapped binding → the whole
        // `let` fell to the backtick call form. let_expr now captures it (wrap `(comment "text" binding)`),
        // `is_let_shape` peels via `strip_field_comments`, and `print_let` renders the comment above the
        // binding (forcing the bindings to break). `strip_comments` peels it; compiles to wasm.
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def f() -> Int64 = let\n  // note\n  x = 1 in x").arenas
            ),
            "(def (f) (: (let ((comment \"note\" (x 1))) x) Int64))",
            "own-line comment before the first let binding is captured, not dropped"
        );
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def f() -> Int64 = let x = 1,\n  // note\n  y = 2 in x + y")
                    .arenas
            ),
            "(def (f) (: (let ((x 1) (comment \"note\" (y 2))) (+ x y)) Int64))",
            "own-line comment before a non-first let binding is captured"
        );
        // Round-trips + idempotent (the layout is faithful even if the comment renders on the `let` line).
        let src = "def f() -> Int64 = let\n  // note\n  x = 1 in x";
        let printed = print(&parser::read_ml(src).arenas, 80);
        assert_eq!(
            print(&parser::read_ml(&printed).arenas, 80),
            printed,
            "idempotent"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml(&printed).arenas),
            "(def (f) (: (let ((comment \"note\" (x 1))) x) Int64))",
            "the let-binding comment round-trips"
        );
        // Clean lets (incl. multi-binding + pattern binder) keep their layout.
        assert_eq!(assert_roundtrip("let x = 1 in x", 80), "let x = 1 in\nx");
        assert_eq!(
            assert_roundtrip("let x = 1, y = 2 in x + y", 80),
            "let x = 1, y = 2 in\nx + y"
        );
    }

    #[test]
    fn an_own_line_comment_leading_an_if_branch_is_preserved_not_dropped() {
        // An own-line `//` above an `if` sub-expression — the condition (`if\n // note\n c then …`), the
        // then-branch (`if c then\n // note\n t else …`), or the else-branch (`else\n // note\n e`) — used
        // to be DROPPED (each sub-expr is a single `expr`, whose leading slot `if_expr` didn't drain).
        // if_expr now captures + wraps each `(comment "text" expr)`; the printer already renders a leading
        // `(comment …)` on its own line above the expr, so no printer change is needed. `strip_comments`
        // peels it; compiles to wasm. This is the LAST filed comment surface — the `//` surface is now
        // complete across all element/branch positions.
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def f(b: Bool) -> Int64 = if b then\n  // note\n  1 else 2")
                    .arenas
            ),
            "(def (f (: b Bool)) (: (if b (comment \"note\" 1) 2) Int64))",
            "own-line comment before the then-branch is captured, not dropped"
        );
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def f(b: Bool) -> Int64 = if b then 1 else\n  // note\n  2")
                    .arenas
            ),
            "(def (f (: b Bool)) (: (if b 1 (comment \"note\" 2)) Int64))",
            "own-line comment before the else-branch is captured"
        );
        assert_eq!(
            sexpr::print(
                &parser::read_ml("def f(b: Bool) -> Int64 = if\n  // note\n  b then 1 else 2")
                    .arenas
            ),
            "(def (f (: b Bool)) (: (if (comment \"note\" b) 1 2) Int64))",
            "own-line comment before the condition is captured"
        );
        // Round-trips + idempotent.
        let src = "def f(b: Bool) -> Int64 = if b then\n  // note\n  1 else 2";
        let printed = print(&parser::read_ml(src).arenas, 80);
        assert_eq!(
            print(&parser::read_ml(&printed).arenas, 80),
            printed,
            "idempotent"
        );
        // A clean `if` keeps its layout.
        assert_eq!(
            assert_roundtrip("if b then 1 else 2", 80),
            "if b then 1 else 2"
        );
    }

    #[test]
    fn destructuring_binder_patterns_round_trip() {
        // A destructuring PATTERN in a `def`/`fn` parameter or a `let` binder renders through the
        // pattern surface (the inverse of `param`/`let_expr` routing a pattern-opening binder to
        // `pattern`), so it round-trips instead of falling to the garbage generic-call form.
        assert_eq!(
            assert_roundtrip("def f((a, b)) = a + b", 80),
            "def f((a, b)) = a + b"
        );
        assert_eq!(
            assert_roundtrip("def head([x, .. rest]) = x", 80),
            "def head([x, .. rest]) = x"
        );
        // CONSTRUCTOR patterns in a parameter — a single-constructor destructure binds like a tuple
        // (the v-guide-editor issue 2026-07-18): a bare `Ctor(c)`, a qualified `Mod.Ctor(n)`, and a
        // multi-payload / nested one all round-trip through the ML surface (parser gained the ctor-pattern
        // binder route; printer's `pattern` already renders `Ctor(p…)` / `A.B(p…)`).
        assert_eq!(
            assert_roundtrip("def to-f(C(c)) = c", 80),
            "def to-f(C(c)) = c"
        );
        assert_eq!(
            assert_roundtrip("def f(Some(x)) = x", 80),
            "def f(Some(x)) = x"
        );
        assert_eq!(
            assert_roundtrip("def f(Id.Mk(n)) = n", 80),
            "def f(Id.Mk(n)) = n"
        );
        assert_eq!(
            assert_roundtrip("def f(P.Mk(a, b)) = a + b", 80),
            "def f(P.Mk(a, b)) = a + b"
        );
        assert_eq!(
            assert_roundtrip("def f(W.Wrap(Id.Mk(n))) = n", 80),
            "def f(W.Wrap(Id.Mk(n))) = n"
        );
        // `let` binder patterns — tuple, list-rest, and a mix with a plain name.
        assert_eq!(
            assert_roundtrip("let (a, b) = p in a + b", 80),
            "let (a, b) = p in\na + b"
        );
        assert_eq!(
            assert_roundtrip("let [x, .. rest] = ys in x", 80),
            "let [x, .. rest] = ys in\nx"
        );
        assert_eq!(
            assert_roundtrip("let x = 1, (a, b) = p in x + a", 80),
            "let x = 1, (a, b) = p in\nx + a"
        );
        // A multi-binding `let` that does NOT fit breaks CONSISTENTLY: every binding drops to its own
        // line, indented under `let` — not a greedy fill that packs two bindings per line. At width 20
        // the three bindings overflow one line, so each gets its own; the first stays on the `let `
        // line and the rest hang at INDENT under it.
        assert_eq!(
            assert_roundtrip("let aa = 1, bb = 2, cc = 3 in aa + bb + cc", 20),
            "let aa = 1,\n  bb = 2,\n  cc = 3 in\naa + bb + cc"
        );
        // The same bindings that DO fit stay on one line (consistent box prints flat when it fits) —
        // so this is a no-op for a `let` that fits, changing only the overflow layout.
        assert_eq!(
            assert_roundtrip("let aa = 1, bb = 2, cc = 3 in aa + bb + cc", 80),
            "let aa = 1, bb = 2, cc = 3 in\naa + bb + cc"
        );
        // The oracle: a string-headed `(tuple …)` binder pattern from the s-expr surface sugars too.
        assert_eq!(
            print(&sexpr::read("(let (((tuple a b) p)) (+ a b))").unwrap(), 80),
            "let (a, b) = p in\na + b"
        );
        // A CONSTRUCTOR-application pattern binder now sugars to the proper `let Ctor(p…) = v in …`
        // surface (both a bare `Ctor` and a qualified `Mod.Ctor`), the parser having gained the
        // ctor-pattern binder route — a single-constructor destructure binds like a tuple (corpus
        // `(let (((Id.Mk n) …)) …)`), so it round-trips through the ML surface instead of the old
        // backtick-`let` fallback. (v-guide-editor issue 2026-07-18.)
        assert_eq!(
            print(&sexpr::read("(let ((((. Id Mk) n) v)) n)").unwrap(), 80),
            "let Id.Mk(n) = v in\nn"
        );
        assert_eq!(
            print(&sexpr::read("(let (((C c) x)) c)").unwrap(), 80),
            "let C(c) = x in\nc"
        );
        assert_eq!(
            print(
                &sexpr::read("(let (((P.Mk a b) (P.Mk 5 6))) (+ a b))").unwrap(),
                80
            ),
            "let P.Mk(a, b) = P.Mk(5, 6) in\na + b"
        );
        // …and the whole thing round-trips through the ML reader (no backtick fallback).
        for sx in [
            "(let ((((. Id Mk) n) v)) n)",
            "(let (((C c) x)) c)",
            "(let (((P.Mk a b) (P.Mk 5 6))) (+ a b))",
        ] {
            let a = sexpr::read(sx).unwrap();
            let ml = print(&a, 80);
            assert!(
                !ml.contains("`let`"),
                "ctor-pattern let must not backtick-fallback: {ml:?}"
            );
            assert!(
                parser::read_ml(&ml).arenas.structurally_eq(&a),
                "ctor-pattern let round-trips: {ml:?}"
            );
        }
        // RECORD binding patterns — the operator-ruled bare-brace surface `{ field = p, … }` (arena
        // `(record (field p) …)`, distinct from the map `#{…}` = `(map …)`). In a param, a let binder, and
        // a match arm; a PARTIAL pattern (fewer fields) too. (v-guide-editor issue 2026-07-18; unblocked by
        // v-patterns' Increment-B compiler support landing.)
        assert_eq!(
            assert_roundtrip("def f({ x = a, y = b }) = a + b", 80),
            "def f({ x = a, y = b }) = a + b"
        );
        assert_eq!(
            assert_roundtrip("def f({ x = a }) = a", 80),
            "def f({ x = a }) = a"
        );
        assert_eq!(
            assert_roundtrip("let { x = a, y = b } = r in a + b", 80),
            "let { x = a, y = b } = r in\na + b"
        );
        assert_eq!(
            assert_roundtrip("match r with | { x = a } => a", 80),
            "match r with\n  | { x = a } => a"
        );
        // The oracle: a hand-authored `(record …)` binder prints the brace surface (NOT the backtick-let
        // fallback), in both let and param position.
        assert_eq!(
            print(
                &sexpr::read("(def (main) (let (((record (= x a) (= y b)) r)) (+ a b)))").unwrap(),
                80
            ),
            "def main() =\n  let { x = a, y = b } = r in\n  a + b"
        );
        // Both a record PATTERN binder AND a record VALUE are the canonical `(= name value)` triple
        // (path B — full symmetry; operator ruling): patterns and literals spell the identical form.
        for sx in [
            "(def (main) (let (((record (= x a) (= y b)) #record((= x 3) (= y 4)))) (+ a b)))",
            "(def (f (record (= x a))) a)",
        ] {
            let a = sexpr::read(sx).unwrap();
            let ml = print(&a, 80);
            assert!(
                !ml.contains("`let`"),
                "record pattern must not backtick-fallback: {ml:?}"
            );
            assert!(
                parser::read_ml(&ml).arenas.structurally_eq(&a),
                "record pattern round-trips: {ml:?}"
            );
        }
        // An EMPTY record pattern `{}` (arena `(record)`, binds nothing) renders `{}` (no inner padding)
        // in BOTH param and let-binder position — consistent, no backtick fallback — and round-trips.
        assert_eq!(
            print(&sexpr::read("(def (f (record)) a)").unwrap(), 80),
            "def f({}) = a"
        );
        assert_eq!(
            print(
                &sexpr::read("(def (main) (let (((record) v)) v))").unwrap(),
                80
            ),
            "def main() =\n  let {} = v in\n  v"
        );
        // NESTED compositions of the binding patterns (tuple/list/map/record/ctor) — the interaction of
        // the record + ctor pattern surfaces with each other and the pre-existing tuple/list/map ones is
        // where a printer/parser asymmetry would most likely hide, so pin the mixes: a tuple inside a
        // record, a ctor inside a tuple / record / map-value, a record inside a ctor / list-rest / record.
        // Assert structural round-trip (the property; layout varies by position) with no backtick fallback.
        for src in [
            "def f({ x = (a, b) }) = a",
            "def f((C(c), d)) = c",
            "def f({ p = C(c) }) = c",
            "def f([{ x = a }, .. rest]) = a",
            "def f(Some({ x = a })) = a",
            "def f(#{ k = C(c) }) = c",
            "let (C(a), { y = b }) = p in a",
            "let { outer = { inner = a } } = r in a",
        ] {
            let a = parser::read_ml(src).arenas;
            let ml = print(&a, 80);
            assert!(
                !ml.contains('`'),
                "nested pattern must not backtick-fallback: {src} -> {ml:?}"
            );
            assert!(
                parser::read_ml(&ml).arenas.structurally_eq(&a),
                "nested pattern round-trips: {src} -> {ml:?}"
            );
        }
    }

    #[test]
    fn type_ascription_round_trips() {
        // `e : T` -> arena (: e T); ascription binds loosest, so it wraps the whole expression.
        assert_eq!(assert_roundtrip("42 : Int64", 80), "42 : Int64");
        assert_eq!(assert_roundtrip("2 + 2 : Int64", 80), "2 + 2 : Int64");
        // the arena head is `:`, matching the s-expr surface.
        let a = sexpr::read("(: 42 Int64)").unwrap();
        assert_eq!(print(&a, 80), "42 : Int64");
        // a compound value/type ascription (the corpus's common output shape).
        assert_eq!(
            assert_roundtrip("(1, 2) : (Int64, Int64)", 80),
            "(1, 2) : (Int64, Int64)"
        );
    }

    #[test]
    fn equality_is_double_equals() {
        // `==` on the surface builds arena head `=` (matching the s-expr corpus) and prints back `==`.
        assert_eq!(assert_roundtrip("a == b", 80), "a == b");
        let a = sexpr::read("(= a b)").unwrap();
        assert_eq!(print(&a, 80), "a == b");
        // a lone `=` is only the binding separator, never equality.
        assert_eq!(
            assert_roundtrip("let x = 1 in x == 1", 80),
            "let x = 1 in\nx == 1"
        );
    }

    #[test]
    fn empty_literals() {
        // Empty list and map from the s-expr surface render as `[]` / `#{}`. The compound-value
        // constructor primitive is a STRING head (`("list")`, `("map")`), not a name — a bare `(list)`
        // name head is an ordinary application, so the literal round-trip uses the string form.
        assert_eq!(print(&sexpr::read("(\"list\")").unwrap(), 80), "[]");
        assert_eq!(print(&sexpr::read("(\"map\")").unwrap(), 80), "#{}");
    }

    #[test]
    fn literals_break_all_or_nothing_when_wide() {
        let out = assert_roundtrip(
            "{ name = \"alice\", scores = [90, 85, 95], active = true }",
            30,
        );
        assert_eq!(
            out,
            "{\n  name = \"alice\",\n  scores = [90, 85, 95],\n  active = true\n}"
        );
    }

    #[test]
    fn paren_grouping_is_not_a_tuple() {
        // `(1 + 2) * 3` — the parens are transparent grouping, NOT a 1-tuple.
        assert_eq!(assert_roundtrip("(1 + 2) * 3", 80), "(1 + 2) * 3");
    }

    #[test]
    fn name_headed_literals_sugar_like_string_heads() {
        // A hand-authored NAME-headed literal (the whole `.sexp` corpus's shape) renders to its
        // surface form, identical to the reader's STRING-headed primitive — an unshadowed `record`/
        // `tuple`/`list`/`map` is the shadowable alias for the same constructor.
        assert_eq!(
            print(&sexpr::read("(record (x 1) (y 2))").unwrap(), 80),
            "{ x = 1, y = 2 }"
        );
        assert_eq!(
            print(&sexpr::read("(tuple 1 2 3)").unwrap(), 80),
            "(1, 2, 3)"
        );
        assert_eq!(
            print(&sexpr::read("(list 1 2 3)").unwrap(), 80),
            "[1, 2, 3]"
        );
        assert_eq!(
            print(&sexpr::read("(map (a 1) (b 2))").unwrap(), 80),
            "#{ a = 1, b = 2 }"
        );
        // a 1-tuple keeps its trailing comma so it re-reads as a 1-tuple, not grouping.
        assert_eq!(print(&sexpr::read("(tuple 1)").unwrap(), 80), "(1,)");
        // empty name-headed forms mirror the string-headed empties (`()` is unit, so no empty tuple).
        assert_eq!(print(&sexpr::read("(list)").unwrap(), 80), "[]");
        assert_eq!(print(&sexpr::read("(record)").unwrap(), 80), "{}");
        assert_eq!(print(&sexpr::read("(map)").unwrap(), 80), "#{}");
    }

    // (Removed `name_headed_literal_round_trips_via_head_normalization`: it pinned the Name↔native head
    // NORMALIZATION that M2 ruling (ii) removes — read_ml now produces native heads end-to-end, so a
    // legacy name-headed input no longer cross-surface round-trips; the native surface is covered by the
    // native-form tests. See DESIGN-native-ast-compound-data.md / v-ast-compound M2.)

    #[test]
    fn record_field_shorthand() {
        // A value-record field is the canonical `(= name value)` triple (RV1/RV2, Phase B). `{ x }`
        // puns to `(record (= x x))`; the printer renders a same-name field back as `{ x }`. The printer
        // also tolerates the legacy `(x x)` pair (prints `{ x }` too) so a stray un-migrated node still
        // renders.
        assert_eq!(
            print(&sexpr::read("(record (= x x))").unwrap(), 80),
            "{ x }"
        );
        assert_eq!(print(&sexpr::read("(record (x x))").unwrap(), 80), "{ x }"); // legacy pair tolerated
        assert_eq!(
            print(&sexpr::read("(record (= x x) (= y 2))").unwrap(), 80),
            "{ x, y = 2 }"
        );
        // a non-punned field keeps `name = value`.
        assert_eq!(
            print(&sexpr::read("(record (= x 1))").unwrap(), 80),
            "{ x = 1 }"
        );
        // parse `{ x }` → the pun as the canonical `(= x x)` triple (a STRING-headed record primitive,
        // per the reader's literal desugar).
        assert_eq!(
            sexpr::print(&parser::read_ml("{ x }").arenas),
            "#record((= x x))"
        );
        assert_eq!(assert_roundtrip("{ x }", 80), "{ x }");
        assert_eq!(assert_roundtrip("{ x = x }", 80), "{ x }");
        assert_eq!(assert_roundtrip("{ x, y = 2 }", 80), "{ x, y = 2 }");
    }

    #[test]
    fn export_brace_surface() {
        // `(export name…)` renders as a brace name group; `export { … }` parses back to it.
        assert_eq!(
            print(&sexpr::read("(export main)").unwrap(), 80),
            "export { main }"
        );
        assert_eq!(
            print(&sexpr::read("(export main helper)").unwrap(), 80),
            "export { main, helper }"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("export { main, helper }").arenas),
            "(export main helper)"
        );
        assert_eq!(
            assert_roundtrip("export { main, helper }", 80),
            "export { main, helper }"
        );
    }

    #[test]
    fn export_constructor_and_wildcard_surface() {
        // Opaque/abstract types: an export element may be a type's CONSTRUCTOR `(. T A)` or the
        // WILDCARD `(. T *)` (the whole constructor set), alongside bare names. Each renders inside the
        // `export { … }` brace group as `T.A` / `T.*`, and parses back to the same member-access form.
        assert_eq!(
            print(&sexpr::read("(export (. Color *))").unwrap(), 80),
            "export { Color.* }"
        );
        assert_eq!(
            print(&sexpr::read("(export (. Color Red) main)").unwrap(), 80),
            "export { Color.Red, main }"
        );
        // Round-trip both directions: `Color.*` reads back to `(. Color *)`.
        assert_eq!(
            sexpr::print(&parser::read_ml("export { Color.* }").arenas),
            "(export (. Color *))"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("export { Color.Red, main }").arenas),
            "(export (. Color Red) main)"
        );
        assert_eq!(
            assert_roundtrip("export { Color.* }", 80),
            "export { Color.* }"
        );
        assert_eq!(
            assert_roundtrip("export { Color.Red, main }", 80),
            "export { Color.Red, main }"
        );
    }

    #[test]
    fn member_wildcard_reads_and_prints() {
        // `T.*` as a bare member access (the segment the export surface reuses) parses to `(. T *)`
        // and prints back — `*` is a reserved final member segment, distinct from the `*` multiply op
        // (which needs an operand before it).
        assert_eq!(
            sexpr::print(&parser::read_ml("Color.*").arenas),
            "(. Color *)"
        );
        assert_eq!(print(&sexpr::read("(. Color *)").unwrap(), 80), "Color.*");
        // Multiply is untouched — `a * b` stays the `*` operator, not a member access.
        assert_eq!(sexpr::print(&parser::read_ml("a * b").arenas), "(* a b)");
    }

    #[test]
    fn import_brace_from_surface() {
        // `(import "path" (name…))` renders `import { name, … } from "path"`; parses back to it.
        assert_eq!(
            print(&sexpr::read("(import \"lib\" (helper))").unwrap(), 80),
            "import { helper } from \"lib\""
        );
        assert_eq!(
            print(&sexpr::read("(import \"lib\" (helper other))").unwrap(), 80),
            "import { helper, other } from \"lib\""
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("import { helper, other } from \"lib\"").arenas),
            "(import \"lib\" (helper other))"
        );
        assert_eq!(
            assert_roundtrip("import { helper, other } from \"lib\"", 80),
            "import { helper, other } from \"lib\""
        );
        // `from` is contextual — still usable as an ordinary name elsewhere.
        assert_eq!(
            assert_roundtrip("let from = 5 in\nfrom", 80),
            "let from = 5 in\nfrom"
        );
    }

    #[test]
    fn shadowed_ctor_name_stays_a_call() {
        // A binder for a ctor name makes every NAME-headed occurrence of it (tree-wide) a user
        // application — rendered as a call, NOT sugared. This is the head-position shadow the
        // resolver honours; the printer must not misrepresent it as a literal.
        assert_eq!(
            assert_roundtrip("let list = fn(a, b) => a + b in\nlist(3, 4)", 80),
            "let list = fn(a, b) => a + b in\nlist(3, 4)"
        );
        // shadowed via a def parameter: the `tuple` argument, not a tuple literal.
        let a = sexpr::read("(def (f tuple) (tuple 3 4))").unwrap();
        assert_eq!(print(&a, 80), "def f(tuple) = tuple(3, 4)");
        // an unshadowed sibling in the SAME tree still sugars (shadow is per-name, not all-ctors).
        let a = sexpr::read("(let ((list (fn (a) a))) (tuple (list 1) 2))").unwrap();
        assert_eq!(print(&a, 80), "let list = fn(a) => a in\n(list(1), 2)");
    }

    #[test]
    fn record_key_must_be_a_name_else_falls_back() {
        // A `record` whose field key is not a plain name can't use the `{…}` surface; it falls back
        // to the generic call form and still round-trips.
        let a = sexpr::read("(record (1 v))").unwrap();
        let printed = print(&a, 80);
        // generic form: `record` applied to the field-list `(1 v)`, which prints as `1(v)`.
        assert_eq!(printed, "record(1(v))");
        assert!(parser::read_ml(&printed).arenas.structurally_eq(&a));
    }

    #[test]
    fn documented_def_prints_doc_line() {
        // A def carrying `(doc …)` forms renders them as `/// …` lines above the `def`.
        let a = sexpr::read("(def (f x) (doc \"hi\") (+ x 1))").unwrap();
        let printed = print(&a, 80);
        assert_eq!(printed, "/// hi\ndef f(x) = x + 1");
        let b = parser::read_ml(&printed);
        assert!(
            b.ok() && b.arenas.structurally_eq(&a),
            "printed:\n{printed}"
        );
    }

    #[test]
    fn non_doc_multi_form_def_falls_back() {
        // A def whose extra body form is NOT a doc (here a `(: type)` annotation) has no dedicated
        // surface; it falls back to the generic call form and still round-trips. `def` is now a
        // reserved keyword, so as a bare call head it is backtick-escaped (`` `def` ``).
        let a = sexpr::read("(def (f x) (: Int64) (+ x 1))").unwrap();
        let printed = print(&a, 80);
        assert_eq!(printed, "`def`(f(x), `:`(Int64), x + 1)");
        let b = parser::read_ml(&printed);
        assert!(
            b.ok() && b.arenas.structurally_eq(&a),
            "printed:\n{printed}"
        );
    }

    #[test]
    fn doc_and_comment_round_trip() {
        // `///` doc attaches inside the def; `//` comment wraps it.
        assert_eq!(
            assert_roundtrip("/// Adds.\ndef add(a, b) = a + b", 80),
            "/// Adds.\ndef add(a, b) = a + b"
        );
        assert_eq!(
            assert_roundtrip("// note\ndef main() = 42", 80),
            "// note\ndef main() = 42"
        );
    }

    #[test]
    fn comment_leading_a_def_body_is_preserved_not_dropped() {
        // A `//` line-comment on its own line at the START of a def body (the interior
        // body-leading position, `def f() =` \n `// note` \n `body`) is captured + wrapped as a
        // `(comment …)`, not DROPPED. `expr` doesn't drain trivia (only `stmt` does), so before the
        // `body_expr` fix the comment was stranded at the body's first-token slot and vanished
        // entirely — a genuine comment LOSS (worse than a downgrade). A structural round-trip can't
        // witness a dropped comment; the count-based assert pins it. Covers function + value defs.
        for (src, label) in [
            (
                "def f() -> Int64 =\n  // interior body comment\n  1",
                "function body",
            ),
            ("def x =\n  // value body note\n  42", "value body"),
        ] {
            let a = parser::read_ml(src);
            assert!(a.ok(), "[{label}] parse: {:?}", a.errors);
            let sexpr = crate::sexpr::print(&a.arenas);
            assert_eq!(
                sexpr.matches("(comment ").count(),
                1,
                "[{label}] the body `//` must be a `(comment …)` node, not dropped: {sexpr}"
            );
            let printed = print(&a.arenas, 100);
            let comment_lines = printed
                .lines()
                .filter(|l| l.trim_start().starts_with("//"))
                .count();
            assert_eq!(
                comment_lines, 1,
                "[{label}] the body comment re-prints as one `//` line: {printed}"
            );
            // Idempotent across a second pass.
            let b = parser::read_ml(&printed);
            assert!(b.ok(), "[{label}] reparse: {:?}", b.errors);
            assert_eq!(print(&b.arenas, 100), printed, "[{label}] not idempotent");
        }
        // CONTROL: a `//` between two top-level defs still round-trips (the statement-leading position
        // `stmt` already handled — the fix must not disturb it).
        let ctrl = "def a() -> Int64 = 1\n// between defs\ndef b() -> Int64 = 2";
        let a = parser::read_ml(ctrl);
        assert!(a.ok(), "[control] parse: {:?}", a.errors);
        assert_eq!(
            crate::sexpr::print(&a.arenas).matches("(comment ").count(),
            1,
            "[control] the between-defs comment survives"
        );
    }

    #[test]
    fn file_header_doc_before_a_non_documentable_form_becomes_a_module_doc() {
        // A `///` file header before a NON-documentable form (an `import` — not a def/type/effect/module
        // that drains its own docs) is preserved as a top-level `(module-doc …)` node, re-printing as
        // `///` — NOT downgraded to `//`. (Before the module-doc node, `stmt`'s leftover-doc path wrapped
        // it as `(comment …)` → `//`, the file-header doc-loss that blocked ~56 files from fmt-apply.)
        let src = "/// Header one.\n/// Header two.\nimport { x } from \"dep\"\ndef f() = 1";
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse: {:?}", a.errors);
        let sexpr = crate::sexpr::print(&a.arenas);
        assert_eq!(
            sexpr.matches("(module-doc ").count(),
            2,
            "both header lines are `(module-doc …)` nodes: {sexpr}"
        );
        assert_eq!(
            sexpr.matches("(comment ").count(),
            0,
            "no header line downgraded to `(comment …)`: {sexpr}"
        );
        // They re-print as `///` (count preserved), and the whole thing is idempotent + structurally
        // round-trips (a `(module-doc)` re-reads to the same node).
        let printed = print(&a.arenas, 100);
        let doc_lines = printed
            .lines()
            .filter(|l| l.trim_start().starts_with("///"))
            .count();
        let comment_lines = printed
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with("//") && !t.starts_with("///")
            })
            .count();
        assert_eq!(doc_lines, 2, "both headers re-print as `///`: {printed}");
        assert_eq!(comment_lines, 0, "no header downgraded to `//`: {printed}");
        let b = parser::read_ml(&printed);
        assert!(b.ok(), "reparse: {:?}", b.errors);
        assert_eq!(print(&b.arenas, 100), printed, "not idempotent");
        assert_eq!(
            crate::sexpr::print(&b.arenas)
                .matches("(module-doc ")
                .count(),
            2,
            "the `(module-doc)` nodes survive the round-trip"
        );
    }

    #[test]
    fn doc_before_effect_stays_a_doc_not_downgraded_to_comment() {
        // A `///` doc before an `effect` decl attaches INSIDE the decl as a `(doc …)` node (mirroring
        // `def`/`type`/`module`), so it re-prints as `///` — NOT downgraded to `//`. The reader used
        // to leave the docs in the statement slot, where `stmt` wrapped them as `(comment …)` and the
        // printer faithfully rendered them `//`, silently losing the doc marker on `cdz fmt`. A
        // structural round-trip does NOT catch this (a `(comment …)` still round-trips structurally);
        // the count-based assert below is the witness that pins the doc-vs-comment distinction.
        let src = "/// Diagnostics.\n/// Two lines.\neffect Diag = | emit : Int64 -> Unit | collect : -> List(Int64)";
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse: {:?}", a.errors);
        // Reader promotes both `///` runs to `(doc …)`, not `(comment …)`.
        let sexpr = crate::sexpr::print(&a.arenas);
        assert_eq!(
            sexpr.matches("(doc ").count(),
            2,
            "both `///` lines should be `(doc …)` nodes: {sexpr}"
        );
        assert_eq!(
            sexpr.matches("(comment ").count(),
            0,
            "no `///` should be downgraded to a `(comment …)`: {sexpr}"
        );
        // And they re-print as `///`, preserved across fmt — count-preserving is the invariant a
        // structural round-trip misses. Check per-line (a `///` line contains a `//` substring, so
        // count LINES whose trimmed start is a doc `///` vs a plain `//` comment).
        let printed = print(&a.arenas, 100);
        let doc_lines = printed
            .lines()
            .filter(|l| l.trim_start().starts_with("///"))
            .count();
        let comment_lines = printed
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with("//") && !t.starts_with("///")
            })
            .count();
        assert_eq!(doc_lines, 2, "both doc lines re-print as `///`: {printed}");
        assert_eq!(
            comment_lines, 0,
            "no doc line downgraded to `//`: {printed}"
        );
        // Idempotent: re-reading the printed form and re-printing is byte-identical.
        let b = parser::read_ml(&printed);
        assert!(b.ok(), "reparse: {:?}", b.errors);
        assert_eq!(print(&b.arenas, 100), printed, "not idempotent");
    }

    #[test]
    fn doc_before_annotated_def_stays_a_doc_not_downgraded_to_comment() {
        // A `///` doc before an `@`-ANNOTATED def belongs to the def below the annotation, so the
        // reader CARRIES it across the `@name` onto the def's slot, where def_expr drains it into a
        // `(doc …)`. Without the carry the docs sat at the `@` slot (unseen by def_expr, which runs
        // after `@name`) and `stmt` downgraded them to a `(comment …)` — the annotated-def doc-loss
        // bug (dense `/// section-divider` files before `@test` defs lost most of their docs on fmt).
        // Verified against a plain-def control (already worked) + a STACKED-annotation case.
        for (src, label) in [
            (
                "/// Doc before annotated def.\n@test\ndef b() -> Bool = true",
                "single annotation",
            ),
            (
                "/// Stacked doc.\n@inline-always\n@test\ndef c() -> Bool = true",
                "stacked annotations",
            ),
            (
                "/// Tagged.\n@tag(\"slow\")\ndef d() -> Int64 = 5",
                "call-style annotation",
            ),
            // The carry deposits at the wrapped-form slot, so EVERY documentable form that drains
            // (type/effect/module, not just def) preserves a doc before its annotation. Pin them so a
            // future change to any of those forms' doc-drain can't silently regress the annotated case.
            (
                "/// Annotated type.\n@derive\ntype Color = | Red | Green",
                "annotated type",
            ),
            (
                "/// Annotated effect.\n@handler\neffect E = | op : -> Unit",
                "annotated effect",
            ),
            (
                "/// Annotated module.\n@inline\nmodule M { def x() -> Int64 = 1 }",
                "annotated module",
            ),
        ] {
            let a = parser::read_ml(src);
            assert!(a.ok(), "[{label}] parse: {:?}", a.errors);
            let sexpr = crate::sexpr::print(&a.arenas);
            assert_eq!(
                sexpr.matches("(doc ").count(),
                1,
                "[{label}] the `///` should be a `(doc …)` node: {sexpr}"
            );
            assert_eq!(
                sexpr.matches("(comment ").count(),
                0,
                "[{label}] no `///` downgraded to `(comment …)`: {sexpr}"
            );
            let printed = print(&a.arenas, 100);
            let doc_lines = printed
                .lines()
                .filter(|l| l.trim_start().starts_with("///"))
                .count();
            let comment_lines = printed
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("//") && !t.starts_with("///")
                })
                .count();
            assert_eq!(doc_lines, 1, "[{label}] doc re-prints as `///`: {printed}");
            assert_eq!(comment_lines, 0, "[{label}] no `//` downgrade: {printed}");
            // Idempotent across a second fmt pass.
            let b = parser::read_ml(&printed);
            assert!(b.ok(), "[{label}] reparse: {:?}", b.errors);
            assert_eq!(print(&b.arenas, 100), printed, "[{label}] not idempotent");
        }
    }

    #[test]
    fn doc_above_an_annotated_def_prints_above_the_annotation_not_between() {
        // A `/// header` the user wrote ABOVE an `@`-annotation must re-print ABOVE the annotation, NOT
        // BETWEEN the annotation and its def. The reader carries the doc INSIDE the def (`carry_docs`),
        // so a naive printer emits `@test` \n `/// header` \n `def` — moving a section header below its
        // annotation (the annotation-comment adjacency the frontend is touchy about; v-cad/v-cdz-tooling
        // report). The printer hoists the def's leading docs above the `@`. Arena is unchanged (a
        // print-POSITION fix), so it stays round-trip-safe.
        for (src, label) in [
            (
                "/// header\n@test\ndef t() -> Bool = true",
                "single annotation",
            ),
            (
                "/// h1\n/// h2\n@test\ndef t() -> Bool = true",
                "multi-line doc",
            ),
            (
                "/// header\n@inline-always\n@test\ndef t() -> Bool = true",
                "stacked annotations",
            ),
        ] {
            let a = parser::read_ml(src);
            assert!(a.ok(), "[{label}] parse: {:?}", a.errors);
            let printed = print(&a.arenas, 100);
            // The FIRST non-blank line must be the doc (`///`), not an `@` annotation.
            let first = printed.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            assert!(
                first.trim_start().starts_with("///"),
                "[{label}] doc must be the FIRST line (above the annotation); got:\n{printed}"
            );
            // No `///` line may appear AFTER an `@` line (the reorder bug's signature).
            let mut seen_at = false;
            for l in printed.lines() {
                let t = l.trim_start();
                if t.starts_with('@') {
                    seen_at = true;
                }
                assert!(
                    !(seen_at && t.starts_with("///")),
                    "[{label}] a `///` appears BELOW an `@` (reordered):\n{printed}"
                );
            }
            // Idempotent + round-trip-safe (arena unchanged): re-read → re-print is byte-identical.
            let b = parser::read_ml(&printed);
            assert!(b.ok(), "[{label}] reparse: {:?}", b.errors);
            assert_eq!(print(&b.arenas, 100), printed, "[{label}] not idempotent");
        }
    }

    #[test]
    fn a_comment_trailing_a_sum_variant_is_preserved_same_line() {
        // A `//` comment on the SAME line as a sum-type variant (`| Ctor(T) // note`) is captured as a
        // `(comment-after "note" (Ctor T))` node and re-prints SAME-LINE after the variant — NOT dropped
        // (the trailing-inline loss) NOR moved to its own line. The reader records same-line-ness (a span
        // check) and attaches the comment to the variant it FOLLOWS. rcdzc's `strip_comments` peels the
        // `comment-after` head (same as `comment`), so the type still scans. Count-based: the `//` count
        // is preserved, and the arena carries a `(comment-after …)` that round-trips.
        let src = "type E =\n  | Mismatch(Int64) // a code\n  | NotImpl";
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse: {:?}", a.errors);
        let sexpr = crate::sexpr::print(&a.arenas);
        assert_eq!(
            sexpr.matches("(comment-after ").count(),
            1,
            "the trailing `//` becomes a `(comment-after …)` node: {sexpr}"
        );
        let printed = print(&a.arenas, 100);
        // The comment re-prints on the SAME line as its variant (not its own line), and is not dropped.
        assert!(
            printed
                .lines()
                .any(|l| l.contains("Mismatch(Int64)") && l.contains("// a code")),
            "the `//` trails its variant same-line: {printed}"
        );
        assert_eq!(
            printed.matches("// a code").count(),
            1,
            "comment preserved once: {printed}"
        );
        // Idempotent + round-trips (the `(comment-after …)` survives a re-read).
        let b = parser::read_ml(&printed);
        assert!(b.ok(), "reparse: {:?}", b.errors);
        assert_eq!(print(&b.arenas, 100), printed, "not idempotent");
        assert_eq!(
            crate::sexpr::print(&b.arenas)
                .matches("(comment-after ")
                .count(),
            1,
            "the `(comment-after …)` survives the round-trip"
        );
    }

    #[test]
    fn a_comment_trailing_a_match_arm_is_preserved_same_line() {
        // A `//` on the same line as a NON-LAST match arm (`| pat => body // note`) is captured as a
        // `(comment-after "note" (pat body))` node and re-prints same-line after the body — not dropped
        // nor moved. (The FILE-FINAL last-arm case falls back to the pre-existing leading-comment reorder,
        // count-preserving; here we pin the common non-last case + a following statement makes the last
        // arm's comment attach too.) `strip_comments` peels it so the match still compiles.
        let src =
            "def f(x) =\n  match x with\n  | 0 => 1 // zero\n  | _ => 2 // other\ndef g() = 9";
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse: {:?}", a.errors);
        let sexpr = crate::sexpr::print(&a.arenas);
        assert_eq!(
            sexpr.matches("(comment-after ").count(),
            2,
            "both arm comments become `(comment-after …)` (the last arm's too, since a def follows): {sexpr}"
        );
        let printed = print(&a.arenas, 100);
        assert!(
            printed
                .lines()
                .any(|l| l.contains("=> 1") && l.contains("// zero")),
            "first arm comment trails same-line: {printed}"
        );
        assert!(
            printed
                .lines()
                .any(|l| l.contains("=> 2") && l.contains("// other")),
            "last arm comment trails same-line: {printed}"
        );
        // Idempotent + round-trips.
        let b = parser::read_ml(&printed);
        assert!(b.ok(), "reparse: {:?}", b.errors);
        assert_eq!(print(&b.arenas, 100), printed, "not idempotent");
        assert_eq!(
            crate::sexpr::print(&b.arenas)
                .matches("(comment-after ")
                .count(),
            2,
            "the `(comment-after …)` arm-comments survive the round-trip"
        );
    }

    #[test]
    fn a_trailing_comment_on_an_if_then_branch_round_trips_not_dropped() {
        // A same-line `//` after an `if`'s THEN branch (`if a then 1 // note` before `else`) was DROPPED
        // — the reader didn't capture a trailing comment in that mid-expression slot, so `cdz fmt`
        // refused (the comment-attachment gap that blocked hm-collect.cdz). Now captured as
        // `(comment-after "note" 1)` on the then-branch + re-printed same-line, with `else` forced to the
        // next line (a `//` runs to EOL, so `else` can't share the line). assert_roundtrip pins re-parse
        // + idempotence.
        let out = assert_roundtrip("def f(a) = if a then 1 // note\nelse 2", 100);
        assert!(
            out.lines().any(|l| l.contains("1 // note")),
            "the then-branch trailing comment re-prints same-line: {out}"
        );
        assert!(
            out.lines().any(|l| l.trim_start().starts_with("else")),
            "`else` drops to its own line after the // (not swallowed into the comment): {out}"
        );
    }

    #[test]
    fn a_trailing_comment_after_let_in_round_trips_not_dropped() {
        // A same-line `//` after `in` (`let x = a in // note` before the body) was DROPPED. Now captured
        // as a `(comment-after "note" binds)` wrapper + re-printed after `in`; the body's own hardbreak
        // drops it to the next line so the `//` can't swallow it.
        let out = assert_roundtrip("def f(a) = let x = a in // note\nx + 1", 100);
        assert!(
            out.lines().any(|l| l.contains("in // note")),
            "the `in` trailing comment re-prints same-line after `in`: {out}"
        );
    }

    #[test]
    fn own_line_comments_leading_match_arm_body_and_let_value_and_let_body_round_trip() {
        // Own-line `//` comments in three more mid-expression leading slots the reader previously dropped
        // (so `cdz fmt` refused): leading a MATCH-ARM BODY (`=> <newline> // note <newline> body`),
        // leading a LET-BINDING VALUE (`let y = <newline> // note <newline> value`), and leading the LET
        // BODY (`in <newline> // note <newline> body`). Each is now captured `(comment "text" …)` +
        // printed own-line above its expr. assert_roundtrip pins re-parse + idempotence for each.
        let arm = assert_roundtrip(
            "def f(x) = match x with\n  | A() =>\n// note\n1\n  | _ => 2",
            100,
        );
        assert!(
            arm.contains("// note"),
            "match-arm-body leading comment preserved: {arm}"
        );
        let val = assert_roundtrip("def f(x) = let y =\n// vnote\nx + 1 in\n  y", 100);
        assert!(
            val.contains("// vnote"),
            "let-value leading comment preserved: {val}"
        );
        let body = assert_roundtrip("def f(x) = let y = x in\n  // bnote\ny + 1", 100);
        assert!(
            body.contains("// bnote"),
            "let-body leading comment preserved: {body}"
        );
    }

    #[test]
    fn an_own_line_comment_before_then_round_trips_not_dropped() {
        // An OWN-LINE `//` sitting BEFORE the `then` keyword (`if c` ⏎ `// note` ⏎ `then t`) was in the
        // `then` token's leading slot and dropped when `expect_keyword` consumed past it (the last
        // comment-attachment position blocking `cdz fmt` on the operator's hm-collect.cdz). Now captured +
        // folded into the then-branch's leading comments — symmetric with the before-`else` capture.
        let out = assert_roundtrip("def f(c) = if c\n// note\nthen 1 else 2", 100);
        assert!(
            out.contains("// note"),
            "the own-line comment before `then` is preserved: {out}"
        );
    }

    #[test]
    fn an_own_line_comment_before_else_round_trips_not_dropped() {
        // An OWN-LINE `//` sitting BEFORE the `else` keyword (`if a then 1` ⏎ `// note` ⏎ `else 2`) was
        // in the `else` token's leading slot and dropped when `expect_keyword` consumed past it. Now
        // captured + folded into the else-branch's leading comments (prints own-line above the else).
        let out = assert_roundtrip("def f(a) = if a then 1\n// note\nelse 2", 100);
        assert!(
            out.contains("// note"),
            "the own-line comment before `else` is preserved: {out}"
        );
    }

    #[test]
    fn an_own_line_comment_leading_a_match_arm_is_preserved_not_dropped() {
        // An own-line `//` above a match arm (`match x with\n  // note\n  | 0 => …`) used to be DROPPED
        // (it sat in the arm's `|`/pattern leading slot, which the arm loop didn't drain → the whole match
        // fell to the generic call form). match_expr now drains it (before the `|` bump) and wraps the arm
        // `(comment "text" (pat body))`; `is_match_shape` unwraps via `strip_field_comments` and
        // `print_match` renders the comment as a `// …` line above the arm's `| `. Own-line, no swallow
        // hazard → captured on any arm. `strip_comments` peels it (compiles to wasm).
        assert_eq!(
            sexpr::print(
                &parser::read_ml(
                    "def f(x: Int64) -> Int64 = match x with\n  // note\n  | 0 => 0\n  | _ => 1"
                )
                .arenas
            ),
            "(def (f (: x Int64)) (: (match x (comment \"note\" (0 0)) (_ 1)) Int64))",
            "own-line comment before the first arm is captured, not dropped"
        );
        // Before a NON-first arm too:
        assert_eq!(
            sexpr::print(
                &parser::read_ml(
                    "def f(x: Int64) -> Int64 = match x with\n  | 0 => 0\n  // mid\n  | _ => 1"
                )
                .arenas
            ),
            "(def (f (: x Int64)) (: (match x (0 0) (comment \"mid\" (_ 1))) Int64))",
            "own-line comment before a non-first arm is captured"
        );
        // Renders above the `| ` and round-trips (idempotent).
        let src = "def f(x: Int64) -> Int64 = match x with\n  // note\n  | 0 => 0\n  | _ => 1";
        let printed = print(&parser::read_ml(src).arenas, 80);
        assert!(
            printed.contains("with\n  // note\n  | 0 =>"),
            "leading comment prints on its own line above the arm: {printed}"
        );
        assert_eq!(
            print(&parser::read_ml(&printed).arenas, 80),
            printed,
            "idempotent"
        );
        // Leading + trailing on one arm compose (nesting normalizes idempotently, nothing dropped).
        let combo =
            "def f(x: Int64) -> Int64 = match x with\n  // lead\n  | 0 => 0 // t\n  | _ => 1";
        let p1 = print(&parser::read_ml(combo).arenas, 80);
        assert_eq!(
            print(&parser::read_ml(&p1).arenas, 80),
            p1,
            "lead+trail idempotent"
        );
        assert!(
            p1.contains("// lead") && p1.contains("// t"),
            "both the leading and trailing arm comments survive: {p1}"
        );
    }

    #[test]
    fn an_own_line_comment_leading_a_type_variant_is_preserved_not_dropped() {
        // An own-line `//` above a sum-type variant (`type T =\n  // note\n  | A\n  | B`) used to be
        // DROPPED (it sat in the variant's `|` leading slot, which the variant loop didn't drain →
        // `is_type_shape` rejected the wrapped variant → the type fell to the backtick call form).
        // type_expr now drains it before the `|` (like match arms) and wraps `(comment "text" variant)`;
        // `is_type_shape` unwraps via `strip_field_comments` and `print_type` renders it above the `| `.
        // Distinct from a leading `///` DOC header (a `(doc)` on the whole decl). Own-line, no swallow
        // hazard → any variant. `strip_comments` peels it; compiles to wasm.
        assert_eq!(
            sexpr::print(&parser::read_ml("type T =\n  // note\n  | A\n  | B").arenas),
            "(type T (comment \"note\" A) B)",
            "own-line comment before the first variant is captured, not dropped"
        );
        assert_eq!(
            sexpr::print(&parser::read_ml("type T =\n  | A\n  // mid\n  | B").arenas),
            "(type T A (comment \"mid\" B))",
            "own-line comment before a non-first variant is captured"
        );
        // Renders above the `| ` and round-trips (idempotent).
        let src = "type T =\n  // note\n  | A\n  | B";
        let printed = print(&parser::read_ml(src).arenas, 80);
        assert_eq!(
            printed, "type T =\n  // note\n  | A\n  | B",
            "leading comment prints on its own line above the variant"
        );
        assert_eq!(
            print(&parser::read_ml(&printed).arenas, 80),
            printed,
            "idempotent"
        );
        // A leading `///` DOC header (a `(doc)` on the decl) is DISTINCT and unchanged.
        assert_eq!(
            sexpr::print(&parser::read_ml("/// doc\ntype T = | A | B").arenas),
            "(type T (doc \"doc\") A B)",
            "a leading /// doc header stays a (doc) on the decl, not a variant comment"
        );
    }

    #[test]
    fn minimal_parens() {
        // precedence: * binds tighter than +, so no parens; but (1 + 2) * 3 needs them
        assert_eq!(assert_roundtrip("(1 + 2) * 3", 80), "(1 + 2) * 3");
        assert_eq!(assert_roundtrip("1 + 2 * 3", 80), "1 + 2 * 3");
    }

    #[test]
    fn match_always_one_arm_per_line() {
        // Arms go one per line even at a WIDE width where they would fit on one line — the OCaml/Rust
        // convention, never packed. Each is led by `| ` (including the first); `match … with` header.
        let out = assert_roundtrip("match e with | Some(n) => n | None => 0 | _ => neg", 200);
        assert_eq!(
            out, "match e with\n  | Some(n) => n\n  | None => 0\n  | _ => neg",
            "got:\n{out}"
        );
    }

    #[test]
    fn call_breaks_all_args_when_wide() {
        let out = assert_roundtrip("some-function(alpha, beta, gamma, delta, epsilon)", 20);
        assert!(out.starts_with("some-function(\n"), "got:\n{out}");
    }

    #[test]
    fn non_last_match_arm_if_let_body_prints_bare_not_parenthesized() {
        // A NON-LAST match-arm body that is an `if`/`let`/`fn` (whose trailing sub-expression ends in a
        // closing token, not an open `|`-arm list) is delimited by the arm's own `|` and must NOT be
        // parenthesized — the pervasive `(if …)`/`(let …)` match-arm parens the operator flagged
        // (hm-collect.cdz). `assert_roundtrip` proves the bare form re-parses identically + is idempotent.
        let out = assert_roundtrip(
            "match x with | A => if x > 0 then 1 else 2 | B => let y = x in y + 1 | C => 9",
            200,
        );
        assert!(
            out.contains("| A => if x > 0 then 1 else 2"),
            "a non-last `if` arm body prints bare (no wrapping parens), got:\n{out}"
        );
        assert!(
            // The multi-line `let` body breaks to its own indented line under `=>` (operator follow-on),
            // still BARE (no wrapping parens).
            out.contains("| B =>\n    let y = x in"),
            "a non-last `let` arm body prints bare (no wrapping parens), got:\n{out}"
        );
        assert!(
            !out.contains("=> (if") && !out.contains("=> (let"),
            "no redundant `( … )` wrapping an if/let arm body, got:\n{out}"
        );
    }

    #[test]
    fn non_last_match_arm_nested_match_body_keeps_parens_so_it_round_trips() {
        // The CORRECTNESS boundary of the above: a non-last arm body whose TRAILING sub-expression IS an
        // open `|`-arm list (a nested `match`/`handle`, possibly under `if`-else / `let`-body / `@`) MUST
        // parenthesize — else the following `| pat` is absorbed into that inner arm list on re-parse. Two
        // cases: a bare nested match, and an `if` whose `else` is a match (tail-reachable arm form).
        // The INPUT must parenthesize the nested match (else `| B` binds to the INNER match — that
        // ambiguity is exactly why the printer must re-emit the parens); the printer keeps them.
        let nested = assert_roundtrip(
            "match x with | A => (match x with | C => 1 | _ => 2) | B => 3",
            200,
        );
        assert!(
            // Parenthesized arm body (operator seq-95): `=> (` on the arm line, body indented, `)`
            // dedented; the nested match KEEPS its parens.
            nested.contains("=> (\n    match x with"),
            "a non-last nested-match arm body keeps its parens, got:\n{nested}"
        );
        // `if` whose else-tail is a match — must wrap (the else-match would swallow the next `|`).
        let if_else_match = assert_roundtrip(
            "match x with | A => if p then 1 else (match x with | C => 2 | _ => 3) | B => 9",
            200,
        );
        assert!(
            if_else_match.contains("=> (\n    if p then"),
            "a non-last `if` whose else-tail is a match keeps parens, got:\n{if_else_match}"
        );
    }

    #[test]
    fn a_bitwise_or_arm_body_parenthesizes_so_the_bare_pipe_does_not_start_a_new_arm() {
        // breaker's pipe-in-arm round-trip bug: a match/handle arm body that is a top-level bitwise-or
        // `(| a b)` prints with a bare `|` glyph (`a | b`); at the arm's own level a `|` TERMINATES the
        // arm (`Parser::arm_bar_terminates`), so a bare-`|` body re-parses as the next arm's separator and
        // the right operand dangles. The printer must parenthesize it — `=> (x | 8)` — for EVERY arm
        // (the last/only arm too). Build the arm arena directly (the ML surface can't author a bare `|`
        // body). handle + match, and the resume/call arg case (already parser-safe) for good measure.
        // A single-arm handle whose body is `(| x 8)`.
        let a = sexpr::read("(handle E n ((tag (x) s (| x 8))) (E.tag 3))").unwrap();
        let out = print(&a, 200);
        assert!(
            out.contains("=> (x | 8)"),
            "a bitwise-or handle-arm body parenthesizes the bare `|`, got:\n{out}"
        );
        // A match arm whose body is `(| x 8)`.
        let m = sexpr::read("(match v ((C (x)) (| x 8)) (_ 0))").unwrap();
        let mout = print(&m, 200);
        assert!(
            mout.contains("=> (x | 8)"),
            "a bitwise-or match-arm body parenthesizes the bare `|`, got:\n{mout}"
        );
        // Round-trip: both re-parse identically (the whole point — without the parens the `| 8` / `| _`
        // would be swallowed as a phantom next arm).
        assert_roundtrip("match v with | C(x) => (x | 8) | _ => 0", 200);
        // The nested-operand shape breaker's bw4 used: `(| (& x 15) (<< (& s 3) 4))`.
        let nested = sexpr::read(
            "(handle E n ((tag (x) s (resume (| (& x 15) (<< (& s 3) 4)) (+ s 1)))) (E.tag 3))",
        )
        .unwrap();
        let nout = print(&nested, 200);
        let re = parser::read_ml(&nout);
        assert!(
            re.ok(),
            "a resume(| …, …) arm body round-trips (call args clear the arm-bar flag): {nout:?} -> {:?}",
            re.errors
        );
    }

    #[test]
    fn reserved_name_backtick_escaped() {
        // A name literally "let" must print backtick-escaped so it round-trips as a name.
        let a = sexpr::read("(f let)").unwrap(); // s-expr: `let` is an ordinary atom here
        let printed = print(&a, 80);
        assert!(printed.contains("`let`"), "got: {printed}");
        // and it re-parses to the same arena
        let b = parser::read_ml(&printed);
        assert!(b.ok());
    }

    #[test]
    fn operator_name_backtick_escaped() {
        // `(+ )` used as a bare name (not infix, wrong arity) prints escaped.
        let a = sexpr::read("(list + -)").unwrap();
        let printed = print(&a, 80);
        // + and - as ordinary list elements -> backtick-escaped names
        assert!(
            printed.contains("`+`") && printed.contains("`-`"),
            "got: {printed}"
        );
    }

    #[test]
    fn emit_name_round_trips_through_the_lexer_over_generated_names() {
        // The NAME round-trip law, swept: `emit_name(s)` prints a name so it RE-LEXES back to `s` — either
        // a bare `Ident` (when bare-safe) or a backtick-escaped `` `…` `` (when reserved / an operator
        // glyph / otherwise not bare-safe), whose `BacktickName` token `unescape_backtick_name` decodes to
        // `s`. This is the printer↔lexer inverse pair for names (the analogue of the int/float/string/char
        // sweeps): a printer that emitted an under-escaped name (a bare `+` or `let`, or a `` ` ``/`\`
        // inside a backtick name left unescaped) would re-lex to a DIFFERENT name or a wrong token — a
        // silent identifier corruption. Sweep names over an alphabet rich in the escape-significant chars
        // (backtick, backslash, operator glyphs, reserved-word letters, `#`/`.`/quote, unicode), asserting
        // `emit_name(s)` lexes to exactly ONE non-trivia token that recovers `s`.
        let alphabet: &[char] = &[
            '`', '\\', '+', '-', '*', '/', '<', '>',
            '=', // operator glyphs + the two escape chars
            'l', 'e', 't', 'i', 'f', 'n', // reserved-word letters (let/if/in/fn)
            'a', 'Z', '0', '9', '_', '-', // ordinary ident chars
            '#', '.', '"', ' ', // sigils/space (never bare-safe)
            'λ', '中', // multi-byte unicode
        ];
        let mut rng = SplitMix64(0xba7c_c0de_11a3_e501);
        let mut backtick_seen = 0usize;
        for _ in 0..30_000 {
            // A non-empty name of 1..=6 chars.
            let len = 1 + (rng.next() as usize) % 6;
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            let printed = emit_name(&s);
            // `emit_name` output must lex to EXACTLY ONE non-trivia token spanning all of it, and that
            // token must recover `s`. (A bare-safe name → Ident text == s; otherwise → BacktickName that
            // `unescape_backtick_name` decodes to s.)
            let mut toks = crate::lexer::Lexer::new(&printed).filter(|t| !t.kind.is_trivia());
            match (toks.next(), toks.next()) {
                (Some(t), None) => {
                    assert_eq!(
                        t.span.start, 0,
                        "emit_name({s:?})={printed:?} did not lex from offset 0"
                    );
                    assert_eq!(
                        t.span.end,
                        printed.len(),
                        "emit_name({s:?})={printed:?} lexed only part of the output"
                    );
                    let recovered = match t.kind {
                        Kind::BacktickName => {
                            backtick_seen += 1;
                            literal::unescape_backtick_name(&printed[t.span.start..t.span.end])
                        }
                        // A bare-safe name lexes as an ordinary Ident (its text IS the name).
                        Kind::Ident => printed[t.span.start..t.span.end].to_string(),
                        other => panic!(
                            "emit_name({s:?})={printed:?} lexed as {other:?}, not Ident/BacktickName"
                        ),
                    };
                    assert_eq!(
                        recovered, s,
                        "emit_name → lex → recover is not the identity for {s:?} (printed {printed:?})"
                    );
                }
                (first, second) => panic!(
                    "emit_name({s:?})={printed:?} did not lex to exactly one token: {first:?}, {second:?}"
                ),
            }
        }
        // Exercise the backtick-escape path DETERMINISTICALLY (not on generator luck) on names that MUST
        // escape: a reserved word, an operator glyph, and a name containing the two escape chars.
        for s in [
            "let",
            "+",
            "->",
            "a`b",
            "x\\y",
            "with space",
            "#hash",
            "a.b",
        ] {
            let printed = emit_name(s);
            let mut toks = crate::lexer::Lexer::new(&printed).filter(|t| !t.kind.is_trivia());
            let (Some(t), None) = (toks.next(), toks.next()) else {
                panic!("emit_name({s:?})={printed:?} must lex to one token");
            };
            assert_eq!(
                t.kind,
                Kind::BacktickName,
                "{s:?} must escape to a backtick name: {printed:?}"
            );
            assert_eq!(
                literal::unescape_backtick_name(&printed[t.span.start..t.span.end]),
                s,
                "backtick round-trip is not the identity for {s:?} (printed {printed:?})"
            );
        }
        let _ = backtick_seen; // a soft coverage hint, not asserted (couples to the alphabet)
    }

    #[test]
    fn guarded_arm_prints_if() {
        let out = assert_roundtrip("match n with | x if x < 0 => neg | _ => pos", 80);
        assert!(out.contains("x if x < 0 =>"), "got: {out}");
    }

    #[test]
    fn nested_if_condition_parenthesizes() {
        // A conditional as another's CONDITION parenthesizes so `if if … ` never appears.
        assert_eq!(
            assert_roundtrip("if (if a then b else c) then 1 else 2", 80),
            "if (if a then b else c) then 1 else 2"
        );
        // but an `else if` chain (a conditional in BRANCH position) stays bare.
        assert_eq!(
            assert_roundtrip("if a then 1 else if b then 2 else 3", 80),
            "if a then 1 else if b then 2 else 3"
        );
    }

    #[test]
    fn wide_if_breaks_condition_on_if_line() {
        // Too wide for one line: the condition stays on the `if` line, branches drop to indented
        // lines, `else` dedents to the `if` column.
        let out = assert_roundtrip(
            "if some-condition then some-then-value(1, 2, 3) else some-else-value(4, 5, 6)",
            40,
        );
        assert_eq!(
            out,
            "if some-condition then\n  some-then-value(1, 2, 3)\nelse\n  some-else-value(4, 5, 6)"
        );
    }

    #[test]
    fn tuple_patterns_use_paren_sugar() {
        // A `(tuple …)` pattern with 2+ elements prints as `(p, …)`, matching the value tuple.
        assert_eq!(
            assert_roundtrip("match p with | (a, b) => a + b | _ => 0", 80),
            "match p with\n  | (a, b) => a + b\n  | _ => 0"
        );
        // nested, and inside a constructor.
        let a = sexpr::read("(match p ((tuple a (tuple b c)) 9) (_ 0))").unwrap();
        assert!(
            print(&a, 80).contains("(a, (b, c)) =>"),
            "got: {}",
            print(&a, 80)
        );
        let a = sexpr::read("(match p ((Some (tuple a b)) 1) (_ 0))").unwrap();
        assert!(
            print(&a, 80).contains("Some((a, b)) =>"),
            "got: {}",
            print(&a, 80)
        );
        // a 1-element tuple pattern prints `(a,)` (trailing comma), re-reading as a 1-tuple not `(a)`.
        let a = sexpr::read("(match p ((tuple a) a) (_ 0))").unwrap();
        assert!(print(&a, 80).contains("(a,) =>"), "got: {}", print(&a, 80));
    }

    #[test]
    fn rest_operator_round_trips_across_collections() {
        // `..` is the ONE rest/spread marker, uniform across list/map in BOTH construction and
        // pattern position. It re-reads as itself (`.. rest`), never the old `` `..` `` name escape.

        // --- construction spread ---
        assert_eq!(assert_roundtrip("[1, 2, .. rest]", 80), "[1, 2, .. rest]");
        assert_eq!(assert_roundtrip("[.. rest]", 80), "[.. rest]");
        assert_eq!(
            assert_roundtrip("#{ 1 = 10, .. rest }", 80),
            "#{ 1 = 10, .. rest }"
        );
        // MULTIPLE + INTERIOR spreads in ONE construction — the operator's `[a, b, ..c, d, ..e]` shape.
        // `rest_marker` runs per element in `list_literal`, so a spread may appear at ANY position and more
        // than once (unlike a PATTERN rest, which is tail-only); the surface round-trips all of them. (The
        // COMPILER lowering of a construction spread is a separate slice — this pins the surface alone.)
        assert_eq!(
            assert_roundtrip("[a, b, .. c, d, .. e]", 80),
            "[a, b, .. c, d, .. e]"
        );
        assert_eq!(assert_roundtrip("[.. a, .. b]", 80), "[.. a, .. b]");
        // A spread of a nested list literal.
        assert_eq!(
            assert_roundtrip("[1, .. [2, 3], 4]", 80),
            "[1, .. [2, 3], 4]"
        );
        // RECORD construction spread (`{ ..base, a = 1 }`) — the record twin; interior + multiple spreads.
        assert_eq!(
            assert_roundtrip("{ .. base, a = 1 }", 80),
            "{ .. base, a = 1 }"
        );
        assert_eq!(
            assert_roundtrip("{ a = 1, .. b, c = 2, .. d }", 80),
            "{ a = 1, .. b, c = 2, .. d }"
        );
        // SET construction spread (`#(..a, x)`) — the set twin; leading + interior spreads.
        assert_eq!(assert_roundtrip("#(.. a, x)", 80), "#(.. a, x)");
        assert_eq!(
            assert_roundtrip("#(1, .. a, 2, .. b)", 80),
            "#(1, .. a, 2, .. b)"
        );

        // --- pattern (list) ---
        assert_eq!(
            assert_roundtrip("match xs with | [] => 0 | [x, .. rest] => x", 80),
            "match xs with\n  | [] => 0\n  | [x, .. rest] => x"
        );
        // a catch-all rest with no leading binders.
        assert_eq!(
            assert_roundtrip("match xs with | [.. all] => 7", 80),
            "match xs with\n  | [.. all] => 7"
        );

        // --- pattern (map) ---
        assert_eq!(
            assert_roundtrip("match m with | #{ 1 = v, .. rest } => v | _ => 0", 80),
            "match m with\n  | #{ 1 = v, .. rest } => v\n  | _ => 0"
        );

        // --- the s-expr surface is the oracle: the flat `… ".." rest` shape prints as `.. rest`,
        //     the SAME shape the compiler's list/map lowering scans for (no arena change). ---
        let a = sexpr::read("(match xs ((list x .. rest) x))").unwrap();
        assert!(
            print(&a, 80).contains("[x, .. rest] =>"),
            "got: {}",
            print(&a, 80)
        );
        let a = sexpr::read("(match m ((map (1 v) .. rest) v) (_ 0))").unwrap();
        assert!(
            print(&a, 80).contains("#{ 1 = v, .. rest } =>"),
            "got: {}",
            print(&a, 80)
        );
        let a = sexpr::read("(list 1 2 .. rest)").unwrap();
        assert_eq!(print(&a, 80), "[1, 2, .. rest]");
        // multiple/interior construction spreads via the flat s-expr oracle.
        let a = sexpr::read("(list a b .. c d .. e)").unwrap();
        assert_eq!(print(&a, 80), "[a, b, .. c, d, .. e]");
    }

    #[test]
    fn exact_numbers_print_and_reparse() {
        // Hex is canonicalized to lowercase digits (case is not preserved in the leaf); the value
        // and base round-trip, and the printed form is idempotent.
        assert_eq!(assert_roundtrip("0x2A", 80), "0x2a");
        assert_eq!(assert_roundtrip("0x2a", 80), "0x2a");
        assert_eq!(assert_roundtrip("1.5", 80), "1.5");
        assert_eq!(assert_roundtrip("1000000", 80), "1000000");
        assert_eq!(assert_roundtrip("0b1010", 80), "0b1010");
    }

    #[test]
    fn ml_has_no_rational_literal_slash_is_int64_division() {
        // seq-204: the operator DROPPED the `r` rational glyph, and there is NO bare `<num>/<den>` rational
        // literal on the ML surface — unspaced `3/2` is Int64 integer division `(/ 3 2)`, identical to the
        // spaced `3 / 2`. (A rational VALUE node still renders `num/den` — see the sexpr-sourced
        // `display_surface_renders_values_readably`; ML SOURCE reaches a rational via `(/ n d)`-style
        // construction, never a scalar literal, per the operator's "native value, no sugar/desugar".)
        let spaced = assert_roundtrip("3 / 2", 80);
        let unspaced = assert_roundtrip("3/2", 80);
        assert_eq!(
            spaced, unspaced,
            "unspaced `3/2` is the same Int64 division as `3 / 2`, not a rational literal"
        );
    }

    #[test]
    fn forall_param_binder_desugars_and_round_trips() {
        // `forall a b. TYPE` in a PARAMETER annotation is INPUT-ONLY sugar (like the brace-record `{…}`):
        // it DESUGARS at parse time to leading `(: a Type)` params, so it PRINTS BACK as `a: Type` (the
        // canonical form), NOT `forall`. The ML→sexpr→ML round-trip is idempotent at that canonical form.
        // (A standalone `(forall …)` node that is NOT hoisted still prints as `forall a. T` — the printer
        // arm from increment 1 — but a def-param forall lowers here.)
        assert_eq!(
            assert_roundtrip("def id(x: forall a. a) = x", 80),
            "def id(a: Type, x: a) = x"
        );
        assert_eq!(
            assert_roundtrip("def apply(f: forall a b. a -> b, x: a) = f(x)", 80),
            "def apply(a: Type, b: Type, f: a -> b, x: a) = f(x)"
        );
        // Idempotent: the desugared form re-prints identically (the sugar is gone after the first parse).
        assert_eq!(
            assert_roundtrip("def id(a: Type, x: a) = x", 80),
            "def id(a: Type, x: a) = x"
        );
        // A forall in a RETURN-TYPE position (`-> forall a. …`) is NOT hoisted (it is not a parameter
        // annotation) — it stays a `(forall …)` node in the return ascription and prints back verbatim
        // via increment 1's printer arm. This round-trips (confirming the earlier "return-type follow-up"
        // concern was a false alarm — it was a wrong `fn(x) -> x` test spelling; the lambda arrow is `=>`).
        assert_eq!(
            assert_roundtrip("def mk() -> forall a. a -> a = fn(x) => x", 80),
            "def mk() -> forall a. a -> a = fn(x) => x"
        );
    }

    #[test]
    fn def_sig_leading_forall_prints_as_canonical_type_params() {
        // The P1 ergonomic spelling `def forall a b. f(…)` is also INPUT-ONLY sugar: it desugars to leading
        // `(: a Type)` params at parse time, so it PRINTS BACK as `f(a: Type, …)` (the canonical form), not
        // `def forall`. Round-trip is idempotent at that canonical form.
        assert_eq!(
            assert_roundtrip("def forall a. id(x: a) = x", 80),
            "def id(a: Type, x: a) = x"
        );
        assert_eq!(
            assert_roundtrip("def forall a b. apply(f: a -> b, x: a) = f(x)", 80),
            "def apply(a: Type, b: Type, f: a -> b, x: a) = f(x)"
        );
    }
}
