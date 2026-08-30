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

use cadenza_syntax_core::ast::{Arenas, Builder, CompoundCtor, Leaf, LeafId, Struct, StructId};
use cadenza_syntax_core::doc::Doc;
use cadenza_syntax_core::span::Span;
use cadenza_syntax_core::spans::{FileId, SpanTable};
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
pub use cadenza_syntax_core::MAX_NESTING_DEPTH;

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
    // An EXPLICIT work stack, not native recursion: `print` runs on arenas from ANY source — a decoded
    // binary AST in particular, which `codec::decode` accepts at arbitrary nesting depth (no cap, unlike
    // the reader's `MAX_NESTING_DEPTH`). A recursive walk overflowed the native stack (SIGABRT) on such a
    // deep-but-valid tree, crashing the process on `cdz convert binary → sexpr`; the printer must stay
    // total. `Node(id)` renders an occurrence; `Str(" ")`/`Str(")")` are the separators/closers queued
    // AFTER a list's children. Items pop LIFO, so a list pushes (in reverse): `)`, then child_n, sep,
    // …, child_0 — yielding `(` child_0 ` ` child_1 … `)` in output order.
    enum Work<'a> {
        Node(StructId),
        Str(&'a str),
    }
    let mut stack: Vec<Work> = vec![Work::Node(id)];
    while let Some(w) = stack.pop() {
        match w {
            Work::Str(s) => out.push_str(s),
            Work::Node(id) => match a.get(id) {
                Struct::Atom(l) => print_leaf(a.leaf(*l), out),
                Struct::List(items) => {
                    // RESUGAR: a `(: <suffixed-literal> BigInt|Rational)` node is the desugared form of a
                    // type suffix (`100N`), so print just the suffixed atom — the suffix already carries
                    // the type. (A bare `(: 100 BigInt)` value-output, whose value child is a plain `Int`
                    // not a `Suffixed`, is NOT matched, so it still prints the explicit annotation.)
                    if let Some(atom) = suffixed_annotation_atom(a, items) {
                        stack.push(Work::Node(atom));
                        continue;
                    }
                    // RESUGAR native compound heads (M2) back to their surface so a re-read reproduces the
                    // SAME native node: a ctor-leaf head → `#word(child…)`, a `FieldPair` → `(= k v)`, a
                    // `Member` → `(. obj key)`. Recognized by leaf KIND, not head text.
                    if let Some(ctor) = a.compound_ctor_leaf(id) {
                        out.push('#');
                        out.push_str(compound_ctor_word(ctor));
                        out.push('(');
                        stack.push(Work::Str(")"));
                        for (i, &child) in items[1..].iter().enumerate().rev() {
                            stack.push(Work::Node(child));
                            if i > 0 {
                                stack.push(Work::Str(" "));
                            }
                        }
                        continue;
                    }
                    if let Some((k, v)) = a.field_pair_parts(id) {
                        out.push_str("(= ");
                        stack.push(Work::Str(")"));
                        stack.push(Work::Node(v));
                        stack.push(Work::Str(" "));
                        stack.push(Work::Node(k));
                        continue;
                    }
                    if let Some((obj, key)) = a.member_parts(id) {
                        out.push_str("(. ");
                        stack.push(Work::Str(")"));
                        stack.push(Work::Node(key));
                        stack.push(Work::Str(" "));
                        stack.push(Work::Node(obj));
                        continue;
                    }
                    out.push('(');
                    // Push in reverse: closing paren first (popped last), then children interleaved with
                    // single-space separators so they pop child_0, " ", child_1, …, ")".
                    stack.push(Work::Str(")"));
                    for (i, &child) in items.iter().enumerate().rev() {
                        stack.push(Work::Node(child));
                        if i > 0 {
                            stack.push(Work::Str(" "));
                        }
                    }
                }
            },
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
pub const DEFAULT_WIDTH: usize = cadenza_syntax_core::DEFAULT_WIDTH;

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

/// Pretty-print a top-level PROGRAM for DISPLAY: if the root is a `(do …)` — the synthetic wrapper
/// [`read_all`] adds around a bare multi-form file, or a user `do` at the root — print its member forms
/// as FLUSH-LEFT, blank-line-separated siblings, with NO `(do …)` wrapper and NO indentation. A program
/// reads as a stacked list of top-level definitions, so the synthetic `do` is elided and its members sit
/// at column 0 (the shape the guide displays). A non-`do` root prints as the ordinary single form.
///
/// Distinct from [`print_pretty_width`], which faithfully SHOWS the `(do …)` structure (each member
/// indented one level under the head) — that is the right rendering when the `do` IS the program's
/// structure (e.g. `cdz convert --to sexpr`), but wrong for displaying a bare-defs file, where stripping
/// the wrapper text alone would leave the members mis-indented (first flush, rest at the `do`'s 2-space
/// indent). Use this for displayed program code; use `print_pretty_width` to show the arena faithfully.
pub fn print_pretty_program(arenas: &Arenas, width: usize) -> String {
    if arenas.head_name(arenas.root) == Some("do")
        && let Struct::List(items) = arenas.get(arenas.root)
        && items.len() > 1
    {
        // Each member printed at column 0 (its own box), blank-line-separated — flush-left siblings.
        let members: Vec<StructId> = items[1..].to_vec();
        return members
            .iter()
            .map(|&m| print_pretty_from(arenas, m, width))
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    print_pretty_width(arenas, width)
}

/// Render occurrence `id`. `top` marks a declaration-level position (the root, or a module body) —
/// where a top-level `(do …)` form-sequence blank-separates its members. It is cleared for every
/// nested child, so a `(do …)` used as a function body deeper in the tree keeps its statements
/// tightly single-broken. A `module` blank-separates its members at ANY depth (a module body is
/// always a declaration list).
fn pretty_node(a: &Arenas, root: StructId, doc: &mut Doc, root_top: bool) {
    // An EXPLICIT work stack, not native recursion: `pretty_node` BUILDS the Oppen `Doc` token stream by
    // walking the arena — one frame per nesting level in the recursive form — and `print_pretty` runs on
    // arenas from ANY source, including a decoded binary AST that `codec::decode` accepts at ARBITRARY
    // depth (no cap, unlike the reader's `MAX_NESTING_DEPTH`). A recursive build overflowed the native
    // stack (SIGABRT) on a deep tree, crashing `cdz convert binary → sexpr` (pretty is the default). The
    // token stream this emits is byte-identical to the recursive version's — only the driver differs.
    enum Work {
        // Render an occurrence (its subtree). `top` carries the `(do …)` root-sequence flag down.
        Node(StructId, bool),
        // Deferred literal Doc ops, queued AFTER a list's opening `(` + head so they fire in order.
        OpenSpace,
        OpenBlank,
        // A HARD break (always fires) — forces the enclosing consistent box to break. Used for the
        // head→first-definition separator of a TOP-LEVEL `(do …)` / `module`, so a program lays out
        // VERTICALLY (each definition on its own line) even when it would fit `width` — a program reads
        // as a stacked list of defs, not a wrapped paragraph. One hardbreak breaks the whole do/module
        // box, so its soft `OpenBlank` def-separators fire too; each def's OWN box stays flat if it fits.
        OpenHardBreak,
        // A NON-breaking literal space — renders ` ` always, never a newline (unlike `OpenSpace`, which
        // fires as a break when its consistent box breaks). Used to keep a `module`'s NAME hugging its head
        // (`(module m`) even though the surrounding box is force-broken by the definition list below it.
        OpenAttach,
        CloseParen, // emits `word(")")` then `end()` — the box closer paired with each `cbox`+`(`.
    }
    let mut stack: Vec<Work> = vec![Work::Node(root, root_top)];
    while let Some(w) = stack.pop() {
        match w {
            Work::OpenSpace => doc.space(),
            Work::OpenBlank => blank_line(doc),
            Work::OpenHardBreak => doc.hardbreak(),
            Work::OpenAttach => doc.word(" "),
            Work::CloseParen => {
                doc.word(")");
                doc.end();
            }
            Work::Node(id, top) => match a.get(id) {
                Struct::Atom(l) => {
                    let mut s = String::new();
                    print_leaf(a.leaf(*l), &mut s);
                    doc.word(s);
                }
                Struct::List(items) => {
                    // The reader never produces an empty list; render defensively as `()`.
                    if items.is_empty() {
                        doc.word("()");
                        continue;
                    }
                    // RESUGAR a desugared type-suffix `(: <suffixed> BigInt|Rational)` to the bare `100N`
                    // atom (same rule as the single-line printer, so both round-trip identically).
                    if let Some(atom) = suffixed_annotation_atom(a, items) {
                        stack.push(Work::Node(atom, false));
                        continue;
                    }
                    // RESUGAR a native ctor-leaf head (M2) to its `#word(child…)` surface: the normal list
                    // path below would print `(list …)` (a name-application that does NOT re-read to the
                    // ctor leaf), so a compound literal needs its own opener. A `FieldPair`/`Member` head
                    // needs no special case — the normal path + `print_leaf` render them `(= k v)` /
                    // `(. obj key)` (the head atom prints as `=` / `.`). Mirrors the single-line printer.
                    if let Some(ctor) = a.compound_ctor_leaf(id) {
                        doc.cbox(INDENT);
                        doc.word(format!("#{}(", compound_ctor_word(ctor)));
                        stack.push(Work::CloseParen);
                        // children `items[1..]`: the first hugs the `#word(` opener (no leading space), the
                        // rest get an inter-child break. Push in REVERSE for source order.
                        for (i, &child) in items.iter().enumerate().skip(1).rev() {
                            stack.push(Work::Node(child, false));
                            if i > 1 {
                                stack.push(Work::OpenSpace);
                            }
                        }
                        continue;
                    }
                    // A consistent box: `(head child…)` stays flat when it fits `width`, else EVERY inter-
                    // child break fires, so each child lands on its own line indented one level under the
                    // head. The head hugs the `(`; the closing `)` hugs the last child (no dangling paren).
                    // Emit the opener NOW; queue the closer + children (reversed) to run in source order.
                    doc.cbox(INDENT);
                    doc.word("(");
                    // The MEMBERS of a top-level form sequence (`do`) or a `module` are definitions — a
                    // single break between them reads as a crammed wall. Separate them with a BLANK line
                    // (materializes only when the box breaks; a fitting sequence stays one line with plain
                    // spaces). The `do`/`module` HEAD, and a module's NAME, still attach with an ordinary
                    // break — only definition-to-definition gets the blank line. A nested `do` (top
                    // cleared) is a statement block, so it stays tightly single-broken.
                    let blank_sep_from = match a.head_name(id) {
                        Some("do") if top => 1,
                        Some("module") => 2,
                        _ => usize::MAX,
                    };
                    // Push in REVERSE so the stack pops head, sep, child1, sep, child2, …, ) in order.
                    let is_module = a.head_name(id) == Some("module");
                    stack.push(Work::CloseParen);
                    for (i, &child) in items.iter().enumerate().skip(1).rev() {
                        stack.push(Work::Node(child, false));
                        stack.push(if i > blank_sep_from {
                            Work::OpenBlank
                        } else if i == blank_sep_from {
                            // The head→FIRST-DEFINITION separator of a top-level `do`/`module`
                            // (`blank_sep_from` is that first-def index; `usize::MAX` for a nested `do`, so
                            // this never fires there). A HARD break here forces the whole do/module box to
                            // break, laying the program out vertically regardless of `width`; the def
                            // separators (`OpenBlank`) then fire too, while each def's own box stays flat.
                            Work::OpenHardBreak
                        } else if is_module && i == 1 {
                            // A module's NAME hugs its head (`(module m`) with a non-breaking space — so the
                            // force-break above (which breaks the whole consistent box) does not push the name
                            // onto its own line.
                            Work::OpenAttach
                        } else {
                            Work::OpenSpace
                        });
                    }
                    stack.push(Work::Node(items[0], false));
                }
            },
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
        Leaf::Int { value, radix } => {
            out.push_str(&cadenza_syntax_core::literal::render_int(value, *radix))
        }
        Leaf::Float(d) => out.push_str(&cadenza_syntax_core::literal::render_decimal(d)),
        // Non-finite float VALUES render `nan`/`inf`/`-inf`. These leaves are produced only by
        // `Ast.encode` of a computed float, NEVER by the reader (which reads a source `nan`/`inf`
        // identifier to a `Name`), so a value-DISPLAY spelling is enough; a round-tripping source
        // literal for them is a separate surface slice.
        Leaf::FloatNan => out.push_str("nan"),
        Leaf::FloatInf { negative } => out.push_str(if *negative { "-inf" } else { "inf" }),
        Leaf::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Leaf::Str(s) => {
            out.push('"');
            out.push_str(&cadenza_syntax_core::literal::escape_string(s));
            out.push('"');
        }
        // A byte sequence renders `b"…"` — the byte-string form (printable ASCII raw, else `\xNN`).
        Leaf::Bytes(b) => {
            out.push_str("b\"");
            out.push_str(&cadenza_syntax_core::literal::escape_bytes(b));
            out.push('"');
        }
        // A name is written verbatim. (The s-expr surface has no reserved words — `let`, `+`, `|`
        // are all ordinary atoms — so no escaping is needed here, unlike the ML surface.)
        Leaf::Name(n) => out.push_str(n),
        // A symbol renders `#"…"` (reusing the string escape set) — re-reads to the same `Leaf::Sym`.
        Leaf::Sym(s) => {
            out.push_str("#\"");
            out.push_str(&cadenza_syntax_core::literal::escape_string(s));
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
        Leaf::Char(c) => out.push_str(&cadenza_syntax_core::literal::render_char(*c)),
        // A bad-char MARKER round-trips to `#\<text>` — re-reading re-detects the malformed literal.
        Leaf::BadChar(s) => {
            out.push_str("#\\");
            out.push_str(s);
        }
        // A TYPE-SUFFIXED literal renders `<body><suffix>` (`100N`, `0.5R`) — re-reads to the same leaf.
        Leaf::Suffixed { value, kind } => {
            out.push_str(&cadenza_syntax_core::literal::render_suffixed(value, *kind))
        }
        // A native compound HEAD leaf (M2) is a LIST head, resugared at the list level (`print_node`) and
        // never printed as a bare atom in a well-formed tree; render a best-effort marker for a stray
        // atom occurrence so the printer stays total.
        Leaf::Ctor(c) => out.push_str(compound_ctor_word(*c)),
        Leaf::FieldPair => out.push('='),
        Leaf::Member => out.push('.'),
    }
}

/// The reserved surface word for a compound constructor — the inverse of the reader's `#word(` → ctor
/// mapping, used by the s-expr printers to resugar a `Leaf::Ctor` head back to `#word(…)`. `pub` (not
/// `pub(crate)`) because after the #5082 sexpr-move the ML printer (cadenza-syntax) resugars a `Ctor`
/// head via `crate::sexpr::compound_ctor_word` CROSS-CRATE (through the facade's `pub use
/// cadenza_syntax_sexpr as sexpr`), so it must be visible outside this crate.
pub fn compound_ctor_word(ctor: CompoundCtor) -> &'static str {
    match ctor {
        CompoundCtor::Record => "record",
        CompoundCtor::Tuple => "tuple",
        CompoundCtor::List => "list",
        CompoundCtor::Map => "map",
        CompoundCtor::Set => "set",
    }
}

// ============================================================================
// M3 SOURCE NATIVIZATION (throwaway migration aid; deleted at M3 Phase-2 completion).
//
// Rewrite name-head compound LITERALS/PATTERNS — `(list …)`/`(tuple …)`/`(record …)`/`(set …)`/`(map …)`
// — to the native `#word(…)` surface across a WHOLE s-expr program source, for the guide-source
// nativization (v-guide-infra drives it per extracted source via the `cdz-nativize` bin, stdin→stdout).
// Span-based surgical edit over the exact reader: surface (comments/formatting/digit-separators) is byte-
// preserved, and strings/comments/char-literals/existing native `#word(` forms are untouched. Shadow-aware
// (a `let`/`fn`/`def`-bound ctor name — a user `(def (map …) …)` — stays name-head), and `map` is HOF-guarded
// (a `(map (\ …) coll)` / `(map inc xs)` HOF CALL is left name-head — only genuine map literals/patterns,
// whose children are all `(k v)`/`(= k v)` entries or a `..` rest, are nativized + their 2-element entries
// field-paired). Handles a BARE multi-form snippet (`read_all` wraps it in a synthetic `(do …)`).
// ============================================================================

/// A shadowable compound-ctor head NAME (the aliases the native `#word(` head replaces).
fn nat_is_ctor_name(n: &str) -> bool {
    matches!(n, "list" | "tuple" | "record" | "map" | "set")
}

/// Push `x`'s name into `out` if it is a ctor NAME (a bare-atom binder that shadows a ctor).
fn nat_push_ctor_name(a: &Arenas, x: StructId, out: &mut Vec<String>) {
    if let Some(n) = a.as_name(x)
        && nat_is_ctor_name(n)
    {
        out.push(n.to_string());
    }
}

/// Push a `fn`/`def` PARAM's binder name (bare atom, or `(: name T)`) if it shadows a ctor.
fn nat_push_param(a: &Arenas, p: StructId, out: &mut Vec<String>) {
    if a.as_name(p).is_some() {
        nat_push_ctor_name(a, p, out);
    } else if let Struct::List(pc) = a.get(p)
        && a.head_name(p) == Some(":")
        && pc.len() >= 2
    {
        nat_push_ctor_name(a, pc[1], out);
    }
}

/// The ctor NAMES a `let`/`fn`/`def` form BINDS (shadowing the ctor for its subtree).
fn nat_collect_binders(a: &Arenas, id: StructId, out: &mut Vec<String>) {
    let Struct::List(ch) = a.get(id) else {
        return;
    };
    match a.head_name(id) {
        Some("let") if ch.len() >= 2 => {
            if let Struct::List(binds) = a.get(ch[1]) {
                for &b in binds {
                    if let Some(n) = a.head_name(b)
                        && nat_is_ctor_name(n)
                    {
                        out.push(n.to_string());
                    }
                }
            }
        }
        Some("fn") if ch.len() >= 2 => {
            if let Struct::List(params) = a.get(ch[1]) {
                for &p in params {
                    nat_push_param(a, p, out);
                }
            }
        }
        Some("def") if ch.len() >= 2 => {
            if let Struct::List(sig) = a.get(ch[1]) {
                if let Some(&f) = sig.first() {
                    nat_push_ctor_name(a, f, out);
                }
                for &p in sig.iter().skip(1) {
                    nat_push_param(a, p, out);
                }
            }
        }
        _ => {}
    }
}

/// Whether a name-head `map` node is a LITERAL/PATTERN (vs a HOF `(map f coll)` call): every non-head child
/// must be an ENTRY (a 2-element `(k v)` or a 3-element FieldPair `(= k v)`) or a REST indicator (`..` / the
/// bare name immediately after it). A lambda / bare-atom arg not in rest position ⇒ HOF ⇒ not eligible.
fn nat_map_eligible(a: &Arenas, ch: &[StructId]) -> bool {
    let mut prev_dd = false;
    for &c in ch.iter().skip(1) {
        match a.get(c) {
            Struct::List(gc) => {
                let ok = gc.len() == 2
                    || (gc.len() == 3
                        && (a.head_name(c) == Some("=")
                            || matches!(a.get(gc[0]), Struct::Atom(_))
                                && a.head_name(c).is_none()));
                if !ok {
                    return false;
                }
                prev_dd = false;
            }
            Struct::Atom(_) => match a.as_name(c) {
                Some("..") => prev_dd = true,
                Some(_) if prev_dd => prev_dd = false,
                _ => return false,
            },
        }
    }
    true
}

/// True if the form starting at byte `form_start` carries a `; cdz-nativize-exempt: …` line-comment on the
/// line IMMEDIATELY above it (the form must begin its own line after only indentation). This is the per-form
/// exemption marker (operator-approved): the codemod leaves the marked form + subtree untouched, and
/// v-corpus-harness's `nativize-check` (which shares this walk) then passes idempotence on it. Tolerant of a
/// `;` or `;;` comment lead and surrounding whitespace.
fn nat_form_is_exempt(bytes: &[u8], form_start: usize) -> bool {
    // Back up over the form's own leading indentation (spaces/tabs) on its line.
    let mut i = form_start.min(bytes.len());
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    // The form must sit at line start (only ws before it); the char before is the '\n' ending the PREVIOUS
    // line. Otherwise there is no "line immediately above" to carry the marker.
    if i == 0 || bytes[i - 1] != b'\n' {
        return false;
    }
    let prev_line_end = i - 1; // the '\n' ending the previous line
    let mut ls = prev_line_end;
    while ls > 0 && bytes[ls - 1] != b'\n' {
        ls -= 1;
    }
    let line = &bytes[ls..prev_line_end];
    let mut j = 0;
    while j < line.len() && matches!(line[j], b' ' | b'\t') {
        j += 1;
    }
    if j >= line.len() || line[j] != b';' {
        return false;
    }
    while j < line.len() && line[j] == b';' {
        j += 1;
    }
    while j < line.len() && matches!(line[j], b' ' | b'\t') {
        j += 1;
    }
    line[j..].starts_with(b"cdz-nativize-exempt:")
}

/// Recurse `id`, collecting head-nativize + map-entry-field-pairify edits (start, end, replacement) into
/// `edits`, tracking ctor-name `shadow` scopes.
#[allow(clippy::too_many_arguments)]
fn nat_walk(
    a: &Arenas,
    spans: &SpanTable,
    bytes: &[u8],
    id: StructId,
    shadow: &mut std::collections::HashMap<String, u32>,
    exempt: &mut std::collections::HashSet<StructId>,
    skip_outputs: bool,
    collect: bool,
    in_wit: bool,
    edits: &mut Vec<(usize, usize, String)>,
) {
    let Struct::List(ch) = a.get(id) else {
        return;
    };
    let ch = ch.clone();
    // PER-FORM EXEMPTION (operator-approved, seq marker): a line-comment `; cdz-nativize-exempt: <reason>`
    // immediately above a form marks it (and its whole subtree) as DELIBERATELY non-native — leave it
    // untouched. The single source of truth for both the codemod (won't rewrite) and v-corpus-harness's
    // `nativize-check` (idempotence passes). Used for transitional NAME-HEAD parity cases that guard a
    // name-head-path fix (e.g. corpus-05 #6047 guards the #6042 ML/paren-surface hang path) — the name-head
    // path is live until the reader-flip, so nativizing them would destroy the coverage. Skip nativize +
    // recursion for the marked form's subtree (a marked corpus `(case …)` exempts every literal within it).
    if let Some(sp) = spans.get(id)
        && nat_form_is_exempt(bytes, sp.start)
    {
        return;
    }
    // A `(wit-world …)` clause is a WIT interface/TYPE declaration, not a compound-VALUE context. Its
    // lowercase `record`/`list`/… heads are WIT TYPE descriptors (`(record (= x (s64)))`, `(list (u8))`),
    // NOT value literals — nativizing them is out of M3's value-literal scope AND regresses: the type
    // parser accepts native heads (`wit_world.rs` `head_ctor`), but the imposed-WIT-world reducer path
    // DECLINES a native-headed type descriptor (corpus-28 case "…via an imposed WIT world" pass→Todo,
    // verified 2026-08-30). So exempt the whole `(wit-world …)`/`(world …)` subtree from head-nativize (a
    // wit-world holds no value literals, so nothing legitimate is skipped). Mirrors the handler-arm-op
    // exemption. Genuine compound VALUES in a wit-boundary case sit OUTSIDE the wit-world clause and still
    // nativize.
    let in_wit = in_wit || matches!(a.head_name(id), Some("wit-world" | "world"));
    // CORPUS inputs-only mode (`skip_outputs`): a corpus case's `(output …)` expected VALUE must match the
    // GATE RENDER exactly, and the render NORMALIZES (Ast.List→`(. Ast List)`, Qty.of, `#"sym"`, map/set key
    // order, Bytes `b"…"`) — a text-nativize would NOT reproduce those, so `(output …)` is v-corpus-harness's
    // grade-driven re-pin, NOT ours. So STOP collecting edits inside an `(output …)` subtree; everything else
    // — `(input …)` programs, `(call …)` argument values (input-side) — still nativizes. (Whole-program mode
    // for the guide passes `skip_outputs=false`, so `collect` never flips and every literal nativizes.)
    let collect = collect && !(skip_outputs && a.head_name(id) == Some("output"));
    let mut introduced = Vec::new();
    nat_collect_binders(a, id, &mut introduced);
    for n in &introduced {
        *shadow.entry(n.clone()).or_insert(0) += 1;
    }
    // An effect HANDLER's op-handler arm — `(handle <effect> <seed> ((<op> (params…) <state> <body>)…)
    // <body>)`, the canonical 5-child surface shape — names its operation BARE at each arm's head. When an
    // op is named after a compound ctor (`set`/`list`/`map`/`tuple`/`record` — a State effect's `set` is the
    // real case), that head is NOT a compound literal and must NOT be nativized (it would corrupt the arm
    // into `#set(…)` and break the handler). Exempt each arm CLAUSE node from head-nativize (its body is
    // still walked, so a genuine literal inside it nativizes). Mirrors the HOF/shadow head guards.
    if a.head_name(id) == Some("handle")
        && ch.len() == 5
        && let Struct::List(arm_nodes) = a.get(ch[3])
    {
        for &arm in arm_nodes {
            exempt.insert(arm);
        }
    }
    // The compound ctor at the head: a NAME head `(list …)` (shadowable — a bound `list` suppresses it) OR
    // the STRING-primitive head `("list" …)` (the unshadowable "strings are the symbols" escape form, used
    // in the corpus where a local binding shadows the name alias, e.g. `(let ((tuple …)) ("tuple" 7 8))`).
    // BOTH must nativize to `#word(…)` for M3 — the native ctor-leaf is likewise unshadowable, so
    // `("tuple" …)` → `#tuple(…)` preserves the unshadowable ctor identity. head_name is None for a string
    // head, so fall back to the head atom's string value.
    let name_head = a.head_name(id);
    // `ch.first()`, NOT `ch[0]` — an EMPTY list `()` has no head (indexing would panic).
    let ctor_name = name_head.or_else(|| ch.first().and_then(|&h| a.as_str(h)));
    if collect
        && !in_wit
        && let Some(name) = ctor_name
        && nat_is_ctor_name(name)
        // A NAME head is suppressed by a shadowing binding; a STRING head is unshadowable (always the ctor).
        && (name_head.is_none() || shadow.get(name).copied().unwrap_or(0) == 0)
        && !exempt.contains(&id)
        && (name != "map" || nat_map_eligible(a, &ch))
    {
        // Head-nativize `(name`/`("name"` → `#name(`, consuming the head→first-child HORIZONTAL whitespace.
        let ls = spans.get(id).expect("list span");
        let hs = spans.get(ch[0]).expect("head span");
        let mut end = hs.end;
        while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
            end += 1;
        }
        edits.push((ls.start, end, alloc_head(name)));
        // A map/record's 2-element POSITIONAL entries `(k v)` → FieldPair `(= k v)` (insert `= ` after the
        // entry's `(`) — the canonical native entry form (map values #5120 + map patterns #5310, record
        // fields #5120). A 3-element entry is already FieldPair; list/tuple/set have elements, not entries.
        if matches!(name, "map" | "record") {
            for &entry in ch.iter().skip(1) {
                // A construction SPREAD `(.. v)` (the wrapped rest/spread node, #5838/#5826) is ALSO a
                // 2-element list, but it is NOT a `(k v)` entry — FieldPair-ifying it to `(= .. v)` would
                // invent a field named `..` and corrupt the spread. Leave spread entries untouched.
                if a.head_name(entry) == Some("..") {
                    continue;
                }
                if let Struct::List(ec) = a.get(entry)
                    && ec.len() == 2
                {
                    let es = spans.get(entry).expect("entry span");
                    edits.push((es.start + 1, es.start + 1, "= ".to_string()));
                }
            }
        }
    }
    for &c in ch.iter() {
        nat_walk(
            a,
            spans,
            bytes,
            c,
            shadow,
            exempt,
            skip_outputs,
            collect,
            in_wit,
            edits,
        );
    }
    for n in &introduced {
        if let Some(v) = shadow.get_mut(n) {
            *v -= 1;
        }
    }
}

fn alloc_head(name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 2);
    s.push('#');
    s.push_str(name);
    s.push('(');
    s
}

/// Nativize every name-head compound LITERAL/PATTERN in an s-expr program `src` to the native `#word(…)`
/// surface (see the module comment above). `Err` if `src` does not parse. The transform is behavior-
/// preserving (a native ctor-leaf head is `structurally_eq` to its name-alias) and surface-preserving
/// (only the target head bytes change). The M3 guide-source migration entry (`cdz-nativize` bin wraps it).
pub fn nativize_compound_source(src: &str) -> Result<String, ReadError> {
    nativize_compound_impl(src, false)
}

/// Like [`nativize_compound_source`] but SKIPS every `(output …)` subtree — the CORPUS inputs-only mode
/// for the M3 Phase-2 corpus migration. A corpus case's `(output …)` expected value must match the GATE
/// RENDER exactly (which normalizes `Ast.List`→`(. Ast List)`, `Qty.of`, `#"sym"`, map/set key order,
/// `Bytes`→`b"…"`), so text-nativizing it would MISMATCH the render — the `(output …)` side is
/// v-corpus-harness's grade-driven render re-pin, not this codemod's. This nativizes the `(input …)`
/// programs + `(call …)` argument values (both input-side) and leaves `(output …)` untouched, so the two
/// passes compose without clobbering (sequence Option A, DESIGN §13.4).
pub fn nativize_compound_source_skip_outputs(src: &str) -> Result<String, ReadError> {
    nativize_compound_impl(src, true)
}

fn nativize_compound_impl(src: &str, skip_outputs: bool) -> Result<String, ReadError> {
    let (arenas, spans) = read_all_spanned(src)?;
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut shadow: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut exempt: std::collections::HashSet<StructId> = std::collections::HashSet::new();
    nat_walk(
        &arenas,
        &spans,
        src.as_bytes(),
        arenas.root,
        &mut shadow,
        &mut exempt,
        skip_outputs,
        true, // collect edits from the root down (flips off only inside an (output …) when skip_outputs)
        false, // in_wit: not inside a (wit-world …) type-descriptor subtree at the root
        &mut edits,
    );
    edits.sort_by_key(|e| core::cmp::Reverse(e.0)); // descending: apply back-to-front, offsets stay valid
    let mut out = src.to_string();
    for (start, end, repl) in &edits {
        out.replace_range(*start..*end, repl);
    }
    Ok(out)
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
            // A COLLECTION LITERAL `#word(…)` — the explicit-constructor s-expr surface for a compound
            // (`DESIGN-native-ast-compound-data.md` §D-SURFACE), one uniform rule for all five: the body
            // is read VERBATIM and the ctor word names the head — `#list(1 2 3)`, `#tuple(a b)`,
            // `#set(a b c)`, and `#record((= x 1) (= y 2))` / `#map((= k v) …)` (a record field / map
            // entry is written as its explicit `(= key value)` `FieldPair`, the same node every other
            // surface produces, so it round-trips and comments compose around it as ordinary nodes — no
            // implicit `=` insertion). Reads to the str-head ctor form (`("word" …)`, the `compound_ctor`
            // tag), unambiguously distinct from an application `(list …)`. Input-only for now: the printer
            // still emits the `(word …)` head until the corpus migrates, so no corpus round-trip is
            // affected. Only the exact `#word(` prefix opens one; a bare `#` or `#other` reads ordinarily.
            Some(b'#') if self.compound_literal_word().is_some() => {
                let word = self
                    .compound_literal_word()
                    .expect("guarded by compound_literal_word().is_some()");
                self.read_compound_literal(word)
            }
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
                    let dot = self.mk_atom_leaf(Leaf::Member, Span::new(dot_pos, dot_pos + 1));
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
                    // just past). An explicit `(. obj key)` list reads to a native `Member` head.
                    let span = Span::new(start, self.pos);
                    // A bare-NAME `record` compound-alias head (the shadowable prelude alias the pattern
                    // reader + corpus author, `DESIGN-native-ast-compound-data.md`) field-pairifies its
                    // DIRECT `(= k v)` entries just like `#record(…)` — so `(record (= x a))` reads to the
                    // SAME `Name`-head + `FieldPair`-field arena `read_ml`'s record-pattern reader emits.
                    self.alias_field_pairify(&mut items);
                    let id = self.mk_list(items, span);
                    break Ok(self.memberize(id, span));
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

    /// At a `#`, the `#word(…)` collection-literal ctor word (`list`/`tuple`/`record`/`map`/`set`)
    /// immediately followed by `(`, if any. A bare `#`, or `#word` not followed by `(`, or `#other(`, is
    /// not a collection literal (`None`) and falls through to ordinary reading. None of the five words is
    /// a prefix of another, so order is moot.
    fn compound_literal_word(&self) -> Option<&'static str> {
        let rest = self.src.get(self.pos + 1..)?;
        ["record", "tuple", "list", "map", "set"]
            .into_iter()
            .find(|word| rest.starts_with(word.as_bytes()) && rest.get(word.len()) == Some(&b'('))
    }

    /// Read a `#word(…)` collection literal into the native ctor-LEAF-KIND form `(<ctor> child…)` — the
    /// M2 native-compound-data surface (`DESIGN-native-ast-compound-data.md`): the head is a
    /// [`Leaf::Ctor`] recognized by kind identity, not a `Str`/`Name` head text. The body is read the
    /// same way for all five ctors; a `#record`/`#map` DIRECT entry written `(= key value)` has its `=`
    /// head rewritten to the [`Leaf::FieldPair`] marker (ruling A — the stored AST is native end-to-end),
    /// while equality `=` anywhere else stays an ordinary `Name("=")`. Comments compose around a field as
    /// ordinary nodes (a comment-wrapped field is descended, see [`field_pairify`]).
    fn read_compound_literal(&mut self, word: &'static str) -> Result<StructId, ReadError> {
        // Same recursion bound as `read_list`: `#word(…)` bodies descend through `read_node` too.
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(ReadError(format!(
                "expression nests too deeply to parse (more than {MAX_NESTING_DEPTH} levels) at byte {}",
                self.pos
            )));
        }
        self.depth += 1;
        let start = self.pos; // at '#'
        // `#` + the ctor word are ASCII, so advancing by their byte length lands on the '(' cleanly.
        self.pos += 1 + word.len();
        let ctor = match word {
            "record" => CompoundCtor::Record,
            "tuple" => CompoundCtor::Tuple,
            "list" => CompoundCtor::List,
            "map" => CompoundCtor::Map,
            "set" => CompoundCtor::Set,
            _ => unreachable!("compound_literal_word yields only the five ctor words"),
        };
        // The native ctor LEAF KIND names the constructor (recognized by kind identity); span the head
        // over the `#word` prefix.
        let head = self.mk_atom_leaf(Leaf::Ctor(ctor), Span::new(start, self.pos));
        self.bump(); // '('
        // In a #record/#map body a DIRECT `(= k v)` entry is a FieldPair; other ctors read their body
        // verbatim (elements are positional).
        let keyed = matches!(ctor, CompoundCtor::Record | CompoundCtor::Map);
        let mut items = vec![head];
        let result = loop {
            self.skip_ws();
            match self.peek() {
                None => {
                    break Err(ReadError(format!(
                        "unterminated `#{word}( … )` at byte {}",
                        self.pos
                    )));
                }
                Some(b')') => {
                    self.bump();
                    break Ok(self.mk_list(items, Span::new(start, self.pos)));
                }
                Some(_) => match self.read_node() {
                    Ok(item) => {
                        let item = if keyed {
                            self.field_pairify(item)
                        } else {
                            item
                        };
                        items.push(item);
                    }
                    Err(e) => break Err(e),
                },
            }
        };
        self.depth -= 1;
        result
    }

    /// The recorded span of an already-built node (full range), or a zero-width span at `self.pos` when
    /// not tracking — used when rebuilding a node with a native head (FieldPair/Member), so the rebuilt
    /// occurrence carries the original's source range.
    fn span_of(&self, id: StructId) -> Span {
        match self.spans.as_ref().and_then(|t| t.get(id)) {
            Some(s) => Span::new(s.start, s.end),
            None => Span::new(self.pos, self.pos),
        }
    }

    /// Rewrite a `#record`/`#map` DIRECT body entry into its native form (ruling A): a `(= k v)` list —
    /// exactly two args after the `=` head — is rebuilt with a [`Leaf::FieldPair`] head (reusing `k`,`v`;
    /// the old `Name("=")` atom + list are left unreferenced and dropped by `canon` on encode). A
    /// `(comment …)` wrapper is descended — each child is field-pairified and the wrapper preserved — so
    /// a comment-wrapped field `(comment "doc" (= x 1))` becomes `(comment "doc" (<field-pair> x 1))`.
    /// Any other item (a bare `=` of the wrong arity, a positional value, equality inside a field VALUE)
    /// is returned unchanged: `field_pairify` only rewrites the DIRECT entry head, so `(= x (= a b))`
    /// keeps its inner equality as `Name("=")`.
    fn field_pairify(&mut self, item: StructId) -> StructId {
        // A direct `(= k v)` — exactly two args.
        let eq_kv = self
            .b
            .as_form(item, "=")
            .filter(|tail| tail.len() == 2)
            .map(|tail| (tail[0], tail[1]));
        if let Some((k, v)) = eq_kv {
            let span = self.span_of(item);
            let fp = self.mk_atom_leaf(Leaf::FieldPair, span);
            return self.mk_list(vec![fp, k, v], span);
        }
        // A `(comment …)` wrapper — descend, field-pairifying each non-head child (nested comments
        // recurse; the `comment` head and any doc-string child are unchanged).
        if self.b.as_form(item, "comment").is_some() {
            let orig: Vec<StructId> = match self.b.get(item) {
                Struct::List(items) => items.clone(),
                Struct::Atom(_) => return item,
            };
            let span = self.span_of(item);
            let mut rebuilt = Vec::with_capacity(orig.len());
            rebuilt.push(orig[0]); // the `comment` head, preserved
            for &child in &orig[1..] {
                let field = self.field_pairify(child);
                rebuilt.push(field);
            }
            return self.mk_list(rebuilt, span);
        }
        item
    }

    /// If `items` is a bare-NAME `record` compound-alias body (`(record …)` — the shadowable prelude
    /// alias the pattern reader + corpus author, distinct from the explicit `#record(…)` ctor surface),
    /// rewrite each DIRECT `(= k v)` entry to a [`Leaf::FieldPair`] in place, exactly as `#record(…)`
    /// does. This lets the shadowable alias surface read to the SAME `Name`-head + `FieldPair`-field arena
    /// `read_ml`'s record reader emits (value + pattern record fields are the canonical `=` FieldPair,
    /// operator ruling: full symmetry). A non-`record` head (or entries that are not `(= k v)`) is left
    /// untouched — `map` alias entries stay bare `(k v)` pairs (its pattern surface), and equality `=`
    /// elsewhere stays `Name("=")`.
    fn alias_field_pairify(&mut self, items: &mut [StructId]) {
        // A `record` OR `map` compound-alias head — the shadowable NAME alias (`(record …)`) OR the
        // unshadowable STRING primitive (`("record" …)`, which Ast-metaprogramming / value-reification
        // emit). Both spell their entries as the canonical `(= k v)` FieldPair in the native arena (a map
        // VALUE entry and a record field are the same `=` node, unified in M2), so a `(= k v)` DIRECT
        // child under either field-pairifies. A map PATTERN's entries are bare `(k p)` pairs (no `=`), so
        // `field_pairify` leaves them untouched.
        let head_word = items
            .first()
            .and_then(|&h| self.b.as_name(h).or_else(|| self.b.as_str(h)));
        if !matches!(head_word, Some("record") | Some("map")) {
            return;
        }
        for slot in items.iter_mut().skip(1) {
            *slot = self.field_pairify(*slot);
        }
    }

    /// Rewrite an explicit member-access list `(. obj key)` — head `Name(".")`, exactly two args — to a
    /// native [`Leaf::Member`] head (ruling A; member access is native everywhere). Any other `.` arity
    /// or a non-`.` list is returned unchanged. The old `Name(".")` atom + list are left unreferenced
    /// (dropped by `canon`). The postfix (`obj.key`) and dotted-token (`a.b`) sugars build a `Member`
    /// head directly at their own sites.
    fn memberize(&mut self, id: StructId, span: Span) -> StructId {
        let dot_kv = self
            .b
            .as_form(id, ".")
            .filter(|tail| tail.len() == 2)
            .map(|tail| (tail[0], tail[1]));
        if let Some((obj, key)) = dot_kv {
            let dot = self.mk_atom_leaf(Leaf::Member, span);
            return self.mk_list(vec![dot, obj, key], span);
        }
        id
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
        Ok(self.mk_atom_leaf(Leaf::Str(s.into()), Span::new(start, self.pos)))
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
        Ok(self.mk_atom_leaf(Leaf::Sym(s.into()), Span::new(start, self.pos)))
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
        Ok(self.mk_atom_leaf(cadenza_syntax_core::literal::char_leaf(word), span))
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
        Ok(self.mk_atom_leaf(Leaf::Bytes(bytes.into()), Span::new(start, self.pos)))
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
    /// [`cadenza_syntax_core::literal::classify_word`] decides Int / Float / Bool / Name — the SAME layer the ML
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
                let dot = self.mk_atom_leaf(Leaf::Member, Span::new(dot_pos, dot_pos + 1));
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
        match cadenza_syntax_core::literal::classify_word_nonname(tok) {
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
    use cadenza_syntax_core::ast::{Decimal, Radix};
    use num_bigint::BigInt;
    use std::str::FromStr;

    #[test]
    fn reads_a_form() {
        let a = read("(+ 1 2)").unwrap();
        assert_eq!(a.head_name(a.root), Some("+"));
    }

    #[test]
    fn nativize_compound_source_nativizes_literals_and_guards_hof_shadow() {
        let n = |s: &str| super::nativize_compound_source(s).unwrap();
        // Bare single-form literals of each kind → native head.
        assert_eq!(n("(list 1 2)"), "#list(1 2)");
        assert_eq!(n("(tuple a b)"), "#tuple(a b)");
        assert_eq!(n("(record (= x 1))"), "#record((= x 1))");
        // A record's 2-element POSITIONAL entry is field-paired as the head nativizes.
        assert_eq!(n("(record (x 1) (y 2))"), "#record((= x 1) (= y 2))");
        assert_eq!(n("(set 1 2)"), "#set(1 2)");
        assert_eq!(n("(map (= 1 2))"), "#map((= 1 2))");
        // A map's 2-element positional entries are field-paired as the head nativizes.
        assert_eq!(n("(map (1 2) (3 4))"), "#map((= 1 2) (= 3 4))");
        // Nesting + a map REST pattern.
        assert_eq!(n("(list (tuple 1 2))"), "#list(#tuple(1 2))");
        assert_eq!(n("(map (1 v) .. rest)"), "#map((= 1 v) .. rest)");
        // Empty forms.
        assert_eq!(n("(list)"), "#list()");
        assert_eq!(n("(map)"), "#map()");
        // HOF `map` calls are NOT nativized (lambda arg; bare-atom args).
        assert_eq!(n("(map (\\ (x) x) xs)"), "(map (\\ (x) x) xs)");
        assert_eq!(n("(map inc xs)"), "(map inc xs)");
        // A `let`/`def`-shadowed ctor name stays name-head (its uses are calls to the bound value).
        assert_eq!(
            n("(let ((list (fn (a b) a))) (list 1 2))"),
            "(let ((list (fn (a b) a))) (list 1 2))"
        );
        assert_eq!(
            n("(do (def (map k) (map (= 0 k))) (export map))"),
            "(do (def (map k) (map (= 0 k))) (export map))"
        );
        // Multi-form (bare) input → each form nativized independently; surface (spacing) preserved.
        assert_eq!(n("(list 1)  (tuple 2 3)"), "#list(1)  #tuple(2 3)");
        // A full module with a pattern match nativizes both value and pattern heads; comments/strings safe.
        assert_eq!(
            n(
                "(do (def (f xs) (match xs (#list() 0) ((list h .. t) h))) (export f)) ; (list x) in a comment"
            ),
            "(do (def (f xs) (match xs (#list() 0) (#list(h .. t) h))) (export f)) ; (list x) in a comment"
        );
        // A doc-string mentioning (map k v) must not be touched.
        assert_eq!(
            n("(def (g) \"(list x) in a string\")"),
            "(def (g) \"(list x) in a string\")"
        );
    }

    #[test]
    fn a_top_level_program_lays_out_vertically_even_when_it_fits() {
        // Operator (a)-canonical requirement (the guide DISPLAYS canonical sexpr): a top-level program is a
        // stacked list of definitions, so a `(do …)` / `module` lays out VERTICALLY — each member on its own
        // line, blank-separated — regardless of whether the whole program would fit `width` on one line. A
        // NESTED `do` (a statement block / function body) stays width-driven. Each member's own box still
        // stays flat when it fits (a short def is one line).
        let src = "(do (def (main) (f 5)) (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1))) (export main))";
        let a = read(src).unwrap();
        // At width 100 the whole program (~90 chars) WOULD fit on one line — but it must still break.
        let out = print_pretty_width(&a, 100);
        assert_eq!(
            out,
            "(do\n  (def (main) (f 5))\n\n  (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))\n\n  (export main))",
            "top-level program must lay out vertically at width 100:\n{out}"
        );
        // Each member's OWN box stays flat when it fits — the `(def (main) (f 5))` and the `@` member are
        // each one line (only the do-level broke), so this is readable, not over-broken.
        assert!(
            out.contains("  (def (main) (f 5))\n"),
            "member stays flat:\n{out}"
        );
        // Re-reads to the same arena (formatting is whitespace-insensitive) — round-trip preserved.
        assert!(
            read(&out).unwrap().structurally_eq(&a),
            "vertical layout re-reads identically"
        );
        // A NESTED `do` (a def body) is NOT force-broken — it stays on one line when it fits.
        let nested = read("(def (f) (do (g) (h)))").unwrap();
        assert_eq!(
            print_pretty_width(&nested, 100),
            "(def (f) (do (g) (h)))",
            "a nested do (statement block) stays width-driven, not force-broken"
        );
    }

    #[test]
    fn print_pretty_program_renders_top_level_defs_flush_left_no_do_wrapper() {
        // OPERATOR seq-256 (render-fidelity): a bare multi-form program (`read_all` wraps it in a synthetic
        // `(do …)`) must DISPLAY as FLUSH-LEFT, blank-separated top-level siblings — NO `(do …)` wrapper and
        // NO indentation. The bug this fixes: stripping the `(do` wrapper text from `print_pretty_width`'s
        // output left the members mis-indented (first flush at col 0, the rest at the do's 2-space indent);
        // `print_pretty_program` elides the synthetic wrapper and prints each member at column 0 instead.
        let a = read_all("(def (dbl (: x Int64)) (* x 2)) (def (main) (dbl 21))").unwrap();
        assert_eq!(
            print_pretty_program(&a, 100),
            "(def (dbl (: x Int64)) (* x 2))\n\n(def (main) (dbl 21))",
            "top-level defs are flush-left, blank-separated, no `(do …)` wrapper"
        );
        // No spurious indent on ANY line (the operator's exact complaint) + re-reads to the same program.
        let out = print_pretty_program(&a, 100);
        assert!(
            out.lines().all(|l| !l.starts_with("  (def")),
            "no top-level def is indented:\n{out}"
        );
        assert!(
            read_all(&out).unwrap().structurally_eq(&a),
            "flush-left program re-reads identically"
        );
        // `print_pretty_width` STILL shows the `(do …)` faithfully (this is a display-only variant).
        assert!(
            print_pretty_width(&a, 100).starts_with("(do\n"),
            "print_pretty_width still shows the (do …) structure"
        );
        // A single top-level form (no synthetic do) prints as itself.
        let one = read("(def (main) 1)").unwrap();
        assert_eq!(print_pretty_program(&one, 100), "(def (main) 1)");
    }

    #[test]
    fn a_compound_value_renders_and_round_trips_as_the_native_ctor_form() {
        // DRIFT GUARD (requested by v-rust-backend; protects #5586 + its bytes-second render_val + the M3
        // name-head-alias removal): the CANONICAL render of a compound VALUE is the native `#ctor` form
        // (#record/#list/#tuple/#map/#set), NOT a name-head `(record …)` (which is a TYPE descriptor's
        // spelling, seq-206 — a distinct, orthogonal axis). A `#ctor` VALUE must (a) render `#ctor` compact,
        // (b) render `#ctor` pretty (the `cdz convert --to sexpr` path), (c) be idempotent, and (d) survive a
        // binary encode→decode round-trip as `#ctor` (the bytes-second path). If a future change flips the
        // value render back to name-head, this fails HERE.
        for form in [
            "#record((= a 5) (= b 2))",
            "#list(1 2 3)",
            "#tuple(1 2)",
            "#map((= 1 2) (= 3 4))",
            "#set(1 2 3)",
            "#list(#tuple(1 2) #record((= x 9)))", // nested compounds render #ctor at every level
        ] {
            let a = read(form).unwrap();
            let ctor_head = form.split('(').next().unwrap(); // `#record` / `#list` / …
            // (a) compact render is the byte-identical `#ctor` form.
            assert_eq!(print(&a), form, "compact render of {form}");
            // (b) pretty render (the convert-to-sexpr path) also emits the `#ctor` head.
            let pretty = print_pretty_width(&a, 80);
            assert!(
                pretty.contains(ctor_head),
                "pretty render of {form} must keep the #ctor head, got {pretty:?}"
            );
            // (c) idempotent: read(print(x)) prints the same canonical form.
            assert_eq!(
                print(&read(&print(&a)).unwrap()),
                form,
                "idempotent render of {form}"
            );
            // (d) binary encode→decode round-trip stays `#ctor` (bytes-second / value-encode path).
            let bytes = cadenza_ast::codec::encode(&a);
            let back = cadenza_ast::codec::decode(&bytes).expect("compound value decodes");
            assert_eq!(print(&back), form, "binary round-trip of {form}");
        }
    }

    #[test]
    fn a_construction_spread_round_trips_through_the_sexpr_surface() {
        // The construction spread `.. s` (seq-204 GO, all four collection ctors) reaches the corpus/harness
        // through the SEXPR surface, so the reader + printer must round-trip it as the flat `.. <operand>`
        // marker at ANY position (multiple/interior — a CONSTRUCTION spread, unlike a trailing-only PATTERN
        // rest). Read -> compact print -> byte-identical, and encode->decode->print stays stable. (The
        // LOWERING of a construction spread is v-inference's slice — this pins the surface the corpus writes.)
        for form in [
            "#list(0 .. c 9)",
            "#list(.. a .. b)",
            "#set(.. a x)",
            "#set(1 .. a 2 .. b)",
            "#map(.. m (= 1 2))",
            "#map((= 1 2) .. m (= 3 4))",
            "#record(.. base (= a 1))",
            "#record((= a 1) .. b (= c 2) .. d)",
        ] {
            let a = read(form).unwrap();
            assert_eq!(print(&a), form, "sexpr round-trip of {form}");
            // encode -> decode -> print stays the same flat spread form.
            let bytes = cadenza_ast::codec::encode(&a);
            let back = cadenza_ast::codec::decode(&bytes).expect("spread construction decodes");
            assert_eq!(print(&back), form, "binary round-trip of {form}");
        }
    }

    #[test]
    fn nativize_compound_source_exempts_effect_op_handler_arm_heads() {
        // An effect op-handler arm names its operation BARE at the arm head. When the op is named after a
        // compound ctor (`set` is the real case — a State effect's setter; also list/map/tuple/record), that
        // head is NOT a compound literal and must NOT be nativized into `#set(…)` (which would break the
        // handler / decline). The arm's BODY is still walked, so a genuine literal there nativizes.
        let n = |s: &str| super::nativize_compound_source(s).unwrap();
        // A `set` op arm head is left BARE; a `(tuple …)` literal in the arm body IS nativized.
        assert_eq!(
            n("(handle St 0 ((set (v) s (resume unit (tuple v s)))) body)"),
            "(handle St 0 ((set (v) s (resume unit #tuple(v s)))) body)"
        );
        // Multiple arms + a `get` (non-ctor, unaffected) alongside a `set` (ctor-named, exempted).
        assert_eq!(
            n("(handle St 0 ((get (u) s (resume s s)) (set (w) s (resume unit w))) body)"),
            "(handle St 0 ((get (u) s (resume s s)) (set (w) s (resume unit w))) body)"
        );
        // The five-part `ctl`-style arm `(op (params) state k body)` — the head is still the arm's ch[0].
        assert_eq!(
            n("(handle St 0 ((set (v) s k (resume unit v))) body)"),
            "(handle St 0 ((set (v) s k (resume unit v))) body)"
        );
        // A `set` LITERAL outside any handler arm still nativizes (the exemption is arm-head-scoped).
        assert_eq!(n("(do (set 1 2))"), "(do #set(1 2))");
    }

    #[test]
    fn nativize_compound_source_preserves_construction_spread_in_record_and_map() {
        // A construction SPREAD `(.. v)` (the wrapped rest/spread node, #5838/#5826) inside a record/map is
        // a 2-element list, but it is NOT a `(k v)` entry — the field-pairify step must NOT rewrite it to
        // `(= .. v)` (which would invent a field named `..` and corrupt the spread). A real `(k v)` entry
        // sitting alongside it still field-pairifies.
        let n = |s: &str| super::nativize_compound_source(s).unwrap();
        // record: an explicit `(= a 1)` field + a spread — spread stays `(.. r)`.
        assert_eq!(
            n("(do (def (f r) (record (= a 1) (.. r))) (export f))"),
            "(do (def (f r) #record((= a 1) (.. r))) (export f))"
        );
        // record: a POSITIONAL `(a 1)` entry field-pairifies to `(= a 1)`; the spread is left alone.
        assert_eq!(
            n("(do (def (f r) (record (a 1) (.. r))) (export f))"),
            "(do (def (f r) #record((= a 1) (.. r))) (export f))"
        );
        // map: same — positional `(k v)` → `(= k v)`, spread preserved.
        assert_eq!(
            n("(do (def (f m) (map (k v) (.. m))) (export f))"),
            "(do (def (f m) #map((= k v) (.. m))) (export f))"
        );
        // list/tuple/set carry ELEMENTS (no field-pairify), so a spread was already safe there — pin it.
        assert_eq!(
            n("(do (def (f xs) (list 1 (.. xs) 2)) (export f))"),
            "(do (def (f xs) #list(1 (.. xs) 2)) (export f))"
        );
        assert_eq!(
            n("(do (def (f t) (tuple (.. t) 9)) (export f))"),
            "(do (def (f t) #tuple((.. t) 9)) (export f))"
        );
    }

    #[test]
    fn nativize_compound_source_nativizes_compounds_inside_quote_and_leaves_strings() {
        // A compound literal QUOTED as AST data still nativizes: the native ctor-leaf is `structurally_eq`
        // to its name alias, so the quoted VALUE is preserved (this is the established corpus-12 norm — 185
        // native `#ctor`, incl. `(quote #list …)`, all gated green). `unquote`/`unquote-splicing` splices are
        // left alone (not compound heads); STRING-literal content (`(doc "…(list …)…")` prose) is never
        // touched — the codemod operates on the parsed AST, and a string is an opaque atom.
        let n = |s: &str| super::nativize_compound_source(s).unwrap();
        assert_eq!(n("(quote (list 1 2))"), "(quote #list(1 2))");
        assert_eq!(n("(quote (record (a 1)))"), "(quote #record((= a 1)))");
        assert_eq!(
            n("(quasiquote (list 1 (unquote x) 2))"),
            "(quasiquote #list(1 (unquote x) 2))"
        );
        // Nested quote inside quasiquote: both compounds nativize, the unquote is left as-is.
        assert_eq!(
            n("(quasiquote (tuple (unquote x) (quote (list 1))))"),
            "(quasiquote #tuple((unquote x) (quote #list(1))))"
        );
        // A `(list …)` INSIDE a string literal (doc prose) is opaque data — NOT nativized; the live sibling is.
        assert_eq!(
            n("(do (doc \"a (list 1 2) in prose\") (def (m) (list 3 4)) (export m))"),
            "(do (doc \"a (list 1 2) in prose\") (def (m) #list(3 4)) (export m))"
        );
    }

    #[test]
    fn nativize_compound_source_exempts_wit_world_type_descriptors() {
        // A `(wit-world …)` clause declares WIT interfaces/TYPES. Its lowercase `record`/`list`/… heads are
        // WIT TYPE descriptors, NOT compound value literals — out of M3's value-literal scope, and
        // nativizing them regresses (the imposed-WIT-world reducer path DECLINES a native-headed type
        // descriptor). So the whole `(wit-world …)` subtree is exempt from head-nativize; a genuine value
        // literal OUTSIDE the clause still nativizes.
        let n = |s: &str| super::nativize_compound_source(s).unwrap();
        // The wit-world `(record …)`/`(list …)` type descriptors stay classic; `(list 1 2)` value nativizes.
        assert_eq!(
            n(
                "(do (wit-world (world w (export i (member f (func (param m (record (= x (s64)))) (result (list (u8)))))))) (def (g) (list 1 2)) (export g))"
            ),
            "(do (wit-world (world w (export i (member f (func (param m (record (= x (s64)))) (result (list (u8)))))))) (def (g) #list(1 2)) (export g))"
        );
        // Nested wit type descriptors (record-of-list) also stay classic throughout the clause.
        assert_eq!(
            n(
                "(wit-world (world w (export i (member f (func (param m (record (= tok (list (u8))))) (result (s64)))))))"
            ),
            "(wit-world (world w (export i (member f (func (param m (record (= tok (list (u8))))) (result (s64)))))))"
        );
    }

    #[test]
    fn nativize_compound_source_respects_per_form_exempt_marker() {
        // A `; cdz-nativize-exempt: <reason>` line-comment IMMEDIATELY above a form marks it (+ its subtree)
        // as deliberately non-native — the codemod leaves it untouched; an unmarked sibling still nativizes.
        // Single source of truth honored by both the codemod and v-corpus-harness's nativize-check
        // (transitional NAME-HEAD parity cases, e.g. corpus-05 #6047 guarding the #6042 ML/paren-surface path).
        let n = |s: &str| super::nativize_compound_source(s).unwrap();
        // Marked def left classic; unmarked sibling nativizes.
        assert_eq!(
            n(
                "(do\n  ; cdz-nativize-exempt: guards the name-head path\n  (def (a) (tuple 1 2))\n  (def (b) (tuple 3 4))\n  (export a))"
            ),
            "(do\n  ; cdz-nativize-exempt: guards the name-head path\n  (def (a) (tuple 1 2))\n  (def (b) #tuple(3 4))\n  (export a))"
        );
        // Tolerant of a `;;` lead + extra whitespace.
        assert_eq!(
            n(";;   cdz-nativize-exempt: r\n(list 1 2)"),
            ";;   cdz-nativize-exempt: r\n(list 1 2)"
        );
        // The marker must be IMMEDIATELY above: a blank line between does NOT exempt.
        assert_eq!(
            n("; cdz-nativize-exempt: r\n\n(list 1 2)"),
            "; cdz-nativize-exempt: r\n\n#list(1 2)"
        );
        // A plain comment (not the tag) does NOT exempt.
        assert_eq!(n("; just a note\n(list 1 2)"), "; just a note\n#list(1 2)");
    }

    #[test]
    fn nativize_compound_source_skip_outputs_leaves_output_expected_values_untouched() {
        // CORPUS inputs-only mode (Phase-2 seq A): nativize `(input …)` programs + `(call …)` arg values,
        // but leave every `(output …)` expected value untouched — v-corpus-harness owns the render re-pin of
        // outputs (a text-nativize would not match the gate's normalizing render).
        let s = |x: &str| super::nativize_compound_source_skip_outputs(x).unwrap();
        // A whole corpus case: the (input …) list literal nativizes; the (output …) tuple is LEFT ALONE.
        assert_eq!(
            s(
                "(case \"c\" (input (do (def (main) (list 1 2)) (export main))) (output (: (tuple 1 2) T)))"
            ),
            "(case \"c\" (input (do (def (main) #list(1 2)) (export main))) (output (: (tuple 1 2) T)))"
        );
        // A `(call …)` ARGUMENT value is input-side → it nativizes; its sibling (output …) does not.
        assert_eq!(
            s("(case \"c\" (input p) (call main (: (list 3) L)) (output (: (record (x 1)) R)))"),
            "(case \"c\" (input p) (call main (: #list(3) L)) (output (: (record (x 1)) R)))"
        );
        // Nested compound INSIDE an (output …) stays legacy even when deep.
        assert_eq!(
            s("(output (: (list (tuple 1 2)) T))"),
            "(output (: (list (tuple 1 2)) T))"
        );
        // Whole-program mode (the default, guide) is unchanged — it nativizes everywhere, output or not.
        assert_eq!(
            super::nativize_compound_source("(output (: (list 1) T))").unwrap(),
            "(output (: #list(1) T))"
        );
    }

    #[test]
    fn nativize_compound_source_nativizes_string_primitive_heads() {
        // The STRING-primitive compound head `("word" …)` — the unshadowable "strings are the symbols"
        // escape form the corpus uses where a local binding shadows the name alias — must nativize to
        // `#word(…)` for M3, exactly like the name head. The native ctor-leaf is likewise unshadowable, so
        // the mapping preserves the ctor identity. A string head has no `head_name`, so the codemod fell
        // through and left `("tuple" …)` un-nativized → it would break at the Phase-2 reader flip.
        let n = |s: &str| super::nativize_compound_source(s).unwrap();
        assert_eq!(n("(\"tuple\" 7 8)"), "#tuple(7 8)");
        assert_eq!(n("(\"list\" 1 2)"), "#list(1 2)");
        assert_eq!(n("(\"set\" 1 2 3)"), "#set(1 2 3)");
        // A string-head map/record field-pairifies its 2-element positional entries, like the name head.
        assert_eq!(n("(\"map\" (1 2) (3 4))"), "#map((= 1 2) (= 3 4))");
        assert_eq!(n("(\"record\" (x 1))"), "#record((= x 1))");
        // Empty string-head form.
        assert_eq!(n("(\"tuple\")"), "#tuple()");
        // A string head is UNSHADOWABLE: even with a local `tuple` binding, `("tuple" …)` nativizes (it is
        // the ctor, not the shadowed value) — while the shadowed NAME head `(tuple …)` stays a value ref.
        assert_eq!(
            n("(let ((tuple (fn (a b) a))) (. (\"tuple\" 7 8) 0))"),
            "(let ((tuple (fn (a b) a))) (. #tuple(7 8) 0))"
        );
        // A string in a NON-head position (a value) is untouched — only a head-position string nativizes.
        assert_eq!(n("(f \"tuple\")"), "(f \"tuple\")");
        // An EMPTY list `()` has NO head — the string-head fallback must use `ch.first()`, not `ch[0]`
        // (which panicked: "index out of bounds: len is 0 but index is 0", crashing on any `()` in the
        // corpus). The `()` is left untouched; a sibling compound still nativizes.
        assert_eq!(n("()"), "()");
        assert_eq!(n("(do () (list 1 2))"), "(do () #list(1 2))");
        assert_eq!(n("(f () (\"tuple\" 3 4))"), "(f () #tuple(3 4))");
    }

    // `#word(…)` collection literals (DESIGN-native-ast-compound-data.md §D-SURFACE / M2). The `#word(`
    // head reads to a native ctor LEAF KIND (recognized by `compound_ctor_leaf`, NOT head text); a
    // `#record`/`#map` DIRECT `(= k v)` entry reads to a `FieldPair` head; member access reads to a
    // `Member` head. The s-expr printer resugars each back to its surface, so every form round-trips.
    #[test]
    fn hash_word_literals_read_to_native_ctor_heads() {
        for (src, ctor, children) in [
            ("#list(1 2 3)", CompoundCtor::List, 3usize),
            ("#tuple(a b)", CompoundCtor::Tuple, 2),
            ("#set(1 2 3)", CompoundCtor::Set, 3),
            ("#list()", CompoundCtor::List, 0),
            ("#record((= x 1) (= y 2))", CompoundCtor::Record, 2),
            ("#map((= 1 2) (= 3 4))", CompoundCtor::Map, 2),
            ("#record()", CompoundCtor::Record, 0),
        ] {
            let a = read(src).unwrap();
            assert_eq!(
                a.compound_ctor_leaf(a.root),
                Some(ctor),
                "{src} reads to a native ctor-leaf head (not a Str/Name head)"
            );
            match a.get(a.root) {
                Struct::List(items) => {
                    assert_eq!(
                        items.len(),
                        children + 1,
                        "{src}: head + {children} children"
                    )
                }
                Struct::Atom(_) => panic!("{src} must be a list"),
            }
            // The s-expr printer resugars the native head back to the `#word(…)` surface byte-exact, and
            // re-reading reproduces the same arena.
            assert_eq!(print(&a), src, "{src} prints back to its #word surface");
            assert!(
                read(&print(&a)).unwrap().structurally_eq(&a),
                "{src} round-trips through the s-expr printer"
            );
        }
    }

    #[test]
    fn hash_word_literals_nest_and_round_trip() {
        let src = "#list(#map((= 1 2)) #record((= a 3)))";
        let a = read(src).unwrap();
        assert_eq!(a.compound_ctor_leaf(a.root), Some(CompoundCtor::List));
        assert_eq!(
            print(&a),
            src,
            "nested native literals print back byte-exact"
        );
        assert!(read(&print(&a)).unwrap().structurally_eq(&a));
    }

    #[test]
    fn native_compound_reader_printer_round_trips_extreme_edge_cases() {
        // Hardening: native #-form reader/printer round-trip on EXTREME edge cases beyond the corpus —
        // deep nesting, #-form as a MAP KEY, arity-1, #set-of-compounds, Member inside a #record value,
        // empty-nested. Each must `print(read(s)) == s` (byte-exact) + re-read structurally-equal. Guards the
        // reader/printer against a resugaring/nesting regression on the shapes the corpus doesn't exercise.
        for s in [
            "#tuple(x)",                                                   // arity-1 tuple
            "#list(x)",                                                    // arity-1 list
            "#set(x)",                                                     // arity-1 set
            "#map((= k v))", // arity-1 map (FieldPair)
            "#list(#tuple(#record((= x #map((= 1 #set(1 2)))))))", // deep nesting, all kinds
            "#map((= #tuple(1 2) 3))", // #-form as a MAP KEY
            "#map((= #list(1) #set(2)))", // #-form key AND value
            "#set(#tuple(1 2) #record((= x 1)) #list(3))", // set of mixed compounds
            "#record((= a #list(1 2)) (= b #set(3 4)) (= c #tuple(5 6)))", // record of compounds
            "#record((= x (. r y)))", // Member inside a #record value
            "#list(#list() #tuple() #record() #map() #set())", // empties nested in a list
            "#map((= 1 #map((= 2 #map((= 3 4))))))", // nested maps (FieldPair depth)
        ] {
            let a = read(s).unwrap_or_else(|e| panic!("{s}: read failed: {:?}", e.0));
            let printed = print(&a);
            assert_eq!(printed, s, "{s}: printer must resugar byte-exact");
            assert!(
                read(&printed).unwrap().structurally_eq(&a),
                "{s}: reader ∘ printer must round-trip structurally"
            );
        }
    }

    #[test]
    fn hash_record_and_map_entries_read_to_field_pairs() {
        // A DIRECT `(= k v)` entry of a `#record`/`#map` reads to a `FieldPair` head (ruling A); the key
        // and value are the written nodes.
        let a = read("#record((= x 1) (= y 2))").unwrap();
        let items = match a.get(a.root) {
            Struct::List(items) => items.clone(),
            Struct::Atom(_) => panic!("record is a list"),
        };
        assert!(
            a.field_pair_parts(items[1]).is_some(),
            "first entry is a FieldPair"
        );
        assert!(
            a.field_pair_parts(items[2]).is_some(),
            "second entry is a FieldPair"
        );
        let (k, v) = a.field_pair_parts(items[1]).unwrap();
        assert_eq!(a.as_name(k), Some("x"), "field key");
        assert_eq!(a.as_name(v), None, "value 1 is a bare int atom, not a name");
    }

    #[test]
    fn equality_outside_a_record_map_body_stays_a_name() {
        // `field_pairify` rewrites ONLY a DIRECT `(= k v)` entry of a #record/#map. Equality `=` anywhere
        // else stays `Name("=")`: a standalone `(= a b)`, AND the inner `(= a b)` of a field VALUE.
        let bare = read("(= a b)").unwrap();
        assert_eq!(
            bare.head_name(bare.root),
            Some("="),
            "a bare (= a b) is equality"
        );
        assert_eq!(bare.field_pair_parts(bare.root), None);
        let a = read("#record((= x (= a b)))").unwrap();
        let entry = match a.get(a.root) {
            Struct::List(items) => items[1],
            Struct::Atom(_) => panic!("record is a list"),
        };
        let (_k, v) = a
            .field_pair_parts(entry)
            .expect("the outer entry is a FieldPair");
        assert_eq!(
            a.head_name(v),
            Some("="),
            "the inner (= a b) in the value stays Name(=)"
        );
        assert_eq!(a.field_pair_parts(v), None);
    }

    #[test]
    fn hash_record_field_may_be_comment_wrapped() {
        // A `(comment …)` wrapper around a field is descended: the inner `(= x 1)` becomes a FieldPair,
        // the wrapper is preserved around it.
        let a = read("#record((comment \"doc\" (= x 1)))").unwrap();
        let entry = match a.get(a.root) {
            Struct::List(items) => items[1],
            Struct::Atom(_) => panic!("record is a list"),
        };
        assert_eq!(
            a.head_name(entry),
            Some("comment"),
            "the comment wrapper is preserved"
        );
        let wrapped = match a.get(entry) {
            Struct::List(items) => *items.last().unwrap(),
            Struct::Atom(_) => panic!("comment wrapper is a list"),
        };
        assert!(
            a.field_pair_parts(wrapped).is_some(),
            "the wrapped (= x 1) is a FieldPair"
        );
    }

    #[test]
    fn member_access_reads_to_a_member_head() {
        // All three member surfaces — explicit `(. obj key)`, postfix `obj.key`, dotted `a.b` — read to a
        // native `Member` head, and print back to the explicit `(. obj key)` form.
        for src in ["(. p x)", "p.x", "(. (. p x) y)"] {
            let a = read(src).unwrap();
            assert!(
                a.member_parts(a.root).is_some(),
                "{src} reads to a Member head"
            );
        }
        let a = read("p.x").unwrap();
        assert_eq!(
            print(&a),
            "(. p x)",
            "the dotted sugar canonicalizes to the explicit form"
        );
        assert!(read(&print(&a)).unwrap().structurally_eq(&a));
    }

    #[test]
    fn only_the_exact_hash_word_paren_prefix_opens_a_literal() {
        // Only the exact `#word(` prefix opens a literal. `#list(1)` is one (a native List head); a longer
        // identifier `#listx(…)` is NOT stolen as a `#list` literal — the `x` breaks the ctor word so it
        // reads the ordinary way (a bare `#listx` token then a trailing `(1)`, a malformed single form),
        // proving the reader keys on the whole word immediately before `(`.
        let a = read("#list(1)").unwrap();
        assert_eq!(a.compound_ctor_leaf(a.root), Some(CompoundCtor::List));
        assert!(read("#listx(1)").is_err());
    }

    #[test]
    fn deeply_nested_input_is_diagnosed_not_crashed() {
        // `read` recurses one native frame per nesting level, so DESCENDING to the depth guard
        // (`MAX_NESTING_DEPTH` = 1024) needs more stack than a default `cargo test` worker on a small-
        // stack platform (macOS ~512 KB–1 MB). Run on a large-stacked thread so the test exercises the
        // depth guard, not the worker's stack limit (a spurious SIGABRT unrelated to this assertion).
        let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                // A pathologically deep but syntactically valid nest overflowed the native stack
                // (SIGABRT) in the unguarded recursive descent; the depth guard makes it a clean
                // `ReadError` instead. The depth here (limit + a margin) exceeds `MAX_NESTING_DEPTH`.
                let n = (MAX_NESTING_DEPTH as usize) + 50;
                let src = format!("{}1{}", "(+ ".repeat(n), " 1)".repeat(n));
                let err = read(&src).expect_err("deep nesting must be a clean error, not a crash");
                assert!(
                    err.0.contains("nests too deeply"),
                    "expected a depth-limit diagnostic, got: {}",
                    err.0
                );
                // Just UNDER the limit still parses (the guard does not reject a valid moderate nest).
                let ok = (MAX_NESTING_DEPTH as usize) - 1;
                let shallow = format!("{}1{}", "(+ ".repeat(ok), " 1)".repeat(ok));
                assert!(
                    read(&shallow).is_ok(),
                    "a nest just under the limit must still parse"
                );
            })
            .expect("spawn deep-read worker");
        if let Err(payload) = h.join() {
            std::panic::resume_unwind(payload);
        }
    }

    #[test]
    fn print_is_iterative_not_recursive_on_a_deep_arena() {
        // `print`/`print_node` runs on arenas from ANY source — a decoded binary AST in particular, which
        // `codec::decode` accepts at ARBITRARY nesting depth (no cap, unlike the reader's
        // MAX_NESTING_DEPTH). A native-recursive walk overflowed the stack (SIGABRT) on such a deep tree,
        // crashing the process on `cdz convert binary → sexpr`. Build a deep single-child chain DIRECTLY
        // (bypassing the reader cap) and assert `print` completes and is correct. 12k levels is well past
        // the native recursion limit; the output here is O(depth) (one `(`/`)` per level, no cumulative
        // indent — unlike the debug tree view), so a deep chain is cheap.
        let depth = 12_000usize;
        let mut b = Builder::new();
        let mut cur = b.name("x");
        for _ in 0..depth {
            cur = b.list(vec![cur]);
        }
        let a = b.finish(cur);
        let out = print(&a); // must NOT overflow (a recursive walk did)
        // The rendering is `(((…(x)…)))`: `depth` opens, then `x`, then `depth` closes — correct shape,
        // proving the iterative walk emits the same nesting a recursive one would. (Re-reading it is not
        // asserted here: `read` is intentionally depth-capped at MAX_NESTING_DEPTH=1024, so a 12k-deep
        // form is not round-trippable through the READER — that cap is the reader's own guarantee, tested
        // in `deeply_nested_input_is_diagnosed_not_crashed`. THIS test pins the PRINTER's totality.)
        assert_eq!(out, format!("{}x{}", "(".repeat(depth), ")".repeat(depth)));
    }

    #[test]
    fn print_pretty_is_iterative_not_recursive_on_a_deep_arena() {
        // Companion to `print_is_iterative_…`: the PRETTY s-expr printer (`print_pretty`, the default
        // multi-line rendering) walks the arena via `pretty_node` to BUILD the Oppen `Doc` token stream —
        // that build is itself a tree walk, one frame per level, so a deep arena overflowed it (SIGABRT)
        // just like the single-line printer, before this fix. Same reachability: `cdz convert binary →
        // sexpr` pretty-prints a decoded (uncapped-depth) AST. Assert a 12k-deep chain pretty-prints
        // without overflow. A single-child chain always fits any width, so it renders flat — identical to
        // the single-line form (`(((…x…)))`), which lets us pin the exact output too.
        let depth = 12_000usize;
        let mut b = Builder::new();
        let mut cur = b.name("x");
        for _ in 0..depth {
            cur = b.list(vec![cur]);
        }
        let a = b.finish(cur);
        let out = print_pretty(&a); // must NOT overflow
        assert_eq!(out, format!("{}x{}", "(".repeat(depth), ")".repeat(depth)));
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
        assert_eq!(slice(src, spans.get(a.root).unwrap()), "(Int 8).max");
        // Native Member head (M2); operand/key read via `member_parts`, their spans unchanged.
        let (obj, key) = a.member_parts(a.root).expect("member projection");
        assert_eq!(slice(src, spans.get(obj).unwrap()), "(Int 8)");
        assert_eq!(slice(src, spans.get(key).unwrap()), "max");
    }

    /// print∘read is stable: reading printed text yields a structurally-equal arena, and printing
    /// it again is byte-identical (the s-expr surface is its own canonical form).
    #[test]
    fn every_leaf_variant_round_trips_through_the_sexpr_printer() {
        // A systematic per-`Leaf`-variant pin of the s-expr PRINTER round-trip: for each leaf kind, build
        // an arena atom of it, `print` (flat s-expr), `read` it back, and assert structural equality — so
        // `print_leaf`'s rendering of EVERY variant re-reads to the same leaf. The existing pins are
        // scattered (char/symbol/bad-escape/bytes each in their own test, several via the codec not the
        // s-expr printer) and `print_reads_back` covers only Int/Str/Bool/Name/Bytes; no single sweep
        // exercised `print_leaf` over the whole variant space. A future escape-set change on any one arm
        // (e.g. a Sym `#"…"` quote-escape drift, a Bytes `\xNN` regression, a Char control-name change)
        // would then silently break that leaf's round-trip with nothing to catch it. This is the
        // s-expr-printer analogue of `cadenza-ast`'s codec `gen_leaf` sweep.
        //
        // NOTE the ONE variant that is NOT round-trippable BARE: `Suffixed` (`100N`). The s-expr reader
        // DELIBERATELY desugars a suffixed token to the annotation `(: <Suffixed> BigInt)` (a suffix IS a
        // terse annotation — see `classify_word_nonname`), so a bare `Suffixed` atom is a shape the reader
        // NEVER produces; printing one re-reads to the `(:  …)` wrapper, not a bare atom. So `Suffixed` is
        // pinned in its REAL desugared context below, not bare.
        let bare: Vec<(&str, Leaf)> = vec![
            (
                "Int-dec-neg",
                Leaf::Int {
                    value: cadenza_syntax_core::ast::IntValue::from_i64(-42),
                    radix: Radix::Dec,
                },
            ),
            (
                "Int-hex",
                Leaf::Int {
                    value: cadenza_syntax_core::ast::IntValue::from_i64(255),
                    radix: Radix::Hex,
                },
            ),
            (
                "Int-bin",
                Leaf::Int {
                    value: cadenza_syntax_core::ast::IntValue::from_i64(10),
                    radix: Radix::Bin,
                },
            ),
            (
                "Int-zero",
                Leaf::Int {
                    value: cadenza_syntax_core::ast::IntValue::from_i64(0),
                    radix: Radix::Dec,
                },
            ),
            (
                "Float-neg",
                Leaf::Float(Decimal {
                    negative: true,
                    significand: cadenza_syntax_core::ast::IntValue::from_i64(125).magnitude,
                    exponent: -2,
                }),
            ),
            ("Str-empty", Leaf::Str(String::new().into())),
            ("Str-escapes", Leaf::Str("a\nb\t\"c\\d".into())),
            ("Str-unicode", Leaf::Str("λ中🎉".into())),
            (
                "Bytes-high",
                Leaf::Bytes(vec![0x89, b'P', b'N', b'G', 0x00, 0xff].into()),
            ),
            ("Bytes-empty", Leaf::Bytes(vec![].into())),
            ("Bool-true", Leaf::Bool(true)),
            ("Bool-false", Leaf::Bool(false)),
            ("Name", Leaf::Name("foo-bar".into())),
            ("Name-op", Leaf::Name("+".into())),
            ("Sym", Leaf::Sym("meter".into())),
            ("Sym-quote", Leaf::Sym("has\"quote".into())),
            ("Char", Leaf::Char('é')),
            ("Char-ctrl", Leaf::Char('\n')),
            ("Char-emoji", Leaf::Char('🎉')),
            ("BadEscape", Leaf::BadEscape('q')),
            ("BadChar", Leaf::BadChar("u+D800".into())),
        ];
        for (label, leaf) in bare {
            let mut b = Builder::new();
            let id = b.leaf(leaf.clone());
            let root = b.atom(id);
            let a = b.finish(root);
            let printed = print(&a);
            let back = read(&printed).unwrap_or_else(|e| {
                panic!("[{label}] printed {printed:?} did not re-read: {}", e.0)
            });
            assert!(
                a.structurally_eq(&back),
                "[{label}] {leaf:?} printed {printed:?} did not round-trip through the s-expr printer"
            );
        }

        // `Suffixed` in its REAL context: a suffixed source token reads to `(: <Suffixed> Type)`. Print
        // THAT arena and assert it round-trips (the `Suffixed` atom rides inside, printed back as `100N`).
        for src in ["100N", "0.5R", "0xFFN", "12e2R"] {
            let a = read(src).unwrap_or_else(|e| panic!("suffixed {src:?} reads: {}", e.0));
            let printed = print(&a);
            let back = read(&printed).unwrap_or_else(|e| {
                panic!("suffixed {src:?} reprint {printed:?} re-reads: {}", e.0)
            });
            assert!(
                a.structurally_eq(&back),
                "suffixed {src:?} did not round-trip (printed {printed:?})"
            );
        }
    }

    #[test]
    fn with_correlation_perform_round_trips_through_the_sexpr_surface() {
        // Gate-protect the WITH-CORRELATION perform surface (coordinated w/ v-effects 2026-08-13, phase-2
        // schema-hash effect-identity; v-platform-conformance case-15 correlation-echo makes it
        // load-bearing). A correlation-carrying perform is spelled `(with-correlation <tok> (. E op
        // <args>))` — a CONTEXTUAL head-form (NOT a reserved keyword, NO parser change): v-effects'
        // reify-to-output lowering pattern-matches the `with-correlation` head at the reducer-output
        // boundary and sets EffectReify.correlation = Some(tok); a bare `(. E op <args>)` stays None. Since
        // it's a plain `(head child…)` node, the generic s-expr reader/printer handles it; this pins the
        // round-trip so a future surface change can't silently break the correlation channel. (This is the
        // SURFACE-fidelity pin; v-pc's suite pins the runtime echo behavior — complementary.)
        let cases = [
            "(with-correlation c1 (. E op a b))", // correlation-carrying perform
            "(. E op a b)",                       // the bare sibling (correlation=None)
            "(with-correlation (. Tok mk 7) (. Kv get k))", // a computed correlation token + real perform
            "(with-correlation c (. E op))", // nullary-arg perform under a correlation
        ];
        for src in cases {
            let a = read(src).unwrap_or_else(|e| panic!("read {src:?}: {e:?}"));
            let printed = print(&a);
            assert_eq!(
                printed.trim(),
                src,
                "with-correlation form must round-trip: {src:?}"
            );
            // Idempotence: re-reading the printed form yields the same bytes again.
            let b = read(&printed).unwrap_or_else(|e| panic!("reread {printed:?}: {e:?}"));
            assert_eq!(
                print(&b).trim(),
                src,
                "with-correlation round-trip not idempotent: {src:?}"
            );
        }
    }

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
                    Leaf::Bytes(b) => assert_eq!(&b[..], &want[..], "bytes for {src:?}"),
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
        // A module blank-separates members, keeping `module` and the name attached (`(module m`).
        let a = read("(module m (type T A B) (def (a x) x) (def (b y) y))").unwrap();
        assert_eq!(
            print_pretty_width(&a, 25),
            "(module m\n  (type T A B)\n\n  (def (a x) x)\n\n  (def (b y) y))"
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

    /// Generate a random VALID s-expr program (bounded by `depth`) — atoms, infix, calls, `let`, `if`,
    /// nested lists, and `record`/`match` shapes that stress the pretty-printer's layout branches.
    fn gen_pretty_prog(rng: &mut SplitMix64, depth: usize) -> String {
        let names = ["a", "b", "x", "y", "f", "+", "g", "\"s\"", "42", "true"];
        if depth == 0 || rng.next().is_multiple_of(3) {
            return names[(rng.next() as usize) % names.len()].to_string();
        }
        let sub = |rng: &mut SplitMix64| gen_pretty_prog(rng, depth - 1);
        match rng.next() % 6 {
            0 => format!("(+ {} {})", sub(rng), sub(rng)),
            1 => format!("(f {} {} {})", sub(rng), sub(rng), sub(rng)),
            2 => format!("(if {} {} {})", sub(rng), sub(rng), sub(rng)),
            3 => format!("(let ((x {}) (y {})) {})", sub(rng), sub(rng), sub(rng)),
            4 => format!("(record (x {}) (y {}))", sub(rng), sub(rng)),
            _ => format!(
                "(match {} ((Some n) {}) ((None _) {}))",
                sub(rng),
                sub(rng),
                sub(rng)
            ),
        }
    }

    #[test]
    fn pretty_round_trip_is_faithful_over_generated_programs_and_widths() {
        // `pretty_reads_back_to_the_same_arena` pins ~6 hand-picked programs at one width; this sweeps the
        // FIDELITY property over random VALID programs at a RANGE of widths: `read(print_pretty_width(a,
        // w))` is structurally equal to `a`, for every width — a broken layout only shows at a width that
        // forces the offending break, so varying the width is essential. Complements
        // `sexpr_printer_is_total_over_arbitrary_arenas` (which sources RECOVERED arenas from byte-soup and
        // pins TOTALITY + flat/pretty agreement, not round-trip fidelity of valid programs). Also asserts
        // pretty is idempotent at each width (the laid-out form re-reads + re-prints to itself).
        let mut rng = SplitMix64(0xbead_c0de_face_1234);
        for _ in 0..3000 {
            let depth = 1 + (rng.next() % 4) as usize;
            let src = gen_pretty_prog(&mut rng, depth);
            let a =
                read(&src).unwrap_or_else(|e| panic!("generated s-expr {src:?} reads: {}", e.0));
            for &width in &[0usize, 1, 8, 30, 100] {
                let pretty = print_pretty_width(&a, width);
                let b = read(&pretty).unwrap_or_else(|e| {
                    panic!("pretty(w={width}) of {src:?} re-reads: {}\n{pretty}", e.0)
                });
                assert!(
                    a.structurally_eq(&b),
                    "pretty(w={width}) not faithful for {src:?}\n--- pretty ---\n{pretty}"
                );
                // Idempotent at this width: the laid-out tree re-prints identically.
                assert_eq!(
                    print_pretty_width(&b, width),
                    pretty,
                    "pretty(w={width}) not idempotent for {src:?}"
                );
            }
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
                    value.to_bigint(),
                    BigInt::from_str("123456789012345678901234567890").unwrap()
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
                    value: cadenza_syntax_core::ast::IntValue::from_i64(val),
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
                significand: cadenza_syntax_core::ast::IntValue::from_i64(15).magnitude,
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
                significand: cadenza_syntax_core::ast::IntValue::from_i64(15).magnitude,
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
        // (. Sign Neg) — a native Member head (M2), recognized by kind via `member_parts`.
        let (obj, key) = a
            .member_parts(a.root)
            .expect("Sign.Neg is a Member projection");
        assert_eq!(a.as_name(obj), Some("Sign"));
        assert_eq!(a.as_name(key), Some("Neg"));
    }

    #[test]
    fn postfix_member_after_a_paren_desugars() {
        // `(Int 8).max` reads to `(. (Int 8) max)` — the paren-postfix sibling of `Int8.max`. This is
        // what lets a type-constructor application be projected directly (the modules `(Int N)` builds
        // carry `max`/`min`/`wrap`), reading identically to the aliased-name form. Native Member head.
        let a = read("(Int 8).max").unwrap();
        let (obj, key) = a.member_parts(a.root).expect("member projection");
        // operand is the `(Int 8)` application; key is `max`.
        assert_eq!(a.head_name(obj), Some("Int"));
        assert_eq!(a.as_name(key), Some("max"));
    }

    #[test]
    fn postfix_member_chains_and_composes_with_application() {
        // `((. (UInt 48) wrap) -1)` is unaffected (explicit form), and a chained postfix `(Int 8).x.y`
        // nests left-to-right: `(. (. (Int 8) x) y)` — nested native Member heads.
        let a = read("(Int 8).x.y").unwrap();
        let (outer_obj, outer_key) = a.member_parts(a.root).expect("outer member");
        assert_eq!(a.as_name(outer_key), Some("y"));
        let (inner_obj, inner_key) = a.member_parts(outer_obj).expect("inner (. (Int 8) x)");
        assert_eq!(a.head_name(inner_obj), Some("Int"));
        assert_eq!(a.as_name(inner_key), Some("x"));
    }

    #[test]
    fn dot_head_form_is_not_a_postfix() {
        // `(. p x)` — a `.` that heads a list (with a following space) is ordinary member-access
        // structure, NOT a postfix on the preceding token. Pins that the postfix only fires on a `.`
        // glued to an identifier segment. Reads to a native Member head.
        let a = read("(. p x)").unwrap();
        let (obj, key) = a
            .member_parts(a.root)
            .expect("explicit (. p x) is a Member projection");
        assert_eq!(a.as_name(obj), Some("p"));
        assert_eq!(a.as_name(key), Some("x"));
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
                value: cadenza_syntax_core::ast::IntValue::from_i64(1_000_000),
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
        assert_eq!(b.leaf(*l), &Leaf::Str("\n".into()));
    }

    #[test]
    fn a_bad_escape_marker_round_trips_through_the_codec() {
        // The marker must survive the binary AST codec (encode→decode) unchanged, so the compiler that
        // reads the binary AST sees the same `BadEscape` the reader produced.
        let a = read("\"\\q\"").unwrap();
        let bytes = cadenza_ast::codec::encode(&a);
        let b = cadenza_ast::codec::decode(&bytes).expect("decode");
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
        assert_eq!(leaf_of("#\\u+D800"), Leaf::BadChar("u+D800".into()));
        // A code point past U+10FFFF is likewise a BadChar.
        assert_eq!(leaf_of("#\\u+110000"), Leaf::BadChar("u+110000".into()));
    }

    #[test]
    fn char_leaves_round_trip_through_the_codec() {
        // A `Char` and a `BadChar` must survive the binary AST codec unchanged (the compiler reads the
        // binary AST, so it must see the same leaf the reader produced).
        for src in ["#\\a", "#\\newline", "#\\u+D800"] {
            let a = read(src).unwrap();
            let bytes = cadenza_ast::codec::encode(&a);
            let b = cadenza_ast::codec::decode(&bytes).expect("decode");
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
        assert_eq!(leaf_of("#\"meter\""), Leaf::Sym("meter".into()));
        assert_eq!(leaf_of("#\"\""), Leaf::Sym(String::new().into())); // the empty symbol
        assert_eq!(
            leaf_of("#\"a b\""),
            Leaf::Sym("a b".into()) // a symbol may carry spaces (it is not an identifier)
        );
        // A symbol is NOT a string and NOT a name.
        assert_ne!(leaf_of("#\"meter\""), Leaf::Str("meter".into()));
        assert_ne!(leaf_of("#\"meter\""), Leaf::Name("meter".into()));
        // The `#`-SIGIL BOUNDARY: `#` opens a symbol/char ONLY before `"`/`\`; before anything else
        // (an identifier char), a bare `#foo` is an ORDINARY token — a `Leaf::Name` whose text INCLUDES
        // the `#` (NOT a `Leaf::Sym("foo")`). So `#foo` and `#"foo"` are DISTINCT nodes. This is the
        // reader-lexis rule the ML printer's `#`-name backtick escape depends on (a `#`-headed name must
        // re-emit `` `#foo` `` so it doesn't re-lex as the symbol), and the answer to whether
        // `(quote #foo)` reifies an `Ast.Name "#foo"` (it does) — pinned here so neither can silently drift.
        assert_eq!(leaf_of("#foo"), Leaf::Name("#foo".into()));
        assert_eq!(leaf_of("#meter"), Leaf::Name("#meter".into()));
        assert_ne!(leaf_of("#foo"), Leaf::Sym("foo".into()));
        // And the two `#foo` spellings read to genuinely different leaves.
        assert_ne!(leaf_of("#foo"), leaf_of("#\"foo\""));
    }

    #[test]
    fn symbol_leaves_round_trip_through_codec_and_printer() {
        // A `Sym` must survive BOTH the binary AST codec (the compiler reads the binary AST) and the
        // printer (`read(print(x)) == x`) — the two gates the `#"…"` literal must hold for the units
        // corpus surface (`(Unit.base #"meter")`).
        for src in ["#\"meter\"", "#\"second\"", "#\"\""] {
            let a = read(src).unwrap();
            // Codec round-trip.
            let bytes = cadenza_ast::codec::encode(&a);
            let b = cadenza_ast::codec::decode(&bytes).expect("decode");
            assert!(a.structurally_eq(&b), "codec round-trip changed {src}");
            // Printer round-trip.
            let printed = print(&a);
            assert_eq!(printed, src, "{src} did not print back verbatim");
            let c = read(&printed).unwrap();
            assert!(a.structurally_eq(&c), "printer round-trip changed {src}");
        }
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible fuzz without a dependency, matching the
    /// lexer/codec/parser house style (the crate stays "plain").
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

    /// The s-expr reader's invariant on ARBITRARY input: it MUST NOT PANIC (it returns a `ReadError`
    /// diagnostic on malformed text, never crashes), and on a SUCCESSFUL read the arena must be
    /// well-formed — root id in range, span table TOTAL (1:1 with the structure vector), and every
    /// reachable child id in range (fully traversable). Runs both `read_spanned` (checks the span-table
    /// totality the whole `SpanTable` design rests on) and `read_all_spanned` (the multi-form wrap).
    fn assert_sexpr_read_invariants(src: &str) {
        for spanned in [read_spanned(src), read_all_spanned(src)] {
            let Ok((a, spans)) = spanned else {
                continue; // a clean `ReadError` on malformed input is fine — the point is no panic.
            };
            let n = a.structure.len();
            assert!(n > 0, "a successful read has a non-empty arena for {src:?}");
            assert!((a.root.0 as usize) < n, "root id in range for {src:?}");
            assert_eq!(
                spans.len(),
                n,
                "span table is total (1:1 with structure) for {src:?}"
            );
            fn walk(a: &Arenas, id: StructId) {
                if let Struct::List(kids) = a.get(id) {
                    for &c in kids {
                        assert!(
                            (c.0 as usize) < a.structure.len(),
                            "child id {} in range",
                            c.0
                        );
                        walk(a, c);
                    }
                }
            }
            walk(&a, a.root);
        }
    }

    #[test]
    fn sexpr_reader_invariants_hold_on_arbitrary_input() {
        // Mirror of the ML parser's `recovered_arena_invariants_hold_on_arbitrary_input`, for the s-expr
        // surface. The alphabet stresses the reader's branches: list delimiters, the atom/literal
        // openers (`"` string, `#"` symbol / `#\` char, `b"` byte-string), the `.`-member postfix, digit
        // + numeric affixes, escape/comment chars, and unicode. Lengths stay ≤32 so the per-nesting-level
        // recursion cannot overflow a default test stack (the deep-nest DEPTH GUARD is tested separately
        // in `deeply_nested_input_is_diagnosed_not_crashed`).
        let alphabet: Vec<char> = "()\"#\\b.,;|=>-+*/<:@`0123456789abcxeNR_ \tλ中\n"
            .chars()
            .collect();
        let mut rng = SplitMix64(0x5e37_c0de_faca_de01);
        for len in 0..=32usize {
            for _ in 0..120 {
                let s: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                assert_sexpr_read_invariants(&s);
            }
        }
        // Truncated/odd literal openers — the classic panic bait — and unbalanced-delimiter soup.
        for s in [
            "\"",
            "\"\\",
            "#\"",
            "#\"\\",
            "#\\",
            "#\\u+",
            "b\"",
            "b\"\\",
            "`",
            "(",
            ")",
            "((((((((",
            "))))))))",
            "( . )",
            "(a .",
            "#",
            "#\\u+D800",
            "0x",
            "1e",
            "1_",
            ".",
            "(.)",
        ] {
            assert_sexpr_read_invariants(s);
            assert_sexpr_read_invariants(&s.repeat(4));
        }
    }
}
