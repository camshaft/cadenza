//! An s-expression reader: text -> [`Arenas`]. Two roles:
//!
//! 1. The **corpus oracle** for round-trip tests — an independent code path from the ML reader, so
//!    a bug in the ML reader/printer can't mask itself (anti-collusion). It parses the canonical
//!    homoiconic display the existing corpus is written in.
//! 2. The first-class **s-expression co-surface** — the direct code-as-data rendering, kept for
//!    metaprogramming and structural editing where the uniform `(head child…)` shape is the
//!    natural target.
//!
//! The numeric classification (radix `0x`/`0b`, `_` between-digits separators, float shape,
//! malformed-is-rejected) is the strict rule ported from the seed reader, adapted to produce
//! arbitrary-precision `Int` and an exact `Decimal` (no `i64`/`f64` ceiling). The ML lexer MUST
//! classify literals identically to this, or the round-trip fails.

use crate::ast::{Arenas, Builder, Leaf, LeafId, Struct, StructId};
use crate::doc::Doc;
use crate::span::Span;
use crate::spans::{FileId, SpanTable};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug)]
pub struct ReadError(pub String);

/// The maximum nesting depth the recursive-descent reader accepts before returning a [`ReadError`]
/// rather than recursing further. Recursive descent (`read_list` → `read_node` → `read_primary` →
/// `read_list`) uses one native stack frame per nesting level, so unbounded depth overflows the stack
/// and ABORTS the process (SIGABRT) on pathologically deep — but syntactically valid — input; the
/// crash is worse on the guide's `cdz-wasm` (a ~1 MB stack parsing UNTRUSTED browser input, where the
/// overflow depth is far lower than the ~25000 a native 8 MB stack reaches).
///
/// Set to the compiler's own `DESCENT_DEPTH_LIMIT` (rcdzc `db.rs` = 1024): the compiler already
/// DECLINES a program nested past that ("expression nests too deeply to compile"), so a source the
/// parser rejects here is one the compiler would reject anyway — no valid program is lost. Matching
/// (not exceeding) the compiler's limit keeps the deepest margin below the smallest target stack (the
/// ~1 MB wasm stack overflows well before 4096 native-frame-equivalents), so every source-ingesting
/// entry point (`convert`/`check`/`fix`, `cdz-wasm`) returns a clean diagnostic instead of crashing.
pub const MAX_NESTING_DEPTH: u32 = 1024;

/// Parse a single s-expression from `text` into its own `Arenas` (rooted at the parsed form). This is
/// the READER — text to the canonical representation, so a program can be written as text before a
/// surface syntax exists; and it is NOT in the compiler's derive path (the compiler consumes the binary
/// AST directly, `ast-encoding.md` §Parsing And Printing Are Not In The Compiler's Trusted Path).
//= spec/capabilities/self-hosting-surface.md#a-reader-converts-text-to-the-canonical-representation
//# A reader MUST convert the text of a program to the program's canonical representation, so that a program can be written as text before a surface syntax exists.
//= spec/capabilities/self-hosting-surface.md#a-reader-converts-text-to-the-canonical-representation
//# A reader MUST NOT be required in the path that derives a component, consistent with the ast-encoding contract keeping parsing out of the compiler's trusted path.
//= spec/contracts/ast-encoding.md#a-textual-syntax-parses-to-and-prints-from-the-canonical-form
//# A textual syntax MUST provide a parser that converts its text to the canonical binary AST.
//= spec/contracts/ast-encoding.md#a-textual-syntax-parses-to-and-prints-from-the-canonical-form
//# No textual syntax MUST be privileged as the stored form, so that a program's identity is its binary AST and not any one rendering of it.
//= spec/capabilities/agent-authoring.md#textual-syntaxes-round-trip-through-the-canonical-form
//# Parsing a textual rendering of a program MUST yield its canonical binary AST.
pub fn read(text: &str) -> Result<Arenas, ReadError> {
    let mut b = Builder::new();
    let mut p = Reader::new(text, &mut b, false);
    p.skip_ws();
    let root = p.read_node()?;
    p.skip_ws();
    if p.peek().is_some() {
        return Err(ReadError(format!("trailing input at byte {}", p.pos)));
    }
    Ok(b.finish(root))
}

/// Parse a single s-expression, ALSO producing a [`SpanTable`] mapping each structure occurrence to
/// its source byte range — the same source-tracking substrate the ML parser produces. This is what
/// lets a formatting-preserving edit splice a replacement into the original text at a node's span,
/// instead of reprinting the whole tree. The arena is byte-identical to [`read`]'s; only the table
/// is extra. Every occurrence gets a span (the table is total and exactly 1:1 with the arena), so
/// synthetic/desugared sub-nodes (a member-access `.` head, a dotted-name spine) carry a best-effort
/// span covering their source extent.
pub fn read_spanned(text: &str) -> Result<(Arenas, SpanTable), ReadError> {
    let mut b = Builder::new();
    let mut p = Reader::new(text, &mut b, true);
    p.skip_ws();
    let root = p.read_node()?;
    p.skip_ws();
    if p.peek().is_some() {
        return Err(ReadError(format!("trailing input at byte {}", p.pos)));
    }
    let spans = p.spans.take().expect("span tracking on");
    Ok((b.finish(root), spans))
}

/// Parse every top-level s-expression from `text`, each as an element of a synthetic `(do …)` root
/// — convenient for reading a corpus file, whose top level is a sequence of `(case …)` forms.
pub fn read_all(text: &str) -> Result<Arenas, ReadError> {
    let (arenas, _) = read_all_impl(text, false)?;
    Ok(arenas)
}

/// Like [`read_all`], but also produces a [`SpanTable`] (see [`read_spanned`]). The synthetic `(do
/// …)` wrapper node spans the whole input and its `do` head an empty span at byte 0.
pub fn read_all_spanned(text: &str) -> Result<(Arenas, SpanTable), ReadError> {
    let (arenas, spans) = read_all_impl(text, true)?;
    Ok((arenas, spans.expect("span tracking on")))
}

fn read_all_impl(text: &str, track: bool) -> Result<(Arenas, Option<SpanTable>), ReadError> {
    let mut b = Builder::new();
    let mut roots = Vec::new();
    let mut spans;
    {
        let mut p = Reader::new(text, &mut b, track);
        loop {
            p.skip_ws();
            if p.peek().is_none() {
                break;
            }
            roots.push(p.read_node()?);
        }
        spans = p.spans.take();
    }
    // The `do` head and the wrapping list are synthetic (no source text): head at byte 0, list over
    // the whole input. They are created AFTER every root, matching structure-id order, so pushing
    // their spans here keeps the table 1:1 with the arena.
    let do_head = b.name("do");
    if let Some(t) = spans.as_mut() {
        t.push(Span::new(0, 0));
    }
    let mut items = Vec::with_capacity(roots.len() + 1);
    items.push(do_head);
    items.extend(roots);
    let root = b.list(items);
    if let Some(t) = spans.as_mut() {
        t.push(Span::new(0, text.len()));
    }
    Ok((b.finish(root), spans))
}

// ============================================================================
// Printer: Arenas -> s-expression text. The direct code-as-data rendering, and the dual of the
// reader above (it re-reads to a structurally-equal arena).
// ============================================================================

/// Render `arenas` as an s-expression string. This is the PRINTER — the canonical representation to
/// text that the reader above converts back to the same canonical representation, so reader ∘ printer
/// round-trips to a structurally-equal value.
//= spec/capabilities/self-hosting-surface.md#a-printer-renders-the-canonical-representation-as-re-readable-text
//# A printer MUST render a program's canonical representation as text that a reader converts back to the same canonical representation.
//= spec/capabilities/self-hosting-surface.md#a-printer-renders-the-canonical-representation-as-re-readable-text
//# Reading the text a printer produced for a value MUST yield a value equal to the original under structural equality, so that the reader and printer round-trip.
//= constitution.md#x-programs-are-readable-by-agents-and-humans
//# A textual syntax MUST be a lossless projection of the canonical form, such that parsing its text yields the canonical form and printing the canonical form yields text that parses back to the same canonical form.
//= spec/contracts/ast-encoding.md#a-textual-syntax-parses-to-and-prints-from-the-canonical-form
//# A textual syntax MUST provide a printer that converts the canonical binary AST to its text.
//= spec/capabilities/agent-authoring.md#textual-syntaxes-round-trip-through-the-canonical-form
//# Printing a program's canonical binary AST in a textual syntax MUST yield text that parses back to the same canonical binary AST.
pub fn print(arenas: &Arenas) -> String {
    let mut out = String::new();
    print_node(arenas, arenas.root, &mut out);
    out
}

/// Render one occurrence (a sub-form at `id`) as an s-expression string — for re-emitting a form
/// extracted from a larger tree (e.g. a `(case …)`'s `(input …)` payload), on a single line.
pub fn print_from(arenas: &Arenas, id: StructId) -> String {
    let mut out = String::new();
    print_node(arenas, id, &mut out);
    out
}

fn print_node(a: &Arenas, id: StructId, out: &mut String) {
    match a.get(id) {
        Struct::Atom(l) => print_leaf(a.leaf(*l), out),
        Struct::List(items) => {
            // RESUGAR: a `(: <suffixed-literal> BigInt|Rational)` node is the desugared form of a type
            // suffix (`100N`), so print just the suffixed atom — the suffix already carries the type.
            // (A bare `(: 100 BigInt)` value-output, whose value child is a plain `Int` not a
            // `Suffixed`, is NOT matched, so it still prints the explicit annotation.)
            if let Some(atom) = suffixed_annotation_atom(a, items) {
                print_node(a, atom, out);
                return;
            }
            out.push('(');
            for (i, &child) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                print_node(a, child, out);
            }
            out.push(')');
        }
    }
}

/// If `items` is a `(: <atom> <type>)` annotation whose value child is a `Leaf::Suffixed`, return that
/// atom's id (the printer resugars it back to the bare `100N`/`0.5R` form). Else `None`. Shared by the
/// single-line and pretty s-expr printers so both resugar identically.
fn suffixed_annotation_atom(a: &Arenas, items: &[StructId]) -> Option<StructId> {
    if items.len() != 3 || a.as_name(items[0]) != Some(":") {
        return None;
    }
    match a.get(items[1]) {
        Struct::Atom(l) if matches!(a.leaf(*l), Leaf::Suffixed { .. }) => Some(items[1]),
        _ => None,
    }
}

// ============================================================================
// Pretty-printer: the same tree, but laid out across lines so it stays readable when a form is too
// wide for one line. Where `print` above emits everything on a single line (the canonical, machine-
// oriented rendering the round-trip oracle and codemod paths depend on), this walks the arena into
// the shared Oppen [`Doc`] engine: each `(head child…)` list prints flat when it fits the target
// width, else breaks with every child on its own line indented one level under the head —
//
//     (match e
//       (Some n)
//       (None 0))
//
// Whitespace is the ONLY difference from `print` (spaces become newline+indent between the same
// tokens), so a pretty-printed form re-reads to a structurally-identical arena.
// ============================================================================

/// Indentation per nesting level (spaces). A layout choice, matching the ML printer's `INDENT`.
const INDENT: isize = 2;

/// The default target width for the pretty-printer (columns), shared with the ML printer.
pub const DEFAULT_WIDTH: usize = crate::printer::DEFAULT_WIDTH;

/// Pretty-print `arenas` as multi-line s-expression text targeting the default width.
pub fn print_pretty(arenas: &Arenas) -> String {
    print_pretty_width(arenas, DEFAULT_WIDTH)
}

/// Pretty-print `arenas` as multi-line s-expression text targeting `width` columns.
pub fn print_pretty_width(arenas: &Arenas, width: usize) -> String {
    print_pretty_from(arenas, arenas.root, width)
}

/// Pretty-print one occurrence (a sub-form at `id`) as multi-line s-expression text targeting
/// `width` columns — the pretty counterpart of [`print_from`].
pub fn print_pretty_from(arenas: &Arenas, id: StructId, width: usize) -> String {
    let mut doc = Doc::new();
    pretty_node(arenas, id, &mut doc, true);
    doc.render(width)
}

/// Render occurrence `id`. `top` marks a declaration-level position (the root, or a module body) —
/// where a top-level `(do …)` form-sequence blank-separates its members. It is cleared for every
/// nested child, so a `(do …)` used as a function body deeper in the tree keeps its statements
/// tightly single-broken. A `module` blank-separates its members at ANY depth (a module body is
/// always a declaration list).
fn pretty_node(a: &Arenas, id: StructId, doc: &mut Doc, top: bool) {
    match a.get(id) {
        Struct::Atom(l) => {
            let mut s = String::new();
            print_leaf(a.leaf(*l), &mut s);
            doc.word(s);
        }
        Struct::List(items) => {
            // The reader never produces an empty list; render defensively as `()`.
            if items.is_empty() {
                doc.word("()");
                return;
            }
            // RESUGAR a desugared type-suffix `(: <suffixed> BigInt|Rational)` to the bare `100N` atom
            // (same rule as the single-line printer, so both round-trip identically).
            if let Some(atom) = suffixed_annotation_atom(a, items) {
                pretty_node(a, atom, doc, false);
                return;
            }
            // A consistent box: `(head child…)` stays flat when it fits `width`, else EVERY inter-
            // child break fires, so each child lands on its own line indented one level under the
            // head. The head hugs the `(`; the closing `)` hugs the last child (no dangling paren).
            doc.cbox(INDENT);
            doc.word("(");
            pretty_node(a, items[0], doc, false);
            // The MEMBERS of a top-level form sequence (`do`) or a `module` are definitions — a
            // single break between them reads as a crammed wall. Separate them with a BLANK line
            // (which only materializes when the box breaks; a small sequence that fits stays on one
            // line with plain single spaces). The `do`/`module` HEAD, and a module's NAME, still
            // attach with an ordinary break — only definition-to-definition gets the blank line. A
            // nested `do` (top cleared) is a statement block, so it stays tightly single-broken.
            let blank_sep_from = match a.head_name(id) {
                Some("do") if top => 1, // root statement sequence: blank between statements
                Some("module") => 2,    // (module name member1 …): blank between the members
                _ => usize::MAX,        // any other form: ordinary single-break separation
            };
            for (i, &child) in items.iter().enumerate().skip(1) {
                if i > blank_sep_from {
                    blank_line(doc);
                } else {
                    doc.space();
                }
                pretty_node(a, child, doc, false);
            }
            doc.word(")");
            doc.end();
        }
    }
}

/// A separator that renders as a single space when its box is flat, but a BLANK line (two newlines)
/// when the box breaks — a `space` (1 space / newline) immediately followed by a `zerobreak`
/// (nothing / newline). In a consistent box both breaks fire together on break, yielding the empty
/// line; when flat, the space is 1 and the zerobreak 0, so it is the same single space as an
/// ordinary separator (a fitting sequence is unchanged, only broken ones get breathing room).
fn blank_line(doc: &mut Doc) {
    doc.space();
    doc.zerobreak();
}

fn print_leaf(leaf: &Leaf, out: &mut String) {
    match leaf {
        Leaf::Int { value, radix } => out.push_str(&crate::literal::render_int(value, *radix)),
        Leaf::Float(d) => out.push_str(&crate::literal::render_decimal(d)),
        Leaf::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Leaf::Str(s) => {
            out.push('"');
            out.push_str(&crate::literal::escape_string(s));
            out.push('"');
        }
        // A byte sequence renders `b"…"` — the byte-string form (printable ASCII raw, else `\xNN`).
        Leaf::Bytes(b) => {
            out.push_str("b\"");
            out.push_str(&crate::literal::escape_bytes(b));
            out.push('"');
        }
        // A name is written verbatim. (The s-expr surface has no reserved words — `let`, `+`, `|`
        // are all ordinary atoms — so no escaping is needed here, unlike the ML surface.)
        Leaf::Name(n) => out.push_str(n),
        // A symbol renders `#"…"` (reusing the string escape set) — re-reads to the same `Leaf::Sym`.
        Leaf::Sym(s) => {
            out.push_str("#\"");
            out.push_str(&crate::literal::escape_string(s));
            out.push('"');
        }
        // A bad-escape MARKER round-trips back to the offending literal `"\<c>"` — so re-reading the
        // printed form yields the SAME marker (the reader re-detects the unknown escape). A marker is not
        // valid source; printing it faithfully keeps the round-trip law (`read(print(x)) == x`) rather
        // than losing the defect.
        Leaf::BadEscape(c) => {
            out.push('"');
            out.push('\\');
            out.push(*c);
            out.push('"');
        }
        // A char renders `#\…` (a name for a common control char, `u+HHHH` for another, else the bare
        // scalar) — re-reads to the same scalar.
        Leaf::Char(c) => out.push_str(&crate::literal::render_char(*c)),
        // A bad-char MARKER round-trips to `#\<text>` — re-reading re-detects the malformed literal.
        Leaf::BadChar(s) => {
            out.push_str("#\\");
            out.push_str(s);
        }
        // A TYPE-SUFFIXED literal renders `<body><suffix>` (`100N`, `0.5R`) — re-reads to the same leaf.
        Leaf::Suffixed { value, kind } => {
            out.push_str(&crate::literal::render_suffixed(value, *kind))
        }
    }
}

struct Reader<'a, 'b> {
    src: &'a [u8],
    pos: usize,
    b: &'b mut Builder,
    /// The current nesting depth of the recursive descent — incremented on entry to each `read_list`
    /// and decremented on exit, so it counts open `(` on the descent path. Past [`MAX_NESTING_DEPTH`]
    /// the reader returns a [`ReadError`] instead of recursing, guarding the native stack against
    /// overflow on pathologically deep input (see the constant's docs).
    depth: u32,
    /// When `Some`, every structure occurrence pushes its source span here, in creation order, so
    /// the table stays exactly 1:1 with the arena (`spans[id]` is that occurrence's span). `None`
    /// on the plain [`read`] path — then the `mk_*` helpers are pure builder calls, byte-identical.
    spans: Option<SpanTable>,
}

impl<'a, 'b> Reader<'a, 'b> {
    fn new(text: &'a str, b: &'b mut Builder, track: bool) -> Reader<'a, 'b> {
        Reader {
            src: text.as_bytes(),
            pos: 0,
            b,
            depth: 0,
            spans: track.then(|| SpanTable::new(FileId::default())),
        }
    }

    // ---- span-recording arena helpers ----
    //
    // Every structure-creating call in the reader routes through one of these so a span is pushed
    // in lockstep with the `StructId` it creates. Children are always built before their parent
    // (recursive descent), so pushing each node's span at its `mk_*` call keeps `SpanTable` in
    // structure-id order without any post-hoc reordering.

    /// Push `span` for the occurrence just created (a no-op when not tracking). Asserts the table
    /// stays 1:1 with the arena.
    fn push_span(&mut self, span: Span) {
        if let Some(t) = self.spans.as_mut() {
            debug_assert_eq!(
                t.len() + 1,
                self.b.structure_len(),
                "sexpr span table drifted from the arena"
            );
            t.push(span);
        }
    }

    /// An `Atom(Name)` occurrence covering `span`.
    fn mk_name(&mut self, name: &str, span: Span) -> StructId {
        let id = self.b.name(name);
        self.push_span(span);
        id
    }

    /// An `Atom` of an already-interned leaf id, covering `span`.
    fn mk_atom(&mut self, leaf: LeafId, span: Span) -> StructId {
        let id = self.b.atom(leaf);
        self.push_span(span);
        id
    }

    /// An `Atom` occurrence of `leaf`, covering `span`.
    fn mk_atom_leaf(&mut self, leaf: Leaf, span: Span) -> StructId {
        let id = self.b.atom_leaf(leaf);
        self.push_span(span);
        id
    }

    /// A `List` occurrence over `items`, covering `span`.
    fn mk_list(&mut self, items: Vec<StructId>, span: Span) -> StructId {
        let id = self.b.list(items);
        self.push_span(span);
        id
    }

    /// The recorded start byte of an already-built node (for spanning a postfix over its operand).
    /// Only meaningful while tracking; falls back to `self.pos` otherwise.
    fn span_start_of(&self, id: StructId) -> usize {
        self.spans
            .as_ref()
            .and_then(|t| t.get(id))
            .map(|s| s.start)
            .unwrap_or(self.pos)
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }
    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// Skip whitespace and `; line comments`.
    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(b) if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' => self.pos += 1,
                Some(b';') => {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    /// Read a node, then fold any tightly-following `.member` postfixes into member access. This is what
    /// makes `(Int 8).max` and `Int8.max` read to the SAME `(. … max)` shape — the paren form is the
    /// postfix sibling of the bare-token dotted-name sugar (`classify_token`), extended to an arbitrary
    /// preceding form (a list, string, …). Both are input-only sugar: `print` always emits the explicit
    /// `(. operand key)` list, so the round-trip stays stable.
    fn read_node(&mut self) -> Result<StructId, ReadError> {
        let primary = self.read_primary()?;
        self.read_postfix_members(primary)
    }

    /// Read one primary node (a list, string, sigil form, or atom) — WITHOUT the postfix `.member`
    /// handling that `read_node` layers on top.
    fn read_primary(&mut self) -> Result<StructId, ReadError> {
        self.skip_ws();
        match self.peek() {
            None => Err(ReadError("unexpected end of input".into())),
            Some(b'(') => self.read_list(),
            Some(b')') => Err(ReadError(format!("unexpected ')' at byte {}", self.pos))),
            Some(b'"') => self.read_string(),
            // A byte-string literal `b"…"` — the value form of a `Bytes` (the companion of the `"…"`
            // string literal). Only the exact `b"` prefix opens one; a bare `b` (or `b` starting a
            // longer identifier like `byte`) reads as an ordinary name. Escapes mirror `escape_bytes`:
            // `\n \t \r \\ \"` named, `\xNN` two-hex, else the raw byte.
            Some(b'b') if self.src.get(self.pos + 1) == Some(&b'"') => self.read_byte_string(),
            // A char literal `#\c` — a single Unicode scalar value. Only the exact `#\` prefix opens
            // one; a bare `#` reads as an ordinary token. `#\a`/`#\é` (single char), `#\space`/`#\newline`
            // (named), `#\u+HHHH` (code point). A literal naming a NON-scalar (`#\u+D800`) becomes a
            // `BadChar` marker → CDZ0002 at the compiler.
            Some(b'#') if self.src.get(self.pos + 1) == Some(&b'\\') => self.read_char(),
            // A symbol literal `#"meter"` — an interned name value (`symbol-interning-direction`). Only
            // the exact `#"` prefix opens one; a bare `#` reads as an ordinary token. Reuses string
            // lexing/escapes, producing a `Leaf::Sym` (distinct from `Leaf::Str`). A base dimension in the
            // units layer is named this way (`(Unit.base #"meter")`).
            Some(b'#') if self.src.get(self.pos + 1) == Some(&b'"') => self.read_symbol(),
            // `` ` `` / `,` / `,@` sigils, matching the corpus quasiquote display. The inner form is
            // built BEFORE the synthetic head (preserving structure-id order — the reader is the
            // round-trip oracle, so the arena stays byte-identical to the untracked path). The head
            // gets the sigil's own byte range; the wrapping list spans sigil-through-inner.
            Some(b'`') => {
                let start = self.pos;
                self.bump();
                let sigil = Span::new(start, self.pos);
                let inner = self.read_node()?;
                let head = self.mk_name("quasiquote", sigil);
                Ok(self.mk_list(vec![head, inner], Span::new(start, self.pos)))
            }
            Some(b',') => {
                let start = self.pos;
                self.bump();
                if self.peek() == Some(b'@') {
                    self.bump();
                    let sigil = Span::new(start, self.pos);
                    let inner = self.read_node()?;
                    let head = self.mk_name("unquote-splicing", sigil);
                    Ok(self.mk_list(vec![head, inner], Span::new(start, self.pos)))
                } else {
                    let sigil = Span::new(start, self.pos);
                    let inner = self.read_node()?;
                    let head = self.mk_name("unquote", sigil);
                    Ok(self.mk_list(vec![head, inner], Span::new(start, self.pos)))
                }
            }
            Some(_) => self.read_atom_or_name(),
        }
    }

    /// Fold `.member` postfixes that IMMEDIATELY follow `node` (no intervening whitespace) into nested
    /// member access: `(Int 8).max` → `(. (Int 8) max)`, `(. x).a.b` → `(. (. (. x) a) b)`. A postfix
    /// applies only when the `.` is followed by an identifier SEGMENT (a letter/`_`-led run) — so `(. p
    /// x)` (a `.` head with a trailing space) and a numeric `.5` are left for ordinary reading, and the
    /// segment rule matches `is_dotted_name`'s per-segment rule so `(e).a` and `e.a` agree. `self.src` is
    /// valid UTF-8 and `.` is ASCII, so `pos+1` is a char boundary and the next char decodes cleanly.
    fn read_postfix_members(&mut self, mut node: StructId) -> Result<StructId, ReadError> {
        while self.peek() == Some(b'.') {
            let next_char = std::str::from_utf8(&self.src[self.pos + 1..])
                .ok()
                .and_then(|s| s.chars().next());
            match next_char {
                Some(c) if c.is_alphabetic() || c == '_' => {
                    let dot_pos = self.pos;
                    self.bump(); // '.'
                    // A segment runs up to whitespace, a paren, a comment, or the NEXT '.' (which starts
                    // a further postfix on the next loop iteration).
                    let start = self.pos;
                    while let Some(b) = self.peek() {
                        if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b';' | b'.') {
                            break;
                        }
                        self.pos += 1;
                    }
                    let seg = std::str::from_utf8(&self.src[start..self.pos])
                        .map_err(|_| ReadError("non-utf8 member segment".into()))?;
                    // Synthetic `(. operand key)`: the `.` head spans the dot, the key spans the
                    // segment, the list spans from the operand's start through the segment.
                    let operand_start = self.span_start_of(node);
                    let dot = self.mk_name(".", Span::new(dot_pos, dot_pos + 1));
                    let key = self.mk_name(seg, Span::new(start, self.pos));
                    node = self.mk_list(vec![dot, node, key], Span::new(operand_start, self.pos));
                }
                _ => break,
            }
        }
        Ok(node)
    }

    fn read_list(&mut self) -> Result<StructId, ReadError> {
        // DEPTH GUARD: a list is the ONE recursive-descent point (`read_list` → `read_node` →
        // `read_primary` → `read_list`), so counting open lists bounds the native stack. Past the limit
        // return a clean diagnostic rather than recurse into a stack overflow (SIGABRT). Checked BEFORE
        // consuming `(` so the depth-limit error anchors at the offending open paren.
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(ReadError(format!(
                "expression nests too deeply to parse (more than {MAX_NESTING_DEPTH} levels) at byte {}",
                self.pos
            )));
        }
        self.depth += 1;
        let start = self.pos;
        self.bump(); // '('
        let mut items = Vec::new();
        let result = loop {
            self.skip_ws();
            match self.peek() {
                None => break Err(ReadError("unterminated list".into())),
                Some(b')') => {
                    self.bump();
                    // The list spans from `(` through the matching `)` (now consumed, so `self.pos` is
                    // just past).
                    break Ok(self.mk_list(items, Span::new(start, self.pos)));
                }
                Some(_) => match self.read_node() {
                    Ok(item) => items.push(item),
                    Err(e) => break Err(e),
                },
            }
        };
        self.depth -= 1;
        result
    }

    //= spec/capabilities/collections-and-text.md#a-string-literal-s-escapes-are-a-closed-set
    //# Within a string literal, a backslash MUST introduce an escape sequence rather than stand for itself.
    //= spec/capabilities/collections-and-text.md#a-string-literal-s-escapes-are-a-closed-set
    //# A conforming reader MUST recognize exactly these escape sequences: `\n` (U+000A), `\t` (U+0009), `\r` (U+000D), `\\` (U+005C), and `\"` (U+0022).
    //= spec/capabilities/collections-and-text.md#a-string-literal-s-escapes-are-a-closed-set
    //# A backslash followed by any character that does not begin one of the recognized escape sequences MUST be a compile-time error, so that an unrecognized escape is a rejected program rather than a silently-dropped backslash or an implementation-defined character.
    fn read_string(&mut self) -> Result<StructId, ReadError> {
        let start = self.pos;
        self.bump(); // opening quote
        let mut bytes: Vec<u8> = Vec::new();
        // The FIRST unrecognized escape encountered (`\q`), if any — a lexical defect the reader records
        // rather than reports (its stderr is not the diagnostic surface). The whole literal becomes a
        // `Leaf::BadEscape` marker the COMPILER rejects (CDZ0001); the reader still consumes to the closing
        // quote so the rest of the program parses (one diagnostic, not a cascade).
        let mut bad_escape: Option<char> = None;
        loop {
            match self.bump() {
                None => return Err(ReadError("unterminated string".into())),
                Some(b'"') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'n') => bytes.push(b'\n'),
                    Some(b't') => bytes.push(b'\t'),
                    Some(b'r') => bytes.push(b'\r'),
                    Some(b'\\') => bytes.push(b'\\'),
                    Some(b'"') => bytes.push(b'"'),
                    // An UNRECOGNIZED escape — the escape set is CLOSED (`\n \t \r \\ \"`). Record the
                    // offending char (first one wins) and keep the byte so the literal still terminates;
                    // the marker below overrides the value with a `BadEscape` the compiler rejects.
                    Some(other) => {
                        if bad_escape.is_none() {
                            bad_escape = Some(other as char);
                        }
                        bytes.push(other);
                    }
                    None => return Err(ReadError("unterminated escape".into())),
                },
                Some(b) => bytes.push(b),
            }
        }
        // A bad escape makes the whole literal a MARKER leaf — the compiler turns it into CDZ0001.
        if let Some(c) = bad_escape {
            return Ok(self.mk_atom_leaf(Leaf::BadEscape(c), Span::new(start, self.pos)));
        }
        let s = String::from_utf8(bytes).map_err(|_| ReadError("non-utf8 string".into()))?;
        // NFC-normalize string contents (the value form normalizes text) — so a string's identity is its
        // NORMALIZED contents: two literals with different byte spellings of the same text normalize to
        // one value and are therefore equal. Both the scalar length and the byte length therefore count
        // these normalized contents (a length is a function of the value, not its pre-normalization spelling).
        //= spec/capabilities/collections-and-text.md#string-equality-follows-normalized-contents
        //# Two strings MUST be equal exactly when their normalized contents are identical, under the text normalization the hashing-and-encoding choice pins.
        //= spec/capabilities/collections-and-text.md#a-string-offers-both-a-scalar-length-and-a-byte-length
        //# The scalar length and the byte length MUST count the string's normalized contents, so that a length is a function of the string's value rather than of an incidental byte spelling that normalization removes.
        let s: String = s.chars().nfc().collect();
        // The string atom spans the opening quote through the closing quote (now consumed).
        Ok(self.mk_atom_leaf(Leaf::Str(s), Span::new(start, self.pos)))
    }

    /// Read a symbol literal `#"meter"` into a `Leaf::Sym` — the interned-name value form. The `#` is
    /// consumed here; the body reuses the SAME string lexing (escapes `\n \t \r \\ \"`, NFC-normalized
    /// contents) as `read_string`, differing only in the leaf produced (`Sym` not `Str`) and the span
    /// (which includes the leading `#`). Its identity is its content (`symbol-interning-direction`); a
    /// base dimension is named this way (`(Unit.base #"meter")`).
    fn read_symbol(&mut self) -> Result<StructId, ReadError> {
        let start = self.pos;
        self.bump(); // '#'
        self.bump(); // opening quote
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match self.bump() {
                None => return Err(ReadError("unterminated symbol".into())),
                Some(b'"') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'n') => bytes.push(b'\n'),
                    Some(b't') => bytes.push(b'\t'),
                    Some(b'r') => bytes.push(b'\r'),
                    Some(b'\\') => bytes.push(b'\\'),
                    Some(b'"') => bytes.push(b'"'),
                    // A symbol reuses the string escape set. An unrecognized escape keeps the raw byte
                    // (a symbol names arbitrary content); the closed-escape-set diagnostic is a string
                    // concern, and a symbol literal is a name, not a text value with a lexical contract.
                    Some(other) => bytes.push(other),
                    None => return Err(ReadError("unterminated escape".into())),
                },
                Some(b) => bytes.push(b),
            }
        }
        let s = String::from_utf8(bytes).map_err(|_| ReadError("non-utf8 symbol".into()))?;
        // NFC-normalize symbol contents (identity is by normalized content — `symbol-interning-direction`
        // §String-backed: a symbol inherits String's normalized-contents equality).
        let s: String = s.chars().nfc().collect();
        // The symbol atom spans the leading `#` through the closing quote (now consumed).
        Ok(self.mk_atom_leaf(Leaf::Sym(s), Span::new(start, self.pos)))
    }

    /// Read a char literal `#\…` into a `Leaf::Char` (a single Unicode scalar). Three spellings:
    /// `#\a` / `#\é` (a single scalar), `#\space` / `#\newline` / `#\tab` / `#\return` / `#\null` (a
    /// named control char), and `#\u+HHHH` (a hex code point). A literal that spells a NON-scalar
    /// (`#\u+D800`, a surrogate; a code point past `U+10FFFF`) or an unknown name becomes a
    /// `Leaf::BadChar` MARKER — the compiler turns it into CDZ0002 (`collections-and-text.md` §A Char Is
    /// A Single Unicode Scalar Value); the reader is not the diagnostic surface. The atom spans `#\`
    /// through the end of the literal.
    fn read_char(&mut self) -> Result<StructId, ReadError> {
        let start = self.pos;
        self.bump(); // '#'
        self.bump(); // '\'
        let is_delim = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b';');
        // A char literal whose scalar IS a delimiter (`#\(`, `#\ `, `#\)`) is written with the raw
        // delimiter directly after `#\`; take exactly that one scalar. (Delimiters are ASCII, so a
        // leading multibyte scalar like `é` is never one.)
        if let Some(b) = self.peek()
            && is_delim(b)
        {
            let c = std::str::from_utf8(&self.src[self.pos..])
                .ok()
                .and_then(|s| s.chars().next());
            match c {
                Some(c) => {
                    self.pos += c.len_utf8();
                    return Ok(self.mk_atom_leaf(Leaf::Char(c), Span::new(start, self.pos)));
                }
                None => return Err(ReadError("unterminated char literal".into())),
            }
        }
        // Otherwise collect a WORD of non-delimiter bytes: a single scalar (`a`), a name (`newline`), or
        // a `u+HHHH` code point.
        let word_start = self.pos;
        while let Some(b) = self.peek() {
            if is_delim(b) {
                break;
            }
            self.pos += 1;
        }
        let word = std::str::from_utf8(&self.src[word_start..self.pos])
            .map_err(|_| ReadError("non-utf8 char literal".into()))?;
        if word.is_empty() {
            return Err(ReadError("empty char literal after `#\\`".into()));
        }
        let span = Span::new(start, self.pos);
        Ok(self.mk_atom_leaf(crate::literal::char_leaf(word), span))
    }

    /// Read a byte-string literal `b"…"` into a `Leaf::Bytes` (arbitrary bytes, NOT normalized as
    /// text). The escape vocabulary is the INVERSE of `literal::escape_bytes` (the render side): `\n \t
    /// \r \\ \"` are the named byte escapes, `\xNN` is a two-hex-digit byte, and any other `\c` keeps `c`
    /// verbatim (matching `read_string`'s lenient fallback). A raw byte stands for itself. `b"A\nB"`
    /// reads to `[65, 10, 66]`; `b"\x89PNG"` to `[137, 80, 78, 71]`. The atom spans `b"` through the
    /// closing quote.
    fn read_byte_string(&mut self) -> Result<StructId, ReadError> {
        let start = self.pos;
        self.bump(); // `b`
        self.bump(); // opening quote
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match self.bump() {
                None => return Err(ReadError("unterminated byte string".into())),
                Some(b'"') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'n') => bytes.push(b'\n'),
                    Some(b't') => bytes.push(b'\t'),
                    Some(b'r') => bytes.push(b'\r'),
                    Some(b'\\') => bytes.push(b'\\'),
                    Some(b'"') => bytes.push(b'"'),
                    // `\xNN` — exactly two hex digits, the byte they name.
                    Some(b'x') => {
                        let hi = self.bump();
                        let lo = self.bump();
                        match (hi.and_then(hex_digit), lo.and_then(hex_digit)) {
                            (Some(h), Some(l)) => bytes.push((h << 4) | l),
                            _ => {
                                return Err(ReadError(
                                    "a byte-string \\x escape needs two hex digits".into(),
                                ));
                            }
                        }
                    }
                    Some(other) => bytes.push(other),
                    None => return Err(ReadError("unterminated escape".into())),
                },
                Some(b) => bytes.push(b),
            }
        }
        Ok(self.mk_atom_leaf(Leaf::Bytes(bytes), Span::new(start, self.pos)))
    }

    fn read_atom_or_name(&mut self) -> Result<StructId, ReadError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b';') {
                break;
            }
            self.pos += 1;
        }
        let tok = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| ReadError("non-utf8 token".into()))?;
        Ok(self.classify_token(tok, start))
    }

    /// Classify a whitespace-delimited token into a leaf occurrence. A dotted token `a.b.c` is
    /// display sugar for nested member access `(. (. a b) c)`; otherwise the shared
    /// [`crate::literal::classify_word`] decides Int / Float / Bool / Name — the SAME layer the ML
    /// surface uses, so literal values are byte-identical across surfaces.
    fn classify_token(&mut self, tok: &str, start: usize) -> StructId {
        // A segmented identifier (`Sign.Neg`, `a.b.c`) desugars to nested member access. This is
        // checked before `classify_word` because a numeric literal (`3.5`) is not a dotted name
        // (its segments start with digits), so the two never conflict. Each segment's span is its
        // slice within `tok`; the `.` heads and the intermediate lists span the source consumed so
        // far, so `(. (. a b) c)` reads left-to-right with each list covering its own extent.
        if is_dotted_name(tok) {
            let mut off = start;
            let mut segs = tok.split('.');
            let first = segs.next().unwrap();
            let mut node = self.mk_name(first, Span::new(off, off + first.len()));
            off += first.len();
            for seg in segs {
                let dot_pos = off; // the '.' separator
                let seg_start = off + 1;
                let seg_end = seg_start + seg.len();
                let dot = self.mk_name(".", Span::new(dot_pos, dot_pos + 1));
                let seg_id = self.mk_name(seg, Span::new(seg_start, seg_end));
                node = self.mk_list(vec![dot, node, seg_id], Span::new(start, seg_end));
                off = seg_end;
            }
            return node;
        }
        let span = Span::new(start, start + tok.len());
        // Classify the word. A NUMBER/BOOL is a non-Name leaf (interned by value); a NAME is interned
        // by its `&str` slice via `leaf_name` — allocating an owned `String` only on a dedup MISS, not
        // for every occurrence (`classify_word` would `to_string()` the name eagerly and discard it on
        // a hit). `classify_word_nonname` returns `Some` only for the number/bool kinds, so a bare name
        // never allocates on the common repeated-identifier path.
        match crate::literal::classify_word_nonname(tok) {
            // A TYPE-SUFFIXED numeric literal (`100N`, `0.5R`) DESUGARS to the annotation `(: <literal>
            // BigInt|Rational)` — a suffix IS a terse annotation, so all typing/grounding reuses the
            // annotation path (and the compiler's codec decodes the `Suffixed` leaf straight to a plain
            // `Int`/`Float`, seeing exactly `(: 100 BigInt)`). The `Suffixed` atom is kept as the value
            // child so the PRINTER re-emits the suffix. The whole `(: … …)` list covers the token span.
            Some(leaf @ Leaf::Suffixed { kind, .. }) => {
                let colon = self.mk_name(":", span);
                let value = self.mk_atom_leaf(leaf, span);
                let ty = self.mk_name(kind.type_name(), span);
                self.mk_list(vec![colon, value, ty], span)
            }
            Some(leaf) => self.mk_atom_leaf(leaf, span),
            None => {
                let id = self.b.leaf_name(tok);
                self.mk_atom(id, span)
            }
        }
    }
}

/// True for an `a.b`(`.c…`) segmented identifier: at least one dot, every segment non-empty and
/// starting with a letter or `_` (so a float like `3.5` never reaches here — its segments are
/// digit-led).
fn is_dotted_name(tok: &str) -> bool {
    if !tok.contains('.') {
        return false;
    }
    let segs: Vec<&str> = tok.split('.').collect();
    if segs.len() < 2 {
        return false;
    }
    segs.iter().all(|s| {
        !s.is_empty()
            && s.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
    })
}

/// The value `0..=15` of an ASCII hex digit byte (`0-9`, `a-f`, `A-F`), or `None`. Used to decode a
/// byte-string `\xNN` escape.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Decimal, Radix};
    use num_bigint::BigInt;
    use std::str::FromStr;

    #[test]
    fn reads_a_form() {
        let a = read("(+ 1 2)").unwrap();
        assert_eq!(a.head_name(a.root), Some("+"));
    }

    #[test]
    fn deeply_nested_input_is_diagnosed_not_crashed() {
        // A pathologically deep but syntactically valid nest overflowed the native stack (SIGABRT) in
        // the unguarded recursive descent; the depth guard makes it a clean `ReadError` instead. The
        // depth here (limit + a margin) far exceeds `MAX_NESTING_DEPTH` — without the guard this
        // recursion would abort the process (the real crash needs ~25000, but the guard fires at the
        // limit, so a modest over-limit depth exercises it deterministically without a huge string).
        let n = (MAX_NESTING_DEPTH as usize) + 50;
        let src = format!("{}1{}", "(+ ".repeat(n), " 1)".repeat(n));
        let err = read(&src).expect_err("deep nesting must be a clean error, not a crash");
        assert!(
            err.0.contains("nests too deeply"),
            "expected a depth-limit diagnostic, got: {}",
            err.0
        );
        // Just UNDER the limit still parses (the guard does not reject a valid moderate nest). A depth
        // of `limit - 1` open lists is within budget.
        let ok = (MAX_NESTING_DEPTH as usize) - 1;
        let shallow = format!("{}1{}", "(+ ".repeat(ok), " 1)".repeat(ok));
        assert!(
            read(&shallow).is_ok(),
            "a nest just under the limit must still parse"
        );
    }

    /// The text a span covers, for span assertions.
    fn slice(src: &str, s: Span) -> &str {
        &src[s.start..s.end]
    }

    #[test]
    fn spanned_arena_is_identical_to_untracked() {
        // Span tracking must not perturb the arena — it is the round-trip oracle, so the tracked
        // and untracked paths MUST build byte-identical arenas (same leaves, structure, root).
        for src in [
            "(+ 1 2)",
            "(let ((p (record (x 1) (y 2)))) (. p x))",
            "(match e ((Some n) n) ((None _) 0))",
            "(Int 8).max",
            "Sign.Neg",
            "(quasiquote (unquote x))",
            "(f `(a ,b ,@c))",
            "\"a string\"",
        ] {
            let plain = read(src).unwrap();
            let (tracked, spans) = read_spanned(src).unwrap();
            assert_eq!(plain, tracked, "arena differs for {src:?}");
            // The table is total and 1:1 with the arena.
            assert_eq!(
                spans.len(),
                tracked.structure.len(),
                "span table not 1:1 for {src:?}"
            );
        }
    }

    #[test]
    fn spans_cover_the_source_text_of_each_node() {
        let src = "(case foo (needs bar) baz)";
        let (a, spans) = read_spanned(src).unwrap();
        // Root list spans the whole form.
        assert_eq!(slice(src, spans.get(a.root).unwrap()), src);
        // The `(needs bar)` child (index 2 of the root list) spans exactly that sub-form.
        let Struct::List(items) = a.get(a.root) else {
            panic!("root is a list");
        };
        let needs = items[2];
        assert_eq!(a.head_name(needs), Some("needs"));
        assert_eq!(slice(src, spans.get(needs).unwrap()), "(needs bar)");
        // The head atom `case` spans its 4 bytes.
        assert_eq!(slice(src, spans.get(items[0]).unwrap()), "case");
    }

    #[test]
    fn read_all_spanned_wraps_and_spans_each_top_form() {
        let src = "(a 1)\n(b 2)\n";
        let (a, spans) = read_all_spanned(src).unwrap();
        // The synthetic `(do …)` root spans the whole input.
        assert_eq!(a.head_name(a.root), Some("do"));
        assert_eq!(slice(src, spans.get(a.root).unwrap()), src);
        let Struct::List(items) = a.get(a.root) else {
            panic!("root is a list");
        };
        // items[0] is the synthetic `do`; items[1]/[2] are the two forms with their own spans.
        assert_eq!(slice(src, spans.get(items[1]).unwrap()), "(a 1)");
        assert_eq!(slice(src, spans.get(items[2]).unwrap()), "(b 2)");
    }

    #[test]
    fn spans_are_correct_for_a_nested_member_access() {
        // `(Int 8).max` desugars to `(. (Int 8) max)`; the operand span must cover `(Int 8)`, the
        // key `max`, and the whole postfix list the full `(Int 8).max`.
        let src = "(Int 8).max";
        let (a, spans) = read_spanned(src).unwrap();
        assert_eq!(a.head_name(a.root), Some("."));
        assert_eq!(slice(src, spans.get(a.root).unwrap()), "(Int 8).max");
        let tail = a.as_form(a.root, ".").unwrap();
        assert_eq!(slice(src, spans.get(tail[0]).unwrap()), "(Int 8)");
        assert_eq!(slice(src, spans.get(tail[1]).unwrap()), "max");
    }

    /// print∘read is stable: reading printed text yields a structurally-equal arena, and printing
    /// it again is byte-identical (the s-expr surface is its own canonical form).
    #[test]
    fn print_reads_back() {
        for src in [
            "(+ 1 2)",
            "(let ((p (record (x 1) (y 2)))) (. p x))",
            "(match e ((Some n) n) ((None _) 0))",
            "42",
            "0x2A",
            "1.5",
            "-0.25",
            "\"a\\nb\"",
            "true",
            "(f a b c)",
            "(quasiquote (unquote x))",
            // Byte-string literals `b"…"`: printable-ASCII raw, named + `\xNN` escapes, empty.
            "b\"ABC\"",
            "b\"\\x89PNG\"",
            "b\"A\\nB\"",
            "b\"\"",
        ] {
            let a = read(src).unwrap();
            let printed = print(&a);
            let b = read(&printed).unwrap();
            assert_eq!(
                print(&b),
                printed,
                "print∘read stable for {src:?} (printed {printed:?})"
            );
        }
    }

    #[test]
    fn reads_a_byte_string_literal_into_bytes() {
        // `b"…"` is a byte-string literal → `Leaf::Bytes` (the companion of `"…"` → `Leaf::Str`).
        // Named escapes, `\xNN` hex (incl. a high byte ≥ 0x80), and the empty literal all decode.
        for (src, want) in [
            ("b\"ABC\"", vec![65u8, 66, 67]),
            ("b\"\\x89PNG\"", vec![137, 80, 78, 71]),
            ("b\"A\\nB\"", vec![65, 10, 66]),
            ("b\"\"", vec![]),
        ] {
            let a = read(src).unwrap();
            match a.get(a.root) {
                Struct::Atom(l) => match a.leaf(*l) {
                    Leaf::Bytes(b) => assert_eq!(b, &want, "bytes for {src:?}"),
                    other => panic!("{src:?} read as {other:?}, not Leaf::Bytes"),
                },
                _ => panic!("{src:?} is not an atom"),
            }
        }
        // A bare `b` is still an ordinary NAME — only the exact `b"` prefix opens a byte string.
        let a = read("b").unwrap();
        assert_eq!(a.as_name(a.root), Some("b"));
    }

    /// A form that fits the width stays on one line — pretty output matches the single-line print.
    #[test]
    fn pretty_keeps_small_forms_on_one_line() {
        for src in ["(+ 1 2)", "(f a b c)", "42", "(. p x)"] {
            let a = read(src).unwrap();
            assert_eq!(print_pretty_width(&a, 80), print(&a), "for {src:?}");
        }
    }

    /// A form too wide for the target width breaks: the head hugs the `(`, each child drops to its
    /// own line indented one level, and the closing `)` hugs the last child.
    #[test]
    fn pretty_breaks_wide_forms_one_child_per_line() {
        // The outer `match` overflows width 20 so it breaks one child per line, but each arm fits on
        // its own line and stays flat — breaking is per-box, only where a form actually overflows.
        let a = read("(match e ((Some n) n) ((None _) 0))").unwrap();
        assert_eq!(
            print_pretty_width(&a, 20),
            "(match\n  e\n  ((Some n) n)\n  ((None _) 0))"
        );
    }

    /// A broken top-level `(do …)` blank-separates its statements; a broken `(module …)` blank-
    /// separates its members (but the head/name attach normally).
    #[test]
    fn pretty_blank_separates_top_level_and_module_members() {
        // read_all wraps top-level forms in a synthetic `(do …)`.
        let a = read_all("(def (a x) (+ x 1)) (def (b y) (* y 2))").unwrap();
        assert_eq!(
            print_pretty_width(&a, 25),
            "(do\n  (def (a x) (+ x 1))\n\n  (def (b y) (* y 2)))"
        );
        // A module blank-separates members, keeping `module` and the name attached.
        let a = read("(module m (type T A B) (def (a x) x) (def (b y) y))").unwrap();
        assert_eq!(
            print_pretty_width(&a, 25),
            "(module\n  m\n  (type T A B)\n\n  (def (a x) x)\n\n  (def (b y) y))"
        );
    }

    /// A NESTED `(do …)` — a function body, not the root — keeps its statements tightly single-
    /// broken (no blank lines); only the ROOT statement sequence blank-separates.
    #[test]
    fn pretty_nested_do_is_not_blank_separated() {
        let a = read("(def (f x) (do (foo x) (bar x) (baz x)))").unwrap();
        assert_eq!(
            print_pretty_width(&a, 15),
            "(def\n  (f x)\n  (do\n    (foo x)\n    (bar x)\n    (baz x)))"
        );
    }

    /// print_pretty ∘ read is stable AND structurally faithful: the pretty layout re-reads to the
    /// same arena the single-line print does (whitespace is the only difference), and pretty-
    /// printing the re-read tree is idempotent.
    #[test]
    fn pretty_reads_back_to_the_same_arena() {
        for src in [
            "(+ 1 2)",
            "(let ((p (record (x 1) (y 2)))) (. p x))",
            "(match e ((Some n) n) ((None _) 0))",
            "(f a b c)",
            "(quasiquote (unquote x))",
            "\"a\\nb\"",
        ] {
            let a = read(src).unwrap();
            // A tight width forces breaks so the layout logic is actually exercised.
            let pretty = print_pretty_width(&a, 10);
            let b = read(&pretty).unwrap();
            assert_eq!(
                a, b,
                "pretty re-read differs for {src:?} (pretty:\n{pretty})"
            );
            // idempotent at the same width
            assert_eq!(
                print_pretty_width(&b, 10),
                pretty,
                "pretty not idempotent for {src:?}"
            );
        }
    }

    #[test]
    fn bigint_no_ceiling() {
        let a = read("123456789012345678901234567890").unwrap();
        let Struct::Atom(l) = a.get(a.root) else {
            panic!()
        };
        match a.leaf(*l) {
            Leaf::Int { value, radix } => {
                assert_eq!(
                    value,
                    &BigInt::from_str("123456789012345678901234567890").unwrap()
                );
                assert_eq!(*radix, Radix::Dec);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn radix_literals() {
        for (src, val, radix) in [
            ("0x2A", 42, Radix::Hex),
            ("0b101010", 42, Radix::Bin),
            ("-0x10", -16, Radix::Hex),
        ] {
            let a = read(src).unwrap();
            let Struct::Atom(l) = a.get(a.root) else {
                panic!()
            };
            assert_eq!(
                a.leaf(*l),
                &Leaf::Int {
                    value: BigInt::from(val),
                    radix
                },
                "src {src}"
            );
        }
    }

    #[test]
    fn exact_float() {
        let a = read("1.5").unwrap();
        let Struct::Atom(l) = a.get(a.root) else {
            panic!()
        };
        assert_eq!(
            a.leaf(*l),
            &Leaf::Float(Decimal {
                negative: false,
                significand: BigInt::from(15),
                exponent: -1
            })
        );
    }

    #[test]
    fn exponent_float() {
        let a = read("1.5e10").unwrap();
        let Struct::Atom(l) = a.get(a.root) else {
            panic!()
        };
        // 15 * 10^(10-1) = 15e9
        assert_eq!(
            a.leaf(*l),
            &Leaf::Float(Decimal {
                negative: false,
                significand: BigInt::from(15),
                exponent: 9
            })
        );
    }

    #[test]
    fn malformed_separator_is_name_not_dropped() {
        // `1_` is not a well-formed int and not a float — it stays a Name (rejected downstream),
        // never silently read as the value 1.
        let a = read("1_").unwrap();
        assert_eq!(a.as_name(a.root), Some("1_"));
    }

    #[test]
    fn dotted_name_desugars() {
        let a = read("Sign.Neg").unwrap();
        // (. Sign Neg)
        assert_eq!(a.head_name(a.root), Some("."));
        let tail = a.as_form(a.root, ".").unwrap();
        assert_eq!(a.as_name(tail[0]), Some("Sign"));
        assert_eq!(a.as_name(tail[1]), Some("Neg"));
    }

    #[test]
    fn postfix_member_after_a_paren_desugars() {
        // `(Int 8).max` reads to `(. (Int 8) max)` — the paren-postfix sibling of `Int8.max`. This is
        // what lets a type-constructor application be projected directly (the modules `(Int N)` builds
        // carry `max`/`min`/`wrap`), reading identically to the aliased-name form.
        let a = read("(Int 8).max").unwrap();
        assert_eq!(a.head_name(a.root), Some("."));
        let tail = a.as_form(a.root, ".").unwrap();
        // operand is the `(Int 8)` application; key is `max`.
        assert_eq!(a.head_name(tail[0]), Some("Int"));
        assert_eq!(a.as_name(tail[1]), Some("max"));
    }

    #[test]
    fn postfix_member_chains_and_composes_with_application() {
        // `((. (UInt 48) wrap) -1)` is unaffected (explicit form), and a chained postfix `(Int 8).x.y`
        // nests left-to-right: `(. (. (Int 8) x) y)`.
        let a = read("(Int 8).x.y").unwrap();
        assert_eq!(a.head_name(a.root), Some("."));
        let outer = a.as_form(a.root, ".").unwrap();
        assert_eq!(a.as_name(outer[1]), Some("y"));
        assert_eq!(a.head_name(outer[0]), Some(".")); // inner (. (Int 8) x)
        let inner = a.as_form(outer[0], ".").unwrap();
        assert_eq!(a.head_name(inner[0]), Some("Int"));
        assert_eq!(a.as_name(inner[1]), Some("x"));
    }

    #[test]
    fn dot_head_form_is_not_a_postfix() {
        // `(. p x)` — a `.` that heads a list (with a following space) is ordinary member-access
        // structure, NOT a postfix on the preceding token. Pins that the postfix only fires on a `.`
        // glued to an identifier segment.
        let a = read("(. p x)").unwrap();
        assert_eq!(a.head_name(a.root), Some("."));
        let tail = a.as_form(a.root, ".").unwrap();
        assert_eq!(a.as_name(tail[0]), Some("p"));
        assert_eq!(a.as_name(tail[1]), Some("x"));
    }

    #[test]
    fn digit_separators_ok() {
        let a = read("1_000_000").unwrap();
        let Struct::Atom(l) = a.get(a.root) else {
            panic!()
        };
        assert_eq!(
            a.leaf(*l),
            &Leaf::Int {
                value: BigInt::from(1_000_000),
                radix: Radix::Dec
            }
        );
    }

    #[test]
    fn an_unknown_string_escape_reads_as_a_bad_escape_marker() {
        // The escape set is CLOSED (`\n \t \r \\ \"`); `\q` begins none of them, so the reader emits a
        // `Leaf::BadEscape('q')` MARKER (not silently `q`) — the compiler turns it into CDZ0001. A VALID
        // escape still reads to its `Str`. (The reader does not error: its stderr is not the diagnostic
        // surface, so the defect must ride the AST to the compiler.)
        let a = read("\"\\q\"").unwrap();
        let Struct::Atom(l) = a.get(a.root) else {
            panic!("expected an atom")
        };
        assert_eq!(a.leaf(*l), &Leaf::BadEscape('q'));
        let b = read("\"\\n\"").unwrap();
        let Struct::Atom(l) = b.get(b.root) else {
            panic!("expected an atom")
        };
        assert_eq!(b.leaf(*l), &Leaf::Str("\n".to_string()));
    }

    #[test]
    fn a_bad_escape_marker_round_trips_through_the_codec() {
        // The marker must survive the binary AST codec (encode→decode) unchanged, so the compiler that
        // reads the binary AST sees the same `BadEscape` the reader produced.
        let a = read("\"\\q\"").unwrap();
        let bytes = crate::codec::encode(&a);
        let b = crate::codec::decode(&bytes).expect("decode");
        let Struct::Atom(l) = b.get(b.root) else {
            panic!("expected an atom")
        };
        assert_eq!(b.leaf(*l), &Leaf::BadEscape('q'));
    }

    #[test]
    fn reads_char_literals_in_each_spelling() {
        // `#\c` — a single scalar; `#\newline` — a named control char; `#\u+HHHH` — a code point. A
        // surrogate / out-of-range code point becomes a `BadChar` MARKER (the compiler rejects CDZ0002).
        let leaf_of = |src: &str| {
            let a = read(src).unwrap();
            let Struct::Atom(l) = a.get(a.root) else {
                panic!("expected an atom for {src}")
            };
            a.leaf(*l).clone()
        };
        assert_eq!(leaf_of("#\\a"), Leaf::Char('a'));
        assert_eq!(leaf_of("#\\é"), Leaf::Char('é'));
        assert_eq!(leaf_of("#\\newline"), Leaf::Char('\n'));
        assert_eq!(leaf_of("#\\space"), Leaf::Char(' '));
        assert_eq!(leaf_of("#\\u+0061"), Leaf::Char('a'));
        assert_eq!(leaf_of("#\\u+00E9"), Leaf::Char('é'));
        // A surrogate is not a scalar → a BadChar marker carrying the literal text.
        assert_eq!(leaf_of("#\\u+D800"), Leaf::BadChar("u+D800".to_string()));
        // A code point past U+10FFFF is likewise a BadChar.
        assert_eq!(
            leaf_of("#\\u+110000"),
            Leaf::BadChar("u+110000".to_string())
        );
    }

    #[test]
    fn char_leaves_round_trip_through_the_codec() {
        // A `Char` and a `BadChar` must survive the binary AST codec unchanged (the compiler reads the
        // binary AST, so it must see the same leaf the reader produced).
        for src in ["#\\a", "#\\newline", "#\\u+D800"] {
            let a = read(src).unwrap();
            let bytes = crate::codec::encode(&a);
            let b = crate::codec::decode(&bytes).expect("decode");
            assert!(a.structurally_eq(&b), "codec round-trip changed {src}");
        }
    }

    #[test]
    fn char_literals_round_trip_through_the_printer() {
        // Printing a char leaf and re-reading yields the SAME leaf (the `#\…` render is `char_leaf`'s
        // inverse). Covers the bare-scalar, named-control, and code-point render paths.
        for src in ["#\\a", "#\\é", "#\\newline", "#\\space"] {
            let a = read(src).unwrap();
            let printed = print(&a);
            let b = read(&printed).unwrap();
            assert!(
                a.structurally_eq(&b),
                "{src} printed as {printed:?} did not round-trip"
            );
        }
    }

    #[test]
    fn reads_symbol_literals() {
        // `#"meter"` is a symbol literal → `Leaf::Sym` (distinct from `Leaf::Str` and `Leaf::Name`);
        // its content is NFC-normalized and the string escape set applies.
        let leaf_of = |src: &str| {
            let a = read(src).unwrap();
            let Struct::Atom(l) = a.get(a.root) else {
                panic!("expected an atom for {src}")
            };
            a.leaf(*l).clone()
        };
        assert_eq!(leaf_of("#\"meter\""), Leaf::Sym("meter".to_string()));
        assert_eq!(leaf_of("#\"\""), Leaf::Sym(String::new())); // the empty symbol
        assert_eq!(
            leaf_of("#\"a b\""),
            Leaf::Sym("a b".to_string()) // a symbol may carry spaces (it is not an identifier)
        );
        // A symbol is NOT a string and NOT a name.
        assert_ne!(leaf_of("#\"meter\""), Leaf::Str("meter".to_string()));
        assert_ne!(leaf_of("#\"meter\""), Leaf::Name("meter".to_string()));
    }

    #[test]
    fn symbol_leaves_round_trip_through_codec_and_printer() {
        // A `Sym` must survive BOTH the binary AST codec (the compiler reads the binary AST) and the
        // printer (`read(print(x)) == x`) — the two gates the `#"…"` literal must hold for the units
        // corpus surface (`(Unit.base #"meter")`).
        for src in ["#\"meter\"", "#\"second\"", "#\"\""] {
            let a = read(src).unwrap();
            // Codec round-trip.
            let bytes = crate::codec::encode(&a);
            let b = crate::codec::decode(&bytes).expect("decode");
            assert!(a.structurally_eq(&b), "codec round-trip changed {src}");
            // Printer round-trip.
            let printed = print(&a);
            assert_eq!(printed, src, "{src} did not print back verbatim");
            let c = read(&printed).unwrap();
            assert!(a.structurally_eq(&c), "printer round-trip changed {src}");
        }
    }
}
