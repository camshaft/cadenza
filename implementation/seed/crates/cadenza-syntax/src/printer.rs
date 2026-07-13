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
use crate::token::{self, Kind, PREC_ARROW, PREC_MEMBER, infix_glyph, infix_prec};

/// Indentation per box level (spaces). A layout choice, not a contract.
const INDENT: isize = 2;

/// The parent-precedence to print a subexpression at when a block-form there (`if`/`let`/`match`)
/// must parenthesize but an infix operator must NOT. Block forms parenthesize when `parent_prec > 0`;
/// the lowest infix precedence is 1 and infixes parenthesize only when `prec < parent_prec`, so a
/// value of 1 forces `(if …)`/`(let …)`/`(match …)` while leaving every infix chain bare. Used for an
/// `if` condition, so a nested conditional condition reads as `if (if …) then …`.
const PREC_KEYWORD: u8 = 1;

/// Pretty-print `arenas` to ML text targeting `width` columns.
pub fn print(arenas: &Arenas, width: usize) -> String {
    let mut p = Printer {
        a: arenas,
        doc: Doc::new(),
        shadowed_ctors: shadowed_ctors(arenas),
    };
    // A `do` at the ROOT is the program's top-level form sequence — print its forms BARE (blank-line
    // separated), not wrapped in `do { … }`. A nested `do` (reached via `expr`) keeps the block form.
    if let Some(forms) = p.a.as_form(arenas.root, "do")
        && !forms.is_empty()
    {
        let forms = forms.to_vec();
        p.print_root_forms(&forms);
    } else {
        p.expr(arenas.root, 0);
    }
    p.doc.render(width)
}

/// The default target width (100 columns).
pub const DEFAULT_WIDTH: usize = 100;

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
}

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
                let items = items.clone();
                self.list(&items, parent_prec);
            }
        }
    }

    fn leaf(&mut self, leaf: &Leaf) {
        match leaf {
            Leaf::Int { value, radix } => self.doc.word(literal::render_int(value, *radix)),
            Leaf::Float(d) => self.doc.word(literal::render_decimal(d)),
            Leaf::Bool(b) => self.doc.word(if *b { "true" } else { "false" }),
            Leaf::Str(s) => self.doc.word(format!("\"{}\"", literal::escape_string(s))),
            Leaf::Bytes(b) => self.doc.word(format!("b\"{}\"", literal::escape_bytes(b))),
            Leaf::Name(n) => self.doc.word(emit_name(n)),
            // A bad-escape MARKER round-trips back to `"\<c>"` so the printed form re-reads to the same
            // marker (the defect survives the round-trip rather than being silently lost).
            Leaf::BadEscape(c) => self.doc.word(format!("\"\\{c}\"")),
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
            match ctor.as_str() {
                "list" => return self.print_list_literal(args),
                "tuple" if !args.is_empty() => return self.print_tuple(args),
                "record" if self.is_record_shape(args) => return self.print_record(args),
                "map" if self.is_map_shape(args) => return self.print_map(args),
                _ => {}
            }
        }
        // A head that is an Atom(Name) may name a construct or an operator; otherwise it is a
        // computed-callee application.
        let head = self.head_name(items[0]);
        let args = &items[1..];

        if let Some(head) = head {
            // ---- function type `(-> A B)` -> `A -> B` (right-associative) ----
            if head == "->" && args.len() == 2 {
                return self.arrow(args[0], args[1], parent_prec);
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
                "module" if self.is_module_shape(args) => return self.print_module(args),
                "export" if self.is_export_shape(args) => return self.print_export(args),
                "import" if self.is_import_shape(args) => return self.print_import(args),
                // The compound-value literals (`list`/`tuple`/`record`/`map`) are STRING-headed now and
                // handled by the `head_ctor` dispatch above — a NAME head of the same spelling is an
                // ordinary application of the shadowable alias (or a user binding), rendered as a call.
                // A `(comment "text" node)` wraps a node in ANY position, so render it as `// text`
                // above the node wherever it appears. A `(doc …)`, by contrast, is only a `///`
                // line in a def/module BODY position (handled by print_def/print_module); a stray
                // `(doc …)` elsewhere falls through to the generic call form.
                "comment" if args.len() == 2 && self.is_string(args[0]) => {
                    return self.print_comment(args[0], args[1]);
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
        if !args.is_empty() && self.is_huggable_arg(args[args.len() - 1]) {
            self.hug_call(args);
        } else {
            self.plain_call(args);
        }
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
    fn infix(&mut self, op: &str, prec: u8, l: StructId, r: StructId, parent_prec: u8) {
        let paren = prec < parent_prec;
        // Collect the flat chain: descend the left spine while the operator has the SAME precedence.
        // Result is operands `[o0, o1, …]` and the operators `[op1, …]` between them.
        let mut operands = vec![r];
        let mut ops = vec![op.to_string()];
        let mut left = l;
        loop {
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
        // first operand (its left child, if any, already bound at this prec)
        self.expr(operands[0], prec);
        for (i, o) in ops.iter().enumerate() {
            self.doc.space(); // break BEFORE the operator
            // In infix position the operator prints as its SURFACE GLYPH (the arena head `=` for
            // equality prints as `==`; every other op is identity). The backtick escape is only for
            // an operator glyph used as an ordinary NAME.
            self.doc.word(infix_glyph(o).to_string());
            self.doc.word(" ");
            self.expr(operands[i + 1], prec + 1); // right operand binds one tighter
        }
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
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
        // bindings
        self.doc.ibox(INDENT);
        if let Struct::List(binds) = self.a.get(args[0]) {
            let binds = binds.clone();
            for (i, &b) in binds.iter().enumerate() {
                if i > 0 {
                    self.doc.word(",");
                    self.doc.space();
                }
                if let Struct::List(pair) = self.a.get(b) {
                    let (n, e) = (pair[0], pair[1]);
                    self.expr(n, 0);
                    self.doc.word(" = ");
                    self.value(e);
                }
            }
        }
        self.doc.end();
        self.doc.word(" in");
        // The body starts a new line at the `let`'s own column (offset 0), so a `let … in` chain
        // reads as a flat sequence — the ML idiom for a pervasive `let … in`.
        self.doc.hardbreak();
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
        self.doc.word("if ");
        // Condition at PREC_KEYWORD so a nested block-form condition (`if`/`let`/`match`)
        // parenthesizes — `if (if a then b else c) then …` rather than the unreadable `if if a …`.
        self.expr(args[0], PREC_KEYWORD);
        self.doc.word(" then");
        // Branches at 0: a nested `if` here does NOT parenthesize, so an `else if` chain stays the
        // idiomatic `… else if c then …` (indentation, not parens, disambiguates). A breakable space
        // keeps `then t` on the line when it fits, else drops `t` to an indented line; `else` dedents
        // back to the `if` column.
        self.doc.space();
        self.expr(args[1], 0);
        self.doc.break_with(1, -INDENT);
        self.doc.word("else");
        self.doc.space();
        self.expr(args[2], 0);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// A function BODY that is a top-level type ascription `(: inner R)` denotes a RETURN TYPE: it is
    /// the shape `def f(x) -> R = inner` and `fn(x) -> R => inner` desugar to. Returns `(inner, R)` so
    /// the printer can put the `-> R` back in signature position (round-tripping the surface form),
    /// leaving `inner` as the printed body. Any other body has no return type.
    fn return_type(&self, body: StructId) -> Option<(StructId, StructId)> {
        let t = self.a.as_form(body, ":")?;
        if t.len() == 2 {
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
        self.doc.word("fn(");
        if let Struct::List(params) = self.a.get(args[0]) {
            let params = params.clone();
            for (i, &p) in params.iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.print_param(p);
            }
        }
        self.doc.word(")");
        self.print_return_type(ret_ty);
        self.doc.word(" =>");
        // A block-like body hugs the `=>` (breaks internally); a plain body drops to an indented
        // line if it overflows — same discipline as a def's `=` body.
        self.body_after_eq(body);
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
            self.doc.word("(");
            for (i, &p) in sig[1..].iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.print_param(p);
            }
            self.doc.word(")");
            self.print_return_type(ret_ty);
            self.doc.word(" =");
        }
        self.body_after_eq(body);
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
        self.body_after_eq(value);
        self.doc.end();
    }

    /// Emit a def's leading `(doc "…")` forms as `/// …` lines, each followed by a hardbreak. Shared
    /// by the function and value def printers.
    fn print_def_docs(&mut self, docs: &[StructId]) {
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

    /// `(comment "text" node)` -> `// text` on its own line, then the annotated node beneath it.
    fn print_comment(&mut self, text: StructId, node: StructId) {
        self.doc.cbox(0);
        self.doc.word(format!("//{}", self.doc_line_text(text)));
        self.doc.hardbreak();
        self.expr(node, 0);
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

    /// Emit ` body` after a `=`/`in`-style keyword: a block-like body (a `match`, `let`, `if`, …
    /// that manages its own multi-line layout) HUGS the `=` — a plain space keeps it on the line so
    /// it breaks internally (`fn f(x) = match … {` … ). A plain-expression body uses a breakable
    /// space so a long flat expression instead drops to an indented line (`fn f(x) =\n  a + b + …`).
    fn body_after_eq(&mut self, body: StructId) {
        if self.is_block_body(body) {
            // Hug: a plain space keeps the block on the `=` line; it breaks internally at its own
            // indentation (the def box is at offset 0, so no extra level is added).
            self.doc.word(" ");
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
    fn print_do_stmts(&mut self, stmts: &[StructId]) {
        for (i, &s) in stmts.iter().enumerate() {
            if i > 0 {
                self.doc.hardbreak();
            }
            self.print_stmt(s);
            if i + 1 < stmts.len() {
                self.doc.word(";");
            }
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

    /// The program root's top-level form sequence, printed bare (no wrapper) — the root counterpart of
    /// a nested `do`, using the same `;` sequencing so the construct reads identically everywhere:
    /// each form on its own line, `;` after every form except the last. A `(doc …)` form renders as
    /// its `///` line (no `;`).
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
            self.expr(form, 0);
            if i + 1 < forms.len() {
                self.doc.word(";");
            }
        }
        self.doc.end();
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
        i > 0 && self.is_doc(forms[i - 1])
    }

    /// `module name { form… }` — one member per line (consistent box) when broken, blank-separated
    /// so definitions don't cram together. The first member breaks straight off the `{` (no leading
    /// blank inside the braces); a `///` doc line hugs the member it documents.
    fn print_module(&mut self, args: &[StructId]) {
        let members = &args[1..];
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
        let variants = &args[docs_end..];
        self.doc.cbox(INDENT);
        for &d in docs {
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
        self.doc.word("type ");
        self.expr(name, 0);
        self.doc.word(" =");
        // Each variant on its own line, led by `| ` (always, including the first) — symmetric with a
        // `match`'s `|`-led arms. The `|` is the surface separator between the structural variant
        // entries, never a node in the tree.
        for &v in variants {
            self.doc.hardbreak();
            self.doc.word("| ");
            self.print_variant(v);
        }
        self.doc.end();
    }

    /// One sum-type variant: a nullary `Ctor` (a `Name` atom) prints as itself; a payload variant
    /// `(Ctor T …)` prints as `Ctor(T, …)` — the same shape as a constructor application.
    fn print_variant(&mut self, id: StructId) {
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
            // a nullary variant (bare name atom), or a defensive fallback for an odd shape
            _ => self.expr(id, 0),
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
            && variants.iter().all(|&v| match self.a.get(v) {
                // nullary: a bare constructor name
                Struct::Atom(_) => self.head_name(v).is_some(),
                // payload: (Ctor T …) with a name head and at least one payload type
                Struct::List(items) => items.len() >= 2 && self.head_name(items[0]).is_some(),
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

    /// `[e, …]`.
    fn print_list_literal(&mut self, elems: &[StructId]) {
        self.bracketed("[", "]", false, elems, |p, e| p.expr(e, 0));
    }

    /// `(e, …)` — a tuple. A 1-element tuple prints as `(e,)` (a trailing comma, Rust-style), which
    /// distinguishes it from `(e)` transparent grouping; 2+ elements print `(e, f, …)`.
    fn print_tuple(&mut self, elems: &[StructId]) {
        if elems.len() == 1 {
            self.doc.word("(");
            self.expr(elems[0], 0);
            self.doc.word(",)");
            return;
        }
        self.bracketed("(", ")", false, elems, |p, e| p.expr(e, 0));
    }

    /// `{ name = e, … }`, with field SHORTHAND: a field whose value is a bare-name reference to the
    /// field's own name (`(x x)`) prints as just `{ x }` (the inverse of the reader's `{ x }` → `(x x)`
    /// pun). A field with any other value prints the full `name = value`.
    fn print_record(&mut self, fields: &[StructId]) {
        self.bracketed("{", "}", true, fields, |p, field| {
            if let Struct::List(pair) = p.a.get(field) {
                let (name, value) = (pair[0], pair[1]);
                p.expr(name, 0);
                if !p.is_field_pun(name, value) {
                    p.doc.word(" = ");
                    p.expr(value, 0);
                }
            }
        });
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
        self.bracketed("{", "}", true, names, |p, name| p.expr(name, 0));
    }

    /// An `(export name…)` the `export { … }` surface handles: at least one arg, every arg a bare
    /// name. A malformed export (a non-name arg) falls back to the generic call form so it round-trips.
    fn is_export_shape(&self, args: &[StructId]) -> bool {
        !args.is_empty() && args.iter().all(|&a| self.a.as_name(a).is_some())
    }

    /// An `(import "path" (name…))` the `import { … } from "path"` surface handles: a string path, a
    /// name-LIST of bare names. Any other shape (e.g. the alias form `(import "path" alias)`) falls
    /// back to the generic call form so it still round-trips.
    fn is_import_shape(&self, args: &[StructId]) -> bool {
        args.len() == 2
            && self.is_string(args[0])
            && matches!(self.a.get(args[1]), Struct::List(names)
                if !names.is_empty() && names.iter().all(|&n| self.a.as_name(n).is_some()))
    }

    /// `#{ key = v, … }`.
    fn print_map(&mut self, entries: &[StructId]) {
        self.bracketed("#{", "}", true, entries, |p, entry| {
            if let Struct::List(pair) = p.a.get(entry) {
                let (key, value) = (pair[0], pair[1]);
                p.expr(key, 0);
                p.doc.word(" = ");
                p.expr(value, 0);
            }
        });
    }

    /// `match scrut { pat => body, … }` — one arm per line (consistent box) when broken.
    fn print_match(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        self.doc.cbox(INDENT);
        if paren {
            self.doc.word("(");
        }
        self.doc.word("match ");
        self.expr(args[0], 0);
        self.doc.word(" with");
        // Arms go one per line, each led by `| ` at the `match` column (OCaml style — the leading `|`
        // is always printed, including on the first arm). No braces, no trailing commas.
        let arms = &args[1..];
        for (i, &arm) in arms.iter().enumerate() {
            self.doc.hardbreak();
            self.doc.word("| ");
            if let Struct::List(pair) = self.a.get(arm) {
                let (pat, body) = (pair[0], pair[1]);
                self.pattern(pat);
                self.doc.word(" => ");
                // A body that is itself a block form (`match`/`let`/`if`/`do`) in a NON-LAST arm must
                // parenthesize, else its own arms/layout would run into the next `| pat`. The last
                // arm needs no guard (nothing follows at this level). PREC_KEYWORD forces the
                // block-form parens without parenthesizing an infix body.
                let last = i + 1 == arms.len();
                self.expr(body, if last { 0 } else { PREC_KEYWORD });
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
    fn pattern(&mut self, id: StructId) {
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
                // tuple pattern `(tuple p …)` -> `(p, …)`, matching the value tuple. A 1-element
                // `(tuple p)` prints `(p,)` (trailing comma) so it re-reads as a 1-tuple, not `(p)`
                // grouping.
                if self.head_name(items[0]).as_deref() == Some("tuple") && items.len() >= 2 {
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
                // dotted constructor `(. A B)` prints as A.B
                if self.head_name(items[0]).as_deref() == Some(".")
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
            _ => {
                let leaf = match self.a.get(id) {
                    Struct::Atom(l) => self.a.leaf(*l).clone(),
                    _ => unreachable!(),
                };
                self.leaf(&leaf);
            }
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

    fn unquote_atomic(&self, id: StructId) -> bool {
        match self.a.get(id) {
            Struct::Atom(_) => true,
            Struct::List(items) => {
                // a pure member-access chain `(. a b)` with a plain-ident key
                self.head_name(items[0]).as_deref() == Some(".")
                    && items.len() == 3
                    && self.plain_key(items[2]).is_some()
                    && self.unquote_atomic(items[1])
            }
        }
    }

    // ---- shape helpers ----

    fn head_name(&self, id: StructId) -> Option<String> {
        match self.a.get(id) {
            Struct::Atom(l) => match self.a.leaf(*l) {
                Leaf::Name(n) => Some(n.clone()),
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
                Leaf::Str(s) => Some(s.clone()),
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

    /// If `id` is an `Atom(Name)` that is a plain member key (alpha/underscore start, no dots), that
    /// name — so `(. a b)` prints as `a.b` but a dotted/odd key falls back to the call form.
    fn plain_key(&self, id: StructId) -> Option<String> {
        let n = self.head_name(id)?;
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
                } if value.sign() != num_bigint::Sign::Minus => {
                    Some(literal::render_int(value, crate::ast::Radix::Dec))
                }
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
                Leaf::Int { radix: crate::ast::Radix::Dec, value } if value.sign() != num_bigint::Sign::Minus
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
        match self.a.get(args[0]) {
            Struct::List(binds) => binds.iter().all(|&b| match self.a.get(b) {
                Struct::List(p) => p.len() == 2 && self.head_name(p[0]).is_some(),
                _ => false,
            }),
            _ => false,
        }
    }

    fn is_match_shape(&self, args: &[StructId]) -> bool {
        // A scrutinee plus at LEAST ONE arm. A zero-arm match (`(match x)` — vacuously exhaustive on
        // a Never-typed scrutinee) has no `| arm` to render and no closer after `with`, so it falls
        // through to the generic call form `` `match`(x) `` instead (which round-trips as a call).
        if args.len() < 2 {
            return false;
        }
        args[1..].iter().all(|&a| match self.a.get(a) {
            Struct::List(p) => p.len() == 2,
            _ => false,
        })
    }

    /// Every arg is a 2-element `(key value)` pair — the shape the record/map surfaces render. A
    /// malformed record/map (an arg that isn't a pair) falls back to the generic call form so it
    /// still round-trips. A record additionally needs its field key to be a name; a map key is any
    /// expression.
    fn is_pairs(&self, args: &[StructId]) -> bool {
        args.iter()
            .all(|&a| matches!(self.a.get(a), Struct::List(p) if p.len() == 2))
    }

    /// A record the `{ name = e, … }` surface handles: every field is a `(name value)` pair whose
    /// key is a plain field name (so it re-reads as a `name = value` binding).
    fn is_record_shape(&self, args: &[StructId]) -> bool {
        self.is_pairs(args)
            && args.iter().all(|&a| match self.a.get(a) {
                Struct::List(p) => self.plain_key(p[0]).is_some(),
                _ => false,
            })
    }

    /// A map the `#{ key: v, … }` surface handles: every entry is a `(key value)` pair (any key).
    fn is_map_shape(&self, args: &[StructId]) -> bool {
        self.is_pairs(args)
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

/// A name prints bare when it re-lexes to exactly itself as a single `Ident`/operator token AND is
/// not a reserved word; otherwise it is backtick-quoted. This is the lossless escape for symbolic
/// heads (`|`, `+`, `->`), keyword-shaped names (`let`, `in`), and anything that would otherwise
/// lex as something else.
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
    use crate::{parser, sexpr};

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

    #[test]
    fn small_forms_inline() {
        assert_eq!(assert_roundtrip("1 + 2 * 3", 80), "1 + 2 * 3");
        assert_eq!(assert_roundtrip("f(a, b, c)", 80), "f(a, b, c)");
        assert_eq!(assert_roundtrip("a.b.c", 80), "a.b.c");
        assert_eq!(
            assert_roundtrip("if a then b else c", 80),
            "if a then b else c"
        );
        // `let … in` always breaks the body to its own line at the let column (ML idiom).
        assert_eq!(assert_roundtrip("let x = 1 in x", 80), "let x = 1 in\nx");
        assert_eq!(
            assert_roundtrip("fn(x, y) => x + y", 80),
            "fn(x, y) => x + y"
        );
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
        // Consecutive top-level definitions get a blank line between them (readability), and the
        // layout round-trips (blank lines are whitespace).
        assert_eq!(
            assert_roundtrip("def a = 1 def b = 2 def c = 3", 80),
            "def a = 1;\n\ndef b = 2;\n\ndef c = 3"
        );
    }

    #[test]
    fn doc_line_hugs_its_def_no_blank() {
        // A `///` doc line stays glued to the def it documents (single break), while distinct
        // definitions are still blank-separated.
        assert_eq!(
            assert_roundtrip("/// first\ndef a = 1\n/// second\ndef b = 2", 80),
            "/// first\ndef a = 1;\n\n/// second\ndef b = 2"
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
    fn def_let_body_drops_and_indents() {
        // A non-brace body (let) is not hugged: it drops to an indented continuation line and lays
        // out flat at that indent.
        let out = assert_roundtrip("def f(x) = let y = x + 1 in y * y", 80);
        assert_eq!(out, "def f(x) =\n  let y = x + 1 in\n  y * y");
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

    #[test]
    fn name_headed_literal_round_trips_via_head_normalization() {
        // Sugaring a NAME head means the reprint re-reads with a STRING head; `structurally_eq`
        // normalizes the two head kinds for the four ctors, so the round-trip still holds.
        for src in [
            "(record (x 1) (y 2))",
            "(tuple 1 2 3)",
            "(list 1 2 3)",
            "(map (a 1))",
        ] {
            let a = sexpr::read(src).unwrap();
            let printed = print(&a, 80);
            let back = parser::read_ml(&printed);
            assert!(
                back.ok() && back.arenas.structurally_eq(&a),
                "{src} -> {printed} did not round-trip"
            );
        }
    }

    #[test]
    fn record_field_shorthand() {
        // `{ x }` puns to `(record (x x))`; the printer renders a same-name field back as `{ x }`.
        assert_eq!(print(&sexpr::read("(record (x x))").unwrap(), 80), "{ x }");
        assert_eq!(
            print(&sexpr::read("(record (x x) (y 2))").unwrap(), 80),
            "{ x, y = 2 }"
        );
        // a non-punned field keeps `name = value`.
        assert_eq!(
            print(&sexpr::read("(record (x 1))").unwrap(), 80),
            "{ x = 1 }"
        );
        // parse `{ x }` → the pun (a STRING-headed record primitive, per the reader's literal desugar).
        assert_eq!(
            sexpr::print(&parser::read_ml("{ x }").arenas),
            "(\"record\" (x x))"
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
    fn exact_numbers_print_and_reparse() {
        // Hex is canonicalized to lowercase digits (case is not preserved in the leaf); the value
        // and base round-trip, and the printed form is idempotent.
        assert_eq!(assert_roundtrip("0x2A", 80), "0x2a");
        assert_eq!(assert_roundtrip("0x2a", 80), "0x2a");
        assert_eq!(assert_roundtrip("1.5", 80), "1.5");
        assert_eq!(assert_roundtrip("1000000", 80), "1000000");
        assert_eq!(assert_roundtrip("0b1010", 80), "0b1010");
    }
}
