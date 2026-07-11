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
use crate::token::{self, infix_prec, Kind, PREC_MEMBER};

/// Indentation per box level (spaces). A layout choice, not a contract.
const INDENT: isize = 2;

/// Pretty-print `arenas` to ML text targeting `width` columns.
pub fn print(arenas: &Arenas, width: usize) -> String {
    let mut p = Printer { a: arenas, doc: Doc::new() };
    p.expr(arenas.root, 0);
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
            Leaf::Name(n) => self.doc.word(emit_name(n)),
        }
    }

    fn list(&mut self, items: &[StructId], parent_prec: u8) {
        if items.is_empty() {
            // The reader never produces an empty list; render defensively as the raw-list escape.
            self.doc.word("#[]");
            return;
        }
        // A head that is an Atom(Name) may name a construct or an operator; otherwise it is a
        // computed-callee application.
        let head = self.head_name(items[0]);
        let args = &items[1..];

        if let Some(head) = head {
            // ---- infix binary operator ----
            if let Some(prec) = infix_prec(&head)
                && args.len() == 2
            {
                return self.infix(&head, prec, args[0], args[1], parent_prec);
            }
            // ---- member access `(. obj key)` -> obj.key ----
            if head == "."
                && args.len() == 2
                && let Some(key) = self.plain_key(args[1])
            {
                self.doc.ibox(0);
                self.expr(args[0], PREC_MEMBER);
                self.doc.word(".");
                self.doc.word(emit_name(&key));
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
                "module" if self.is_module_shape(args) => return self.print_module(args),
                "list" => return self.print_list_literal(args),
                "tuple" if args.len() >= 2 => return self.print_tuple(args),
                "record" if self.is_record_shape(args) => return self.print_record(args),
                "map" if self.is_map_shape(args) => return self.print_map(args),
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
            // In infix position the operator prints VERBATIM (`+`, `and`) — the backtick escape is
            // only for an operator glyph used as an ordinary NAME.
            self.doc.word(o.clone());
            self.doc.word(" ");
            self.expr(operands[i + 1], prec + 1); // right operand binds one tighter
        }
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// `let n = e, … in body`.
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
                    self.expr(e, 0);
                }
            }
        }
        self.doc.end();
        self.doc.word(" in");
        // The body always starts a new line at the `let`'s own column (offset 0, not indented), so
        // a chain of `let … in` reads as a flat sequence. This is the ML idiom for an
        // expression-only language where `let … in` is pervasive.
        self.doc.hardbreak();
        self.expr(args[1], 0);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// `if c then t else e`.
    fn print_if(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
        self.doc.cbox(INDENT);
        if paren {
            self.doc.word("(");
        }
        self.doc.word("if ");
        self.expr(args[0], 0);
        self.doc.space();
        self.doc.word("then ");
        self.expr(args[1], 0);
        self.doc.space();
        self.doc.word("else ");
        self.expr(args[2], 0);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// `fn(p, …) => body`.
    fn print_fn(&mut self, args: &[StructId], parent_prec: u8) {
        let paren = parent_prec > 0;
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
                self.expr(p, 0);
            }
        }
        self.doc.word(") =>");
        // A block-like body hugs the `=>` (breaks internally); a plain body drops to an indented
        // line if it overflows — same discipline as a def's `=` body.
        self.body_after_eq(args[1]);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// `fn name(p, …) = body` — a named function definition. `args` is `signature doc… body`: the
    /// signature list `(name p …)`, zero or more `(doc "…")` forms (printed as `/// …` lines above
    /// the `fn`), and the single body. Never parenthesized (a def is a declaration).
    fn print_def(&mut self, args: &[StructId]) {
        let signature = args[0];
        let docs = &args[1..args.len() - 1];
        let body = args[args.len() - 1];
        // Offset 0: a hugged block body (match/let/…) carries its own indentation relative to its
        // own start column, so the def box must not add another level on top. A plain-expression
        // body that overflows is indented explicitly by `body_after_eq`.
        self.doc.cbox(0);
        for &d in docs {
            // each doc member is a `(doc "text")` (guaranteed by is_def_shape) -> `/// text`
            if let Some(a) = self.a.as_form(d, "doc") {
                self.print_doc(a[0]);
            }
            self.doc.hardbreak();
        }
        self.doc.word("fn ");
        if let Struct::List(sig) = self.a.get(signature) {
            let sig = sig.clone();
            // name
            self.expr(sig[0], 0);
            self.doc.word("(");
            for (i, &p) in sig[1..].iter().enumerate() {
                if i > 0 {
                    self.doc.word(", ");
                }
                self.expr(p, 0);
            }
            self.doc.word(") =");
        }
        self.body_after_eq(body);
        self.doc.end();
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
            Struct::List(items) if !items.is_empty() => {
                (self.head_name(items[0]), &items[1..])
            }
            _ => return false,
        };
        match head.as_deref() {
            Some("match") => self.is_match_shape(args),
            Some("module") => self.is_module_shape(args),
            Some("list") => true,
            Some("tuple") => args.len() >= 2,
            Some("record") => self.is_record_shape(args),
            Some("map") => self.is_map_shape(args),
            _ => false,
        }
    }

    /// `module name { form… }` — one form per line (consistent box) when broken.
    fn print_module(&mut self, args: &[StructId]) {
        self.doc.cbox(INDENT);
        self.doc.word("module ");
        self.expr(args[0], 0); // name
        self.doc.word(" {");
        for &form in &args[1..] {
            self.doc.hardbreak(); // one member per line
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

    /// `(e, …)` — a tuple of 2+ elements.
    fn print_tuple(&mut self, elems: &[StructId]) {
        self.bracketed("(", ")", false, elems, |p, e| p.expr(e, 0));
    }

    /// `{ name = e, … }`.
    fn print_record(&mut self, fields: &[StructId]) {
        self.bracketed("{", "}", true, fields, |p, field| {
            if let Struct::List(pair) = p.a.get(field) {
                let (name, value) = (pair[0], pair[1]);
                p.expr(name, 0);
                p.doc.word(" = ");
                p.expr(value, 0);
            }
        });
    }

    /// `#{ key: v, … }`.
    fn print_map(&mut self, entries: &[StructId]) {
        self.bracketed("#{", "}", true, entries, |p, entry| {
            if let Struct::List(pair) = p.a.get(entry) {
                let (key, value) = (pair[0], pair[1]);
                p.expr(key, 0);
                p.doc.word(": ");
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
        self.doc.word(" {");
        // Match arms ALWAYS go one per line (ML/Rust convention — arms are never packed onto a
        // line, even when they'd fit), each with a trailing comma, and the closing `}` dedents to
        // the `match` column.
        for &arm in &args[1..] {
            self.doc.hardbreak();
            if let Struct::List(pair) = self.a.get(arm) {
                let (pat, body) = (pair[0], pair[1]);
                self.pattern(pat);
                self.doc.word(" => ");
                self.expr(body, 0);
                self.doc.word(",");
            }
        }
        self.doc.break_with(1, -INDENT);
        self.doc.word("}");
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
    }

    /// Print a structural pattern. Shapes: a guarded pattern `(guard <pat> <expr>)` -> `pat if g`;
    /// a constructor application `(Ctor p…)` -> `Ctor(p, …)`; a dotted ctor `(. A B)` -> `A.B`; a
    /// bare name / literal prints as itself.
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
        if args.is_empty() {
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
        args.iter().all(|&a| matches!(self.a.get(a), Struct::List(p) if p.len() == 2))
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

    /// A def the `fn name(…) = body` surface handles: a signature list `(name p…)` (head is a
    /// name, so params lower as binders), then zero or more `(doc "…")` forms, then a single body.
    /// Any OTHER interleaved body form (e.g. a `(: type)` annotation) falls back to the generic call
    /// form so it still round-trips.
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
        (Some(t), None) => {
            t.kind == Kind::Ident && t.span.start == 0 && t.span.end == s.len()
        }
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
        assert_eq!(printed, printed2, "not idempotent: {src:?} -> {printed:?} -> {printed2:?}");
        printed
    }

    #[test]
    fn small_forms_inline() {
        assert_eq!(assert_roundtrip("1 + 2 * 3", 80), "1 + 2 * 3");
        assert_eq!(assert_roundtrip("f(a, b, c)", 80), "f(a, b, c)");
        assert_eq!(assert_roundtrip("a.b.c", 80), "a.b.c");
        assert_eq!(assert_roundtrip("if a then b else c", 80), "if a then b else c");
        // `let … in` always breaks the body to its own line at the let column (ML idiom).
        assert_eq!(assert_roundtrip("let x = 1 in x", 80), "let x = 1 in\nx");
        assert_eq!(assert_roundtrip("fn(x, y) => x + y", 80), "fn(x, y) => x + y");
    }

    #[test]
    fn function_definition() {
        // named def vs anonymous lambda are distinct surfaces.
        assert_eq!(assert_roundtrip("fn add(a, b) = a + b", 80), "fn add(a, b) = a + b");
        assert_eq!(assert_roundtrip("fn main() = 42", 80), "fn main() = 42");
        assert_eq!(assert_roundtrip("fn(x) => x * 2", 80), "fn(x) => x * 2");
    }

    #[test]
    fn module_block() {
        let out = assert_roundtrip("module math { fn add(a, b) = a + b fn main() = add(2, 3) }", 80);
        assert_eq!(out, "module math {\n  fn add(a, b) = a + b\n  fn main() = add(2, 3)\n}");
    }

    #[test]
    fn def_match_body_hugs_the_eq() {
        // A brace-delimited body (match) stays on the `=` line and breaks internally; arms indent
        // one level under the def, not two.
        let out = assert_roundtrip("fn describe(s) = match s { A(_) => 1, B(_) => 2 }", 80);
        assert_eq!(out, "fn describe(s) = match s {\n  A(_) => 1,\n  B(_) => 2,\n}");
    }

    #[test]
    fn def_let_body_drops_and_indents() {
        // A non-brace body (let) is not hugged: it drops to an indented continuation line and lays
        // out flat at that indent.
        let out = assert_roundtrip("fn f(x) = let y = x + 1 in y * y", 80);
        assert_eq!(out, "fn f(x) =\n  let y = x + 1 in\n  y * y");
    }

    #[test]
    fn def_record_body_hugs_the_eq() {
        // A literal body (record) hugs the `=` too: `{` stays on the line, fields indent one level.
        let out = assert_roundtrip("fn point() = { x = 1, y = 2, z = 3 }", 20);
        assert_eq!(out, "fn point() = {\n  x = 1,\n  y = 2,\n  z = 3\n}");
        // and inline when it fits
        assert_eq!(assert_roundtrip("fn point() = { x = 1 }", 80), "fn point() = { x = 1 }");
    }

    #[test]
    fn last_arg_lambda_hugs() {
        // A trailing lambda stays on the call line, breaking only its own body — head args inline.
        let out = assert_roundtrip(
            "fold(xs, zero, fn(acc, x) => match x { Some(v) => acc + v, None(_) => acc })",
            80,
        );
        assert_eq!(
            out,
            "fold(xs, zero, fn(acc, x) => match x {\n  Some(v) => acc + v,\n  None(_) => acc,\n})"
        );
    }

    #[test]
    fn last_arg_hug_fits_inline() {
        // When the whole call fits, hugging is invisible — it stays on one line.
        assert_eq!(assert_roundtrip("map(items, fn(x) => x + 1)", 80), "map(items, fn(x) => x + 1)");
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
        assert_eq!(out, "some-function(\n  alpha,\n  beta,\n  gamma,\n  delta\n)");
    }

    #[test]
    fn literals_render_and_round_trip() {
        assert_eq!(assert_roundtrip("{ x = 1, y = 2 }", 80), "{ x = 1, y = 2 }");
        assert_eq!(assert_roundtrip("[1, 2, 3]", 80), "[1, 2, 3]");
        assert_eq!(assert_roundtrip("(1, 2)", 80), "(1, 2)");
        assert_eq!(assert_roundtrip("(1, 2, 3)", 80), "(1, 2, 3)");
        assert_eq!(assert_roundtrip("#{ \"a\": 1, \"b\": 2 }", 80), "#{ \"a\": 1, \"b\": 2 }");
        assert_eq!(assert_roundtrip("#{ 1: 10 }", 80), "#{ 1: 10 }");
    }

    #[test]
    fn empty_literals() {
        // Empty list and map from the s-expr surface (`(list)`, `(map)`) render as `[]` / `#{}`.
        assert_eq!(print(&sexpr::read("(list)").unwrap(), 80), "[]");
        assert_eq!(print(&sexpr::read("(map)").unwrap(), 80), "#{}");
    }

    #[test]
    fn literals_break_all_or_nothing_when_wide() {
        let out = assert_roundtrip("{ name = \"alice\", scores = [90, 85, 95], active = true }", 30);
        assert_eq!(out, "{\n  name = \"alice\",\n  scores = [90, 85, 95],\n  active = true\n}");
    }

    #[test]
    fn paren_grouping_is_not_a_tuple() {
        // `(1 + 2) * 3` — the parens are transparent grouping, NOT a 1-tuple.
        assert_eq!(assert_roundtrip("(1 + 2) * 3", 80), "(1 + 2) * 3");
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
        // A def carrying `(doc …)` forms renders them as `/// …` lines above the `fn`.
        let a = sexpr::read("(def (f x) (doc \"hi\") (+ x 1))").unwrap();
        let printed = print(&a, 80);
        assert_eq!(printed, "/// hi\nfn f(x) = x + 1");
        let b = parser::read_ml(&printed);
        assert!(b.ok() && b.arenas.structurally_eq(&a), "printed:\n{printed}");
    }

    #[test]
    fn non_doc_multi_form_def_falls_back() {
        // A def whose extra body form is NOT a doc (here a `(: type)` annotation) has no dedicated
        // surface; it falls back to the generic call form and still round-trips.
        let a = sexpr::read("(def (f x) (: Int64) (+ x 1))").unwrap();
        let printed = print(&a, 80);
        assert_eq!(printed, "def(f(x), `:`(Int64), x + 1)");
        let b = parser::read_ml(&printed);
        assert!(b.ok() && b.arenas.structurally_eq(&a), "printed:\n{printed}");
    }

    #[test]
    fn doc_and_comment_round_trip() {
        // `///` doc attaches inside the def; `//` comment wraps it.
        assert_eq!(
            assert_roundtrip("/// Adds.\nfn add(a, b) = a + b", 80),
            "/// Adds.\nfn add(a, b) = a + b"
        );
        assert_eq!(
            assert_roundtrip("// note\nfn main() = 42", 80),
            "// note\nfn main() = 42"
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
        // Arms go one per line even at a WIDE width where they would fit on one line — the ML/Rust
        // convention, never packed.
        let out = assert_roundtrip("match e { Some(n) => n, None => 0, _ => neg }", 200);
        assert_eq!(out, "match e {\n  Some(n) => n,\n  None => 0,\n  _ => neg,\n}", "got:\n{out}");
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
        assert!(printed.contains("`+`") && printed.contains("`-`"), "got: {printed}");
    }

    #[test]
    fn guarded_arm_prints_if() {
        let out = assert_roundtrip("match n { x if x < 0 => neg, _ => pos }", 80);
        assert!(out.contains("x if x < 0 =>"), "got: {out}");
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
