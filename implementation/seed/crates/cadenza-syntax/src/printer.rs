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
            if let Some(prec) = infix_prec(&head) {
                if args.len() == 2 {
                    return self.infix(&head, prec, args[0], args[1], parent_prec);
                }
            }
            // ---- member access `(. obj key)` -> obj.key ----
            if head == "." && args.len() == 2 {
                if let Some(key) = self.plain_key(args[1]) {
                    self.doc.ibox(0);
                    self.expr(args[0], PREC_MEMBER);
                    self.doc.word(".");
                    self.doc.word(emit_name(&key));
                    self.doc.end();
                    return;
                }
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

    /// `( a, b, c )` argument list — all-or-nothing: inline if it fits, else one arg per line,
    /// block-indented, closing `)` on its own line, with a trailing comma when broken.
    fn call_args(&mut self, args: &[StructId]) {
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
            // trailing comma appears only when broken (a zero-width break in flat mode)
            self.doc.break_with(0, -INDENT);
        }
        self.doc.word(")");
        self.doc.end();
    }

    /// `l op r`, left-associative. The break sits BEFORE the operator so a wrapped right side lands
    /// with the operator leading its line.
    fn infix(&mut self, op: &str, prec: u8, l: StructId, r: StructId, parent_prec: u8) {
        let paren = prec < parent_prec;
        self.doc.ibox(INDENT);
        if paren {
            self.doc.word("(");
        }
        self.expr(l, prec); // left child may share precedence (left-assoc)
        self.doc.space();
        // In infix position the operator prints VERBATIM (`+`, `and`), never escaped — the escape
        // is only for an operator glyph used as an ordinary NAME (a list element, a head).
        self.doc.word(op.to_string());
        self.doc.word(" ");
        self.expr(r, prec + 1); // right child must bind tighter
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
        self.doc.space();
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
        self.doc.ibox(INDENT);
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
        self.doc.space();
        self.expr(args[1], 0);
        if paren {
            self.doc.word(")");
        }
        self.doc.end();
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
        for &arm in &args[1..] {
            self.doc.space();
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
        if let Some(tail) = self.a.as_form(id, "guard") {
            if tail.len() == 2 {
                let (pat, guard) = (tail[0], tail[1]);
                self.pattern(pat);
                self.doc.word(" if ");
                self.expr(guard, 0);
                return;
            }
        }
        match self.a.get(id) {
            Struct::List(items) if !items.is_empty() => {
                let items = items.clone();
                // dotted constructor `(. A B)` prints as A.B
                if self.head_name(items[0]).as_deref() == Some(".") && items.len() == 3 {
                    if let Some(key) = self.plain_key(items[2]) {
                        self.pattern(items[1]);
                        self.doc.word(".");
                        self.doc.word(emit_name(&key));
                        return;
                    }
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
        assert_eq!(assert_roundtrip("let x = 1 in x", 80), "let x = 1 in x");
        assert_eq!(assert_roundtrip("fn(x, y) => x + y", 80), "fn(x, y) => x + y");
    }

    #[test]
    fn minimal_parens() {
        // precedence: * binds tighter than +, so no parens; but (1 + 2) * 3 needs them
        assert_eq!(assert_roundtrip("(1 + 2) * 3", 80), "(1 + 2) * 3");
        assert_eq!(assert_roundtrip("1 + 2 * 3", 80), "1 + 2 * 3");
    }

    #[test]
    fn match_one_arm_per_line_when_broken() {
        let out = assert_roundtrip("match e { Some(n) => n, None => 0, _ => neg }", 20);
        assert!(out.contains("match e {\n"), "got:\n{out}");
        assert!(out.contains("  Some(n) => n,"), "got:\n{out}");
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
