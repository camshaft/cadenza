//! The **JSON surface** — a faithful data document as a projection of the one canonical arena.
//!
//! JSON is a first-class front-end syntax, exactly like the s-expression, ML, and markdown surfaces:
//! a parser (`read`) turns JSON text into the shared [`Arenas`], and a printer (`print`) turns a JSON
//! arena back into JSON text. It is not privileged (`spec/contracts/ast-encoding.md` §A Textual Syntax
//! Parses To And Prints From The Canonical Form) — a `.json` reads to the same binary AST any surface
//! does, so `cdz convert data.json --to binary` yields a canonical arena.
//!
//! Like a markdown document, a JSON value is *data*, not a program: its nodes are JSON structure
//! (`json-object`/`json-array`/`json-null` plus bare scalar leaves), not language constructs, so the
//! compiler never sees one. The surface is deliberately **faithful** rather than coerced into
//! Cadenza's typed value universe: it maps to a dedicated JSON vocabulary rather than onto native
//! `record`/`list`, so every property real JSON has that the typed value universe would reject or
//! normalize survives a round-trip — duplicate object keys, key order, arbitrary (non-identifier,
//! Unicode) keys, heterogeneous arrays, exact arbitrary-precision numbers, and `null`.
//!
//! ## Node vocabulary (all ordinary `Name`-headed lists + bare scalar leaves — no codec change)
//!
//! The ROOT is the top-level JSON value directly (RFC 8259 allows any value at top level — object,
//! array, or scalar); there is no wrapper node.
//! - Object → `(json-object (member <key:Str> <value>)…)` — the `key` is a `Str` leaf holding the
//!   JSON-DECODED string (any Unicode), the `value` is any value node. Order and duplicate keys are
//!   preserved (an ordered child list, never a set).
//! - Array → `(json-array <value>…)` — order preserved; children need not share a type.
//! - `null` → `(json-null)` — a nullary form, distinct from any scalar (there is no native null).
//! - Scalars → bare leaf atoms: `Str` (JSON-decoded), `Int { value: BigInt, radix: Dec }`,
//!   `Float(Decimal)`, `Bool`.
//!
//! ## Round-trip
//!
//! JSON is not injective (whitespace, `1.00` vs `1.0`, `A` vs `A`, `1E9` vs `1e9` all render one
//! tree), so the guarantee is **arena-idempotence** — `read(print(read(json)))` equals `read(json)` —
//! the same contract the ML and markdown surfaces hold, not byte identity of the source. Numbers are
//! exact at the leaf layer (`Int` is arbitrary-precision `BigInt`, `Float` is an exact width-free
//! `Decimal`), and a document is never type-checked, so `Int64`/`Float64` bounds never apply — a huge
//! integer or a `1e400` survives as a leaf where it would overflow/reject as a typed value.

use cadenza_syntax_core::arena_read::{child_tail, list_items};
use cadenza_syntax_core::ast::{Arenas, Builder, Leaf, Radix, StructId};
use cadenza_syntax_core::span::Span;
use cadenza_syntax_core::spans::{FileId, SpanTable};

/// A JSON parse failure, with a human-readable message (mirrors `sexpr::ReadError`). The message ends
/// in `at byte N` where a position is meaningful, so a caller holding the source can turn it into a
/// `line:col`.
#[derive(Debug)]
pub struct ReadError(pub String);

/// The maximum object/array nesting depth the recursive-descent reader accepts before returning a
/// [`ReadError`] rather than recursing further — the same guard, and the same value, the s-expr reader
/// uses (`sexpr::MAX_NESTING_DEPTH`): recursive descent uses one native stack frame per level, so
/// unbounded depth would overflow the stack on pathologically deep (but syntactically valid) input,
/// which matters most for `cdz-wasm`'s ~1 MB stack parsing UNTRUSTED browser input.
pub const MAX_NESTING_DEPTH: u32 = cadenza_syntax_core::MAX_NESTING_DEPTH;

/// Parse JSON `src` into a value arena (the root is the top-level value directly), or a [`ReadError`]
/// on malformed input. Total-with-refusal: unlike CommonMark, JSON can fail, so a bad document is a
/// clean error, never a patched-up tree.
pub fn read(src: &str) -> Result<Arenas, ReadError> {
    let mut b = Builder::new();
    let mut it = src.char_indices().peekable();
    let root = Json::new(&mut b, &mut it, src, None).parse_document()?;
    Ok(b.finish(root))
}

/// Parse JSON `src` into a value arena, ALSO producing a [`SpanTable`] mapping each structure
/// occurrence to its source byte range — the same source-tracking substrate the other surfaces
/// produce. The arena is byte-identical to [`read`]'s; only the table is extra.
pub fn read_spanned(src: &str) -> Result<(Arenas, SpanTable), ReadError> {
    let mut b = Builder::new();
    let mut it = src.char_indices().peekable();
    let mut p = Json::new(
        &mut b,
        &mut it,
        src,
        Some(SpanTable::new(FileId::default())),
    );
    let root = p.parse_document()?;
    let spans = p.spans.take().expect("span tracking on");
    Ok((b.finish(root), spans))
}

// ============================================================================
// Reader: JSON text -> value arena
// ============================================================================

type Cursor<'s> = std::iter::Peekable<std::str::CharIndices<'s>>;

/// The recursive-descent reader. Builds children before parents (pre-order recursive descent), so the
/// arena is already in canonical form — like the s-expr reader, and unlike the ML surface, no
/// `canonicalize_with_map` + `SpanTable::remap` is needed (and a JSON document is data, never handed to
/// the compiler, so its span ids only ever serve the query/rewrite path over this same arena).
struct Json<'b, 's, 'c> {
    b: &'b mut Builder,
    /// The `(byte-offset, char)` cursor over the source, peekable for one-token lookahead.
    it: &'c mut Cursor<'s>,
    src: &'s str,
    /// When `Some`, every created occurrence pushes its span here in id order (kept 1:1 with the arena).
    spans: Option<SpanTable>,
    depth: u32,
}

impl<'b, 's, 'c> Json<'b, 's, 'c> {
    fn new(
        b: &'b mut Builder,
        it: &'c mut Cursor<'s>,
        src: &'s str,
        spans: Option<SpanTable>,
    ) -> Json<'b, 's, 'c> {
        Json {
            b,
            it,
            src,
            spans,
            depth: 0,
        }
    }

    /// Parse a whole document: one value, then only trailing whitespace (no trailing garbage).
    fn parse_document(&mut self) -> Result<StructId, ReadError> {
        self.skip_ws();
        let root = self.parse_value()?;
        self.skip_ws();
        if let Some(&(i, c)) = self.it.peek() {
            return Err(ReadError(format!(
                "trailing input after the JSON value: {c:?} at byte {i}"
            )));
        }
        Ok(root)
    }

    /// Parse one JSON value at the cursor (whitespace already skipped by the caller when needed).
    fn parse_value(&mut self) -> Result<StructId, ReadError> {
        self.skip_ws();
        match self.it.peek().copied() {
            None => Err(ReadError("unexpected end of input".into())),
            Some((start, c)) => match c {
                '{' => self.parse_object(start),
                '[' => self.parse_array(start),
                '"' => {
                    let (s, span) = self.parse_string()?;
                    Ok(self.mk_atom_leaf(Leaf::Str(s.into()), span))
                }
                't' | 'f' => self.parse_bool(start),
                'n' => self.parse_null(start),
                '-' | '0'..='9' => self.parse_number(start),
                other => Err(ReadError(format!(
                    "unexpected character {other:?} at byte {start}"
                ))),
            },
        }
    }

    fn parse_object(&mut self, start: usize) -> Result<StructId, ReadError> {
        self.enter()?;
        self.bump(); // consume '{'
        let head = self.mk_name("json-object", Span::new(start, start + 1));
        let mut items = vec![head];
        self.skip_ws();
        // Empty object.
        if self.eat('}') {
            self.leave();
            let span = self.span_to(start);
            return Ok(self.mk_list(items, span));
        }
        loop {
            self.skip_ws();
            // A key MUST be a string.
            let key_start = match self.it.peek() {
                Some(&(i, '"')) => i,
                Some(&(i, c)) => {
                    return Err(ReadError(format!(
                        "object key must be a string, found {c:?} at byte {i}"
                    )));
                }
                None => return Err(ReadError("unterminated object (expected a key)".into())),
            };
            let (key, key_span) = self.parse_string()?;
            let key_leaf = self.mk_atom_leaf(Leaf::Str(key.into()), key_span);
            self.skip_ws();
            if !self.eat(':') {
                return Err(ReadError(format!(
                    "expected ':' after object key at byte {}",
                    self.pos_or_end(key_start)
                )));
            }
            let value = self.parse_value()?;
            // (member <key:Str> <value>)
            let member_head = self.mk_name("member", key_span);
            let member = self.mk_list(vec![member_head, key_leaf, value], key_span);
            items.push(member);
            self.skip_ws();
            match self.it.peek().copied() {
                Some((_, ',')) => {
                    self.bump();
                }
                Some((_, '}')) => {
                    self.bump();
                    break;
                }
                Some((i, c)) => {
                    return Err(ReadError(format!(
                        "expected ',' or '}}' in object, found {c:?} at byte {i}"
                    )));
                }
                None => return Err(ReadError("unterminated object".into())),
            }
        }
        self.leave();
        let span = self.span_to(start);
        Ok(self.mk_list(items, span))
    }

    fn parse_array(&mut self, start: usize) -> Result<StructId, ReadError> {
        self.enter()?;
        self.bump(); // consume '['
        let head = self.mk_name("json-array", Span::new(start, start + 1));
        let mut items = vec![head];
        self.skip_ws();
        if self.eat(']') {
            self.leave();
            let span = self.span_to(start);
            return Ok(self.mk_list(items, span));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.it.peek().copied() {
                Some((_, ',')) => {
                    self.bump();
                }
                Some((_, ']')) => {
                    self.bump();
                    break;
                }
                Some((i, c)) => {
                    return Err(ReadError(format!(
                        "expected ',' or ']' in array, found {c:?} at byte {i}"
                    )));
                }
                None => return Err(ReadError("unterminated array".into())),
            }
        }
        self.leave();
        let span = self.span_to(start);
        Ok(self.mk_list(items, span))
    }

    /// Parse a `true` or `false` literal into a `Bool` leaf.
    fn parse_bool(&mut self, start: usize) -> Result<StructId, ReadError> {
        if self.eat_keyword("true") {
            let span = self.span_to(start);
            Ok(self.mk_atom_leaf(Leaf::Bool(true), span))
        } else if self.eat_keyword("false") {
            let span = self.span_to(start);
            Ok(self.mk_atom_leaf(Leaf::Bool(false), span))
        } else {
            Err(ReadError(format!(
                "invalid literal at byte {start} (expected `true`/`false`)"
            )))
        }
    }

    /// Parse a `null` literal into a `(json-null)` node.
    fn parse_null(&mut self, start: usize) -> Result<StructId, ReadError> {
        if self.eat_keyword("null") {
            let span = self.span_to(start);
            let head = self.mk_name("json-null", span);
            Ok(self.mk_list(vec![head], span))
        } else {
            Err(ReadError(format!(
                "invalid literal at byte {start} (expected `null`)"
            )))
        }
    }

    /// Parse a JSON number into an `Int` or `Float` leaf. Validates the STRICT JSON number grammar
    /// (`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`) — no leading `+`, no leading zeros, a digit
    /// required after `.` and after the exponent marker — then reuses `literal::parse_int`/`parse_float`
    /// to build the exact `BigInt`/`Decimal` (the token is already known JSON-legal, so their `_`/`0x`
    /// leniency is never exercised). A token containing `.`/`e`/`E` is a `Float`, otherwise an `Int`.
    fn parse_number(&mut self, start: usize) -> Result<StructId, ReadError> {
        let mut is_float = false;
        // Leading minus (a leading '+' is not JSON).
        if self.peek_is('-') {
            self.bump();
        }
        // Integer part: a single `0`, or a nonzero digit followed by more digits (no leading zeros).
        match self.it.peek().copied() {
            Some((_, '0')) => {
                self.bump();
            }
            Some((_, '1'..='9')) => {
                self.bump();
                while self.peek_ascii_digit() {
                    self.bump();
                }
            }
            _ => {
                return Err(ReadError(format!(
                    "invalid number: expected a digit at byte {}",
                    self.pos_or_end(start)
                )));
            }
        }
        // Fraction.
        if self.peek_is('.') {
            is_float = true;
            self.bump();
            if !self.peek_ascii_digit() {
                return Err(ReadError(format!(
                    "invalid number: a digit must follow '.' at byte {}",
                    self.pos_or_end(start)
                )));
            }
            while self.peek_ascii_digit() {
                self.bump();
            }
        }
        // Exponent.
        if self.peek_is('e') || self.peek_is('E') {
            is_float = true;
            self.bump();
            if self.peek_is('+') || self.peek_is('-') {
                self.bump();
            }
            if !self.peek_ascii_digit() {
                return Err(ReadError(format!(
                    "invalid number: a digit must follow the exponent at byte {}",
                    self.pos_or_end(start)
                )));
            }
            while self.peek_ascii_digit() {
                self.bump();
            }
        }
        // The token runs from `start` to the current cursor (next char, or source end).
        let end = self.it.peek().map(|&(i, _)| i).unwrap_or(self.src.len());
        let tok = &self.src[start..end];
        let span = Span::new(start, end);
        if is_float {
            let d = cadenza_syntax_core::literal::parse_float(tok)
                .ok_or_else(|| ReadError(format!("invalid number {tok:?} at byte {start}")))?;
            Ok(self.mk_atom_leaf(Leaf::Float(d), span))
        } else {
            let (value, _radix) = cadenza_syntax_core::literal::parse_int(tok)
                .ok_or_else(|| ReadError(format!("invalid number {tok:?} at byte {start}")))?;
            // A JSON integer is always base-10.
            Ok(self.mk_atom_leaf(
                Leaf::Int {
                    value,
                    radix: Radix::Dec,
                },
                span,
            ))
        }
    }

    /// Parse a `"…"` string at the cursor, returning the DECODED content and its full span (quotes
    /// included). Handles the JSON escape set (`\" \\ \/ \b \f \n \r \t` and `\uXXXX`, combining a
    /// valid high+low surrogate PAIR into one astral scalar). A control char below `0x20` unescaped, a
    /// bad/short `\u`, a lone surrogate, or an unterminated string is a [`ReadError`] — the surface is
    /// faithful-or-refuse.
    fn parse_string(&mut self) -> Result<(String, Span), ReadError> {
        let start = match self.it.peek() {
            Some(&(i, '"')) => i,
            _ => return Err(ReadError("expected a string".into())),
        };
        self.bump(); // opening quote
        let mut out = String::new();
        loop {
            match self.it.next() {
                None => return Err(ReadError(format!("unterminated string at byte {start}"))),
                Some((i, '"')) => {
                    return Ok((out, Span::new(start, i + 1)));
                }
                Some((i, '\\')) => {
                    self.parse_escape(&mut out, i)?;
                }
                Some((i, c)) if (c as u32) < 0x20 => {
                    return Err(ReadError(format!(
                        "unescaped control character U+{:04X} in string at byte {i}",
                        c as u32
                    )));
                }
                Some((_, c)) => out.push(c),
            }
        }
    }

    /// Handle one escape sequence after a `\` (whose byte offset is `bs`), appending the decoded
    /// scalar(s) to `out`.
    fn parse_escape(&mut self, out: &mut String, bs: usize) -> Result<(), ReadError> {
        match self.it.next() {
            Some((_, '"')) => out.push('"'),
            Some((_, '\\')) => out.push('\\'),
            Some((_, '/')) => out.push('/'),
            Some((_, 'b')) => out.push('\u{0008}'),
            Some((_, 'f')) => out.push('\u{000C}'),
            Some((_, 'n')) => out.push('\n'),
            Some((_, 'r')) => out.push('\r'),
            Some((_, 't')) => out.push('\t'),
            Some((_, 'u')) => {
                let hi = self.parse_hex4(bs)?;
                if (0xD800..=0xDBFF).contains(&hi) {
                    // High surrogate — a low surrogate MUST follow as `\uXXXX`.
                    if !(self.eat('\\') && self.eat('u')) {
                        return Err(ReadError(format!(
                            "lone high surrogate U+{hi:04X} at byte {bs} (expected a following \\u low surrogate)"
                        )));
                    }
                    let lo = self.parse_hex4(bs)?;
                    if !(0xDC00..=0xDFFF).contains(&lo) {
                        return Err(ReadError(format!(
                            "invalid low surrogate U+{lo:04X} after high surrogate at byte {bs}"
                        )));
                    }
                    let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                    match char::from_u32(cp) {
                        Some(c) => out.push(c),
                        None => {
                            return Err(ReadError(format!("invalid surrogate pair at byte {bs}")));
                        }
                    }
                } else if (0xDC00..=0xDFFF).contains(&hi) {
                    return Err(ReadError(format!(
                        "lone low surrogate U+{hi:04X} at byte {bs}"
                    )));
                } else {
                    match char::from_u32(hi) {
                        Some(c) => out.push(c),
                        None => {
                            return Err(ReadError(format!(
                                "invalid code point U+{hi:04X} at byte {bs}"
                            )));
                        }
                    }
                }
            }
            Some((i, c)) => {
                return Err(ReadError(format!("invalid escape \\{c} at byte {i}")));
            }
            None => return Err(ReadError(format!("unterminated escape at byte {bs}"))),
        }
        Ok(())
    }

    /// Read exactly four hex digits (a `\uXXXX` payload) into their `u32` value.
    fn parse_hex4(&mut self, bs: usize) -> Result<u32, ReadError> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            match self.it.next() {
                Some((_, c)) if c.is_ascii_hexdigit() => {
                    v = (v << 4) | c.to_digit(16).unwrap();
                }
                Some((i, c)) => {
                    return Err(ReadError(format!(
                        "invalid \\u escape: {c:?} is not a hex digit at byte {i}"
                    )));
                }
                None => {
                    return Err(ReadError(format!("unterminated \\u escape at byte {bs}")));
                }
            }
        }
        Ok(v)
    }

    // ---- cursor primitives ----

    /// Skip JSON whitespace (space, tab, newline, carriage return).
    fn skip_ws(&mut self) {
        while let Some(&(_, c)) = self.it.peek() {
            if matches!(c, ' ' | '\t' | '\n' | '\r') {
                self.it.next();
            } else {
                break;
            }
        }
    }

    /// Advance past one char.
    fn bump(&mut self) {
        self.it.next();
    }

    /// Consume the next char iff it equals `c`; report whether it did.
    fn eat(&mut self, c: char) -> bool {
        if self.peek_is(c) {
            self.it.next();
            true
        } else {
            false
        }
    }

    /// Whether the next char equals `c`.
    fn peek_is(&mut self, c: char) -> bool {
        matches!(self.it.peek(), Some(&(_, x)) if x == c)
    }

    /// Whether the next char is an ASCII digit.
    fn peek_ascii_digit(&mut self) -> bool {
        matches!(self.it.peek(), Some(&(_, c)) if c.is_ascii_digit())
    }

    /// Consume `kw` iff the upcoming chars match it exactly; otherwise leave the cursor put.
    fn eat_keyword(&mut self, kw: &str) -> bool {
        // The cursor is at the first char; clone to look ahead without consuming on a mismatch.
        let mut look = self.it.clone();
        for want in kw.chars() {
            match look.next() {
                Some((_, c)) if c == want => {}
                _ => return false,
            }
        }
        // Matched — advance the real cursor by the keyword length.
        for _ in kw.chars() {
            self.it.next();
        }
        true
    }

    /// The byte offset of the next char, or the source length at end — a best-effort error position
    /// for "where the cursor is now". The `_fallback` (a token's start) is accepted for call-site
    /// readability but the current position is always the more precise answer.
    fn pos_or_end(&mut self, _fallback: usize) -> usize {
        self.it.peek().map(|&(i, _)| i).unwrap_or(self.src.len())
    }

    /// A span from `start` to the current cursor position (the next char's offset, or source end).
    fn span_to(&mut self, start: usize) -> Span {
        let end = self.it.peek().map(|&(i, _)| i).unwrap_or(self.src.len());
        Span::new(start, end)
    }

    /// Enter one nesting level, refusing input nested past [`MAX_NESTING_DEPTH`].
    fn enter(&mut self) -> Result<(), ReadError> {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            return Err(ReadError(format!(
                "JSON nests deeper than the limit of {MAX_NESTING_DEPTH}"
            )));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // ---- span-recording arena helpers (mirror sexpr/markdown's `mk_*`; push one span per StructId) ----

    fn push_span(&mut self, span: Span) {
        if let Some(t) = self.spans.as_mut() {
            debug_assert_eq!(
                t.len() + 1,
                self.b.structure_len(),
                "json span table drifted from the arena"
            );
            t.push(span);
        }
    }

    fn mk_name(&mut self, name: &str, span: Span) -> StructId {
        let id = self.b.name(name);
        self.push_span(span);
        id
    }

    fn mk_atom_leaf(&mut self, leaf: Leaf, span: Span) -> StructId {
        let id = self.b.atom_leaf(leaf);
        self.push_span(span);
        id
    }

    fn mk_list(&mut self, items: Vec<StructId>, span: Span) -> StructId {
        let id = self.b.list(items);
        self.push_span(span);
        id
    }
}

// ============================================================================
// Printer: value arena -> JSON text
// ============================================================================

/// The indentation step (spaces per nesting level) for the pretty-printer.
const INDENT_STEP: usize = 2;

/// Render a JSON value arena as pretty-printed JSON (2-space indentation). `width` is accepted for
/// surface-layer uniformity; JSON's structure is line-per-member/element, so there is nothing to
/// reflow. A NON-JSON root (e.g. a bare program handed to `cdz convert prog.cdz --to json`) is
/// rendered as a single JSON STRING holding the program's ML rendering, so `--to json` stays total and
/// meaningful (a program is not JSON data, but it is a value that can be carried as a string).
///
/// `ml_print` renders an arbitrary arena as ML text — INJECTED (not called directly) so this crate stays
/// BELOW the ML surface (the facade re-exports it, so a dependency on the ML printer would cycle). Only
/// the non-JSON fallback path invokes it; a JSON-node root never touches it. The facade (and the ML
/// printer, when embedding a `json{…}` sub-document) pass `cadenza_syntax::printer::print`.
pub fn print(arenas: &Arenas, width: usize, ml_print: fn(&Arenas, usize) -> String) -> String {
    let mut out = String::new();
    if is_json_node(arenas, arenas.root) {
        print_value(arenas, arenas.root, 0, &mut out);
    } else {
        // Fallback: carry the program's ML text as a JSON string.
        let ml = ml_print(arenas, width);
        out.push_str(&json_string(&ml));
    }
    out.push('\n');
    out
}

/// Whether `id` roots a node this surface renders as JSON (a json container, or a bare scalar leaf).
/// A foreign program root (e.g. `(+ 1 2)`) is not, and takes the string fallback.
fn is_json_node(a: &Arenas, id: StructId) -> bool {
    match a.head_name(id) {
        Some("json-object") | Some("json-array") | Some("json-null") => true,
        Some(_) => false, // some other List head — a foreign program form
        None => matches!(
            a.get(id),
            // A bare scalar atom is a valid top-level JSON value.
            cadenza_syntax_core::ast::Struct::Atom(_)
        ),
    }
}

fn print_value(a: &Arenas, id: StructId, indent: usize, out: &mut String) {
    match a.head_name(id) {
        Some("json-object") => print_object(a, id, indent, out),
        Some("json-array") => print_array(a, id, indent, out),
        Some("json-null") => out.push_str("null"),
        _ => print_scalar(a, id, out),
    }
}

fn print_object(a: &Arenas, id: StructId, indent: usize, out: &mut String) {
    let members = child_tail(a, id);
    if members.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push('{');
    out.push('\n');
    let inner = indent + INDENT_STEP;
    for (i, &m) in members.iter().enumerate() {
        push_indent(inner, out);
        // (member <key:Str> <value>)
        let items = list_items(a, m);
        let key = items.get(1).and_then(|&k| a.as_str(k)).unwrap_or("");
        out.push_str(&json_string(key));
        out.push_str(": ");
        if let Some(&v) = items.get(2) {
            print_value(a, v, inner, out);
        } else {
            out.push_str("null");
        }
        if i + 1 < members.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(indent, out);
    out.push('}');
}

fn print_array(a: &Arenas, id: StructId, indent: usize, out: &mut String) {
    let elems = child_tail(a, id);
    if elems.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push('[');
    out.push('\n');
    let inner = indent + INDENT_STEP;
    for (i, &e) in elems.iter().enumerate() {
        push_indent(inner, out);
        print_value(a, e, inner, out);
        if i + 1 < elems.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(indent, out);
    out.push(']');
}

/// Render a bare scalar leaf as a JSON scalar. Reuses the shared `literal` renderers so numbers are
/// byte-consistent with the other surfaces: `render_int(_, Dec)` is plain decimal (JSON-legal) and
/// `render_decimal` always emits `<digits>.<digits>` with no exponent (also JSON-legal). A `Bool` is
/// `true`/`false`; a `Str` is JSON-escaped. Any non-JSON leaf (`Char`/`Bytes`/`Sym`/`Name`/markers,
/// which this surface's reader never produces) degrades to a JSON string of its debug text rather than
/// emitting invalid JSON.
fn print_scalar(a: &Arenas, id: StructId, out: &mut String) {
    match a.get(id) {
        cadenza_syntax_core::ast::Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Str(s) => out.push_str(&json_string(s)),
            Leaf::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Leaf::Int { value, .. } => {
                // Always base-10 for JSON, regardless of the leaf's recorded radix.
                out.push_str(&cadenza_syntax_core::literal::render_int(value, Radix::Dec));
            }
            Leaf::Float(d) => out.push_str(&cadenza_syntax_core::literal::render_decimal(d)),
            other => {
                // Not a JSON scalar (a foreign leaf) — carry its rendered text as a string.
                out.push_str(&json_string(&format!("{other:?}")));
            }
        },
        // A bare List with an unrecognized head reaching here — carry a placeholder rather than crash.
        cadenza_syntax_core::ast::Struct::List(_) => out.push_str("null"),
    }
}

/// Encode `s` as a JSON string literal (quotes included): `"` and `\` are backslash-escaped, the C0
/// control chars use their short escapes (`\b \t \n \f \r`) or a `\uXXXX`, and every other char stands
/// for itself (UTF-8). The inverse of the reader's `parse_string`, so `read`→`print` re-reads the same
/// leaf.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Append `n` spaces of indentation.
fn push_indent(n: usize, out: &mut String) {
    for _ in 0..n {
        out.push(' ');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core surface contract: parse → print → parse is a fixed point (arena-idempotent), the same
    /// guarantee the ML and markdown surfaces hold. JSON is not injective, so byte identity of the
    /// source is NOT required — but the TREE is stable.
    fn assert_idempotent(json: &str) {
        let a1 = read(json).expect("valid JSON");
        let printed = print(&a1, 100, |_, _| String::new());
        let a2 = read(&printed).expect("reprinted JSON parses");
        assert!(
            a1.structurally_eq(&a2),
            "not arena-idempotent\n--- source ---\n{json}\n--- reprinted ---\n{printed}"
        );
    }

    #[test]
    fn scalars() {
        assert_idempotent("42");
        assert_idempotent("-7");
        assert_idempotent("0");
        assert_idempotent("3.14");
        assert_idempotent("-0.25");
        assert_idempotent("1.5e10");
        assert_idempotent("true");
        assert_idempotent("false");
        assert_idempotent("null");
        assert_idempotent("\"hello\"");
        assert_idempotent("\"\"");
    }

    #[test]
    fn huge_and_precise_numbers_survive() {
        // A magnitude that would OVERFLOW Int64, and an exponent that would REJECT as Float64, both
        // survive at the leaf layer — a document is never type-checked.
        assert_idempotent("123456789012345678901234567890");
        assert_idempotent("1e400");
        // Exactness: the parsed int is the true BigInt value.
        let a = read("123456789012345678901234567890").unwrap();
        match a.get(a.root) {
            cadenza_syntax_core::ast::Struct::Atom(l) => match a.leaf(*l) {
                Leaf::Int { value, .. } => {
                    assert_eq!(value.to_decimal_string(), "123456789012345678901234567890")
                }
                _ => panic!("expected an Int leaf"),
            },
            _ => panic!("expected a scalar root"),
        }
    }

    #[test]
    fn string_escapes_round_trip() {
        assert_idempotent("\"a\\nb\\tc\"");
        assert_idempotent("\"quote \\\" and backslash \\\\ and slash \\/\"");
        assert_idempotent("\"\\b\\f\"");
        // \u escape and a surrogate PAIR (an astral scalar, U+1F600 😀).
        assert_idempotent("\"unicode \\u0041 and astral \\uD83D\\uDE00 done\"");
        // The decoded content is the real scalar, not the escape text.
        let a = read("\"\\u0041\"").unwrap();
        assert_eq!(a.as_str(a.root), Some("A"));
        let astral = read("\"\\uD83D\\uDE00\"").unwrap();
        assert_eq!(astral.as_str(astral.root), Some("😀"));
    }

    #[test]
    fn objects() {
        assert_idempotent("{}");
        assert_idempotent("{\"a\": 1, \"b\": 2}");
        assert_idempotent("{\"nested\": {\"x\": [1, 2, 3]}}");
    }

    #[test]
    fn object_preserves_key_order_and_duplicates() {
        // A record would reject duplicate keys and sort them; the faithful document preserves BOTH the
        // duplicate `a` and the source order.
        let a = read("{\"b\": 1, \"a\": 2, \"a\": 3}").unwrap();
        let members = child_tail(&a, a.root);
        assert_eq!(members.len(), 3, "all three members, duplicate kept");
        let keys: Vec<&str> = members
            .iter()
            .map(|&m| {
                let items = list_items(&a, m);
                a.as_str(items[1]).unwrap()
            })
            .collect();
        assert_eq!(keys, vec!["b", "a", "a"], "source order preserved");
        assert_idempotent("{\"b\": 1, \"a\": 2, \"a\": 3}");
    }

    #[test]
    fn non_identifier_and_unicode_keys() {
        // Keys a record could never hold: a space, a leading digit, non-ASCII.
        assert_idempotent("{\"first name\": \"Ada\", \"123\": true, \"café\": null}");
        let a = read("{\"first name\": 1}").unwrap();
        let member = child_tail(&a, a.root)[0];
        assert_eq!(a.as_str(list_items(&a, member)[1]), Some("first name"));
    }

    #[test]
    fn arrays() {
        assert_idempotent("[]");
        assert_idempotent("[1, 2, 3]");
        assert_idempotent("[[1], [2, 3], []]");
    }

    #[test]
    fn heterogeneous_array() {
        // A native `list` is homogeneous; the faithful document holds mixed element kinds.
        assert_idempotent("[1, \"a\", true, null, 2.5, {\"k\": []}]");
        let a = read("[1, \"a\", true, null]").unwrap();
        assert_eq!(a.head_name(a.root), Some("json-array"));
        assert_eq!(child_tail(&a, a.root).len(), 4);
    }

    #[test]
    fn realistic_document() {
        assert_idempotent(
            "{\"name\": \"Ada\", \"tags\": [\"math\", \"code\"], \"active\": true, \"note\": null, \"score\": 9.5}",
        );
    }

    #[test]
    fn null_is_a_distinct_node() {
        let a = read("null").unwrap();
        assert_eq!(a.head_name(a.root), Some("json-null"));
    }

    #[test]
    fn errors_are_refused() {
        for bad in [
            "",               // empty
            "{",              // unterminated object
            "[",              // unterminated array
            "[1,]",           // trailing comma in array
            "{\"a\": 1,}",    // trailing comma in object
            "{\"a\" 1}",      // missing colon
            "{a: 1}",         // unquoted key
            "nul",            // truncated literal
            "tru",            // truncated literal
            "1 2",            // trailing garbage
            "01",             // leading zero
            "+1",             // leading plus
            "1.",             // no digit after point
            "1e",             // no digit in exponent
            ".5",             // no integer part
            "\"unterminated", // unterminated string
            "\"\\uD83D\"",    // lone high surrogate
            "\"\\uDE00\"",    // lone low surrogate
            "\"\\q\"",        // invalid escape
            "\"\\u00G0\"",    // bad hex in \u
        ] {
            assert!(
                read(bad).is_err(),
                "expected a parse error for {bad:?}, got Ok"
            );
        }
    }

    #[test]
    fn json_to_binary_round_trips() {
        // Through the canonical binary form and back — the arena survives.
        let src = "{\"a\": [1, 2, {\"b\": null}], \"c\": \"x\"}";
        let a1 = read(src).unwrap();
        let bin = cadenza_ast::codec::encode(&a1);
        let a2 = cadenza_ast::codec::decode(&bin).expect("decodes");
        assert!(a1.structurally_eq(&a2));
        // And printing the decoded arena re-reads to the same tree.
        let printed = print(&a2, 100, |_, _| String::new());
        let a3 = read(&printed).unwrap();
        assert!(a1.structurally_eq(&a3));
    }

    // NOTE: `non_json_root_falls_back_to_json_string` moved to `cadenza-syntax`'s in-crate
    // `surface_tests` — it exercises the ML-printer fallback (a non-JSON root → a JSON string carrying
    // the ML text), which needs the ML printer + the sexpr reader, neither of which this
    // below-the-surface crate may depend on.

    #[test]
    fn span_table_is_total_and_ordered() {
        // read_spanned keeps a 1:1 span table; every node has a span, in id order (the debug_assert in
        // push_span also guards this during construction).
        let (a, spans) = read_spanned("{\"a\": [1, true], \"b\": null}").unwrap();
        assert_eq!(spans.len(), a.structure.len());
        for id in (0..a.structure.len() as u32).map(StructId) {
            assert!(spans.get(id).is_some(), "node {id:?} has a span");
        }
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Generate a random VALID JSON value string, bounded by `depth` (depth 0 = a scalar, so generation
    /// terminates). Mixes objects (incl. duplicate/unicode keys), arrays (heterogeneous), null, and the
    /// scalar kinds (int, float, exponent, string with escapes, bool) — the properties the faithful JSON
    /// surface must preserve.
    fn gen_json(rng: &mut Rng, depth: usize) -> String {
        let scalar = |rng: &mut Rng| -> String {
            match rng.below(8) {
                0 => rng.below(100000).to_string(),
                1 => format!("-{}", rng.below(1000)),
                2 => format!("{}.{}", rng.below(1000), rng.below(1000)),
                3 => format!("{}e{}", rng.below(100), rng.below(30)),
                4 => "true".to_string(),
                5 => "false".to_string(),
                6 => "null".to_string(),
                // A string with an escape and a unicode char.
                _ => format!("\"s{}\\n\\t\\\"{}\"", rng.below(100), "é中"),
            }
        };
        if depth == 0 {
            return scalar(rng);
        }
        match rng.below(5) {
            0 | 1 => scalar(rng),
            2 => {
                // array (0..=3 elements), possibly heterogeneous
                let n = rng.below(4);
                let elems: Vec<String> = (0..n).map(|_| gen_json(rng, depth - 1)).collect();
                format!("[{}]", elems.join(","))
            }
            _ => {
                // object (0..=3 members); keys drawn from a small set to exercise DUPLICATE keys, plus a
                // unicode key — all of which the surface must preserve verbatim (never dedup/normalize).
                let keys = ["a", "b", "a", "duplicate", "ké中"];
                let n = rng.below(4);
                let members: Vec<String> = (0..n)
                    .map(|_| {
                        let k = keys[rng.below(keys.len())];
                        format!("\"{}\":{}", k, gen_json(rng, depth - 1))
                    })
                    .collect();
                format!("{{{}}}", members.join(","))
            }
        }
    }

    #[test]
    fn json_surface_is_idempotent_over_generated_documents() {
        // The surface contract (arena-idempotence: read(print(read(json))) == read(json)) swept over
        // random JSON, complementing the hand-picked cases above. A generator explores nestings and
        // duplicate-key / unicode-key / heterogeneous-array / exact-number COMBINATIONS the fixed tests
        // don't, so a printer/parser asymmetry that no hand-written case hits still gets caught. Fixed
        // seeds → reproducible; a failure prints the source + reprint via `assert_idempotent`.
        let seeds: [u64; 3] = [
            0x0bad_c0de_dead_beef,
            0x5eed_1234_5678_9abc,
            0xfeed_face_cafe_babe,
        ];
        let mut total = 0usize;
        for &seed in &seeds {
            let mut rng = Rng(seed);
            for _ in 0..1500 {
                let depth = 1 + rng.below(4);
                assert_idempotent(&gen_json(&mut rng, depth));
                total += 1;
            }
        }
        assert!(total >= 4000, "swept a meaningful space, got {total}");
    }

    #[test]
    fn a_generated_json_document_survives_the_binary_codec_round_trip() {
        // The binary codec is the CANONICAL STORED form for a data document too (not just code), so it
        // must faithfully preserve a JSON value arena. `json_to_binary_round_trips` pins ONE hand doc;
        // this sweeps it: for random JSON, `read → encode → decode` is structurally identical to the
        // parsed arena, AND printing the decoded arena re-reads to the same tree (so a doc survives
        // `cdz convert --from json --to binary` and back). A codec/JSON-surface mismatch on some
        // generated shape (deep nesting, duplicate/unicode keys, heterogeneous arrays, exact/edge numbers)
        // would silently corrupt stored data — the sweep the hand case can't reach. Also asserts encode is
        // DETERMINISTIC (re-encoding the decoded arena reproduces the bytes — the bijection guarantee).
        let seeds: [u64; 3] = [
            0x1a7e_c0de_0bad_f00d,
            0x5eed_face_1234_abcd,
            0xd06f_00d5_beef_cafe,
        ];
        let mut total = 0usize;
        for &seed in &seeds {
            let mut rng = Rng(seed);
            for _ in 0..1500 {
                let depth = 1 + rng.below(4);
                let src = gen_json(&mut rng, depth);
                let Ok(a1) = read(&src) else { continue };
                let bin = cadenza_ast::codec::encode(&a1);
                let a2 = cadenza_ast::codec::decode(&bin)
                    .expect("a JSON arena decodes from its own encoding");
                assert!(
                    a1.structurally_eq(&a2),
                    "JSON arena survives binary round-trip for {src}"
                );
                // Determinism: re-encoding the decoded arena reproduces the exact bytes.
                assert_eq!(
                    bin,
                    cadenza_ast::codec::encode(&a2),
                    "binary encode is deterministic for {src}"
                );
                // And the decoded arena prints back to a tree that re-reads identically.
                let a3 = read(&print(&a2, 100, |_, _| String::new()))
                    .expect("decoded-then-printed JSON re-reads");
                assert!(
                    a1.structurally_eq(&a3),
                    "JSON survives binary → print → re-read for {src}"
                );
                total += 1;
            }
        }
        assert!(total >= 4000, "swept a meaningful codec space, got {total}");
    }

    #[test]
    fn json_read_never_panics_on_arbitrary_input() {
        // `read` operates on UNTRUSTED text; it must return a diagnostic, never panic. Sweep random
        // byte-ish strings (drawn from JSON's structural chars + digits + escapes + unicode) plus
        // truncated/odd fragments. Any panic (OOB slice, unwrap, bad-UTF-8 boundary) fails this test.
        // On a SUCCESSFUL read the arena must also be well-formed with a TOTAL span table (a broken
        // table would silently corrupt a span-based edit) — see `assert_json_read_invariants`.
        let alphabet: Vec<char> = "{}[]\":,0123456789.-+eEtfn\\/ \tλ".chars().collect();
        let mut rng = Rng(0x1357_9bdf_2468_ace0);
        for len in 0..=32usize {
            for _ in 0..80 {
                let s: String = (0..len)
                    .map(|_| alphabet[rng.below(alphabet.len())])
                    .collect();
                assert_json_read_invariants(&s);
            }
        }
        // A few deliberately truncated openers.
        for s in [
            "{",
            "[",
            "\"",
            "{\"a\":",
            "[1,",
            "\"\\u",
            "-",
            "1e",
            "tru",
            "nul",
            "{\"a\":1,",
        ] {
            assert_json_read_invariants(s);
        }
    }

    /// `read` must not panic on arbitrary input, and on a SUCCESSFUL read the arena is well-formed with
    /// a TOTAL span table: `read`/`read_spanned` agree structurally, the arena is non-empty with root in
    /// range, `spans` is exactly 1:1 with the structure vector, and every reachable child id is in range.
    /// A clean `ReadError` on malformed input is fine (the point is no crash + a sound arena when it does
    /// parse). Mirrors the ML/s-expr/markdown reader fuzzes.
    fn assert_json_read_invariants(src: &str) {
        let plain = read(src); // must not panic
        let Ok((a, spans)) = read_spanned(src) else {
            assert!(plain.is_err(), "read_spanned Err but read Ok for {src:?}");
            return;
        };
        assert!(
            plain.is_ok_and(|p| p.structurally_eq(&a)),
            "read and read_spanned disagree for {src:?}"
        );
        let n = a.structure.len();
        assert!(
            n > 0 && (a.root.0 as usize) < n,
            "root id in range for {src:?}"
        );
        assert_eq!(spans.len(), n, "span table is total for {src:?}");
        // Every span is a GEOMETRICALLY VALID slice of the source — ordered, in-bounds, on UTF-8 char
        // boundaries — even on malformed input. Totality only says a span EXISTS per node; this says
        // `&src[sp.start..sp.end]` (an LSP hover / diagnostic underline / span-based edit) can be taken
        // WITHOUT panicking. The reader synthesizes spans for structural nodes (objects/arrays), so an
        // off-by-one or a span past a truncated source is a real risk on the error path.
        for id in (0..n as u32).map(StructId) {
            let sp = spans.get(id).expect("total span table");
            assert!(
                sp.start <= sp.end
                    && sp.end <= src.len()
                    && src.is_char_boundary(sp.start)
                    && src.is_char_boundary(sp.end),
                "span {sp:?} for node {id:?} is not a valid slice of {src:?}"
            );
        }
        fn walk(a: &Arenas, id: StructId) {
            if let cadenza_syntax_core::ast::Struct::List(kids) = a.get(id) {
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
