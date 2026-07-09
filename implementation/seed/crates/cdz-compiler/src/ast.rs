//! The canonical representation: a homoiconic s-expression AST.
//!
//! A program's canonical stored form is the binary serialization of this tree
//! (spec/contracts/ast-encoding.md; options/ast-encoding/binary-sexpr.md). We model the
//! tree in the classic homoiconic way — a node is an atom or a list of nodes — which is
//! exactly the corpus display form (options/code-shape/homoiconic-decoupled-display.md)
//! and what the interpreter walks: a list's head names a core construct or a function to
//! apply. Code is data (bootstrap-interpreter.md §"A Program's Syntax Tree Is An Ordinary
//! Value").
//!
//! The reader (text -> Node) is NOT in the compiler's trusted derivation path
//! (ast-encoding.md §"Parsing And Printing Are Not In The Compiler's Trusted Path").

use std::fmt;

/// A node of the abstract syntax tree: an atom (leaf primitive), a name (identifier),
/// or a list (a form — `(head child…)`). This uniform shape is the container the
/// binary encoding serializes; construct kinds are just the head symbol of a list, so
/// adding a construct adds no container variant (ast-encoding.md §"The Encoding Is
/// General And Stable").
#[derive(Clone, PartialEq, Debug)]
pub enum Node {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    /// An identifier: a name reference, a construct head, a variant, or a qualified name.
    Name(String),
    /// A form `(child…)`. An empty list is not produced by the reader.
    List(Vec<Node>),
}

impl Node {
    /// If this is a list whose head is the name `head`, return the tail (the arguments).
    pub fn as_form<'a>(&'a self, head: &str) -> Option<&'a [Node]> {
        if let Node::List(items) = self {
            if let Some(Node::Name(h)) = items.first() {
                if h == head {
                    return Some(&items[1..]);
                }
            }
        }
        None
    }
    /// The head name of a list, if it is a list headed by a name.
    pub fn head_name(&self) -> Option<&str> {
        if let Node::List(items) = self {
            if let Some(Node::Name(h)) = items.first() {
                return Some(h);
            }
        }
        None
    }
}

// ============================================================================
// Textual s-expression reader — text -> Node.
// ============================================================================

#[derive(Debug)]
pub struct ReadError(pub String);

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "read error: {}", self.0)
    }
}
impl std::error::Error for ReadError {}

/// Parse a single s-expression from `text`.
pub fn read(text: &str) -> Result<Node, ReadError> {
    let mut p = Reader::new(text);
    p.skip_ws();
    let node = p.read_node()?;
    p.skip_ws();
    if p.peek().is_some() {
        return Err(ReadError(format!("trailing input at byte {}", p.pos)));
    }
    Ok(node)
}

/// Parse every top-level s-expression from `text` (a corpus file of cases).
pub fn read_all(text: &str) -> Result<Vec<Node>, ReadError> {
    let mut p = Reader::new(text);
    let mut out = Vec::new();
    loop {
        p.skip_ws();
        if p.peek().is_none() {
            break;
        }
        out.push(p.read_node()?);
    }
    Ok(out)
}

/// Read a PROGRAM: the top-level forms of a source unit, synthesizing the implicit-module `(do …)`
/// wrapper when there is more than one top-level form. A program is a sequence of top-level forms
/// (defs / types / exports) with no explicit `(module …)` wrapper; a `do` block scopes a declaration
/// to the forms that follow, so the whole program is one `(do <form>…)` node — a single `Node`
/// downstream. A single top-level form is returned unwrapped (a one-definition program); an empty
/// source is an error. This is where "the reader assumes a top-level `do`" lives.
pub fn read_program(text: &str) -> Result<Node, ReadError> {
    let mut forms = read_all(text)?;
    match forms.len() {
        0 => Err(ReadError("empty program".into())),
        1 => Ok(forms.pop().unwrap()),
        _ => {
            let mut items = Vec::with_capacity(forms.len() + 1);
            items.push(Node::Name("do".into()));
            items.append(&mut forms);
            Ok(Node::List(items))
        }
    }
}

struct Reader<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str) -> Reader<'a> {
        Reader { src: text.as_bytes(), pos: 0 }
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

    fn read_node(&mut self) -> Result<Node, ReadError> {
        self.skip_ws();
        match self.peek() {
            None => Err(ReadError("unexpected end of input".into())),
            Some(b'(') => self.read_list(),
            Some(b')') => Err(ReadError(format!("unexpected ')' at byte {}", self.pos))),
            Some(b'"') => self.read_string(),
            // `b"…"` is READER SUGAR for a byte sequence — it reads to `(Bytes.of (list b0 b1 …))`,
            // the way `a.b` is sugar for `(. a b)`, so the canonical tree carries only `Bytes.of`
            // and there is no new node kind (options/binary-syntax; homoiconic-decoupled-display.md
            // §"Member access"). The `b` sigil is a byte-string literal ONLY when a `"` follows it
            // immediately; a bare `b` (or `b` starting a longer name like `bin`) is an ordinary name.
            Some(b'b') if self.src.get(self.pos + 1) == Some(&b'"') => self.read_byte_string(),
            Some(b'`') => {
                self.bump();
                let inner = self.read_node()?;
                Ok(Node::List(vec![Node::Name("quasiquote".into()), inner]))
            }
            Some(b',') => {
                self.bump();
                if self.peek() == Some(b'@') {
                    self.bump();
                    let inner = self.read_node()?;
                    Ok(Node::List(vec![Node::Name("unquote-splicing".into()), inner]))
                } else {
                    let inner = self.read_node()?;
                    Ok(Node::List(vec![Node::Name("unquote".into()), inner]))
                }
            }
            Some(_) => self.read_atom_or_name(),
        }
    }

    fn read_list(&mut self) -> Result<Node, ReadError> {
        self.bump(); // '('
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(ReadError("unterminated list".into())),
                Some(b')') => {
                    self.bump();
                    break;
                }
                Some(_) => items.push(self.read_node()?),
            }
        }
        Ok(Node::List(items))
    }

    fn read_string(&mut self) -> Result<Node, ReadError> {
        self.bump(); // opening quote
        let mut bytes: Vec<u8> = Vec::new();
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
                    Some(other) => bytes.push(other),
                    None => return Err(ReadError("unterminated escape".into())),
                },
                Some(b) => bytes.push(b),
            }
        }
        let s = String::from_utf8(bytes).map_err(|_| ReadError("non-utf8 string".into()))?;
        // Normalize to Unicode NFC (options/hashing-and-encoding §String-value text normalization;
        // collections-and-text.md #String Equality Follows Normalized Contents): the composed
        // "café" (U+00E9) and the decomposed "café" (e + U+0301) canonicalize to one scalar-value
        // sequence, so equality and length see one normalized form (13-strings.sexp §"strings
        // differing only in Unicode normalization are equal", §"string length counts scalar values
        // after normalization").
        Ok(Node::Str(unicode_normalization::UnicodeNormalization::nfc(s.chars()).collect()))
    }

    /// Read a `b"…"` byte-string literal into `(Bytes.of (list b0 b1 …))`. The `b` sigil and the
    /// opening quote have already been peeked (`read_node` dispatched here on `b"`). A byte-string
    /// is a raw byte sequence, NOT text: each source byte contributes one byte, and the escape set
    /// is the EXACT inverse of the byte-sequence renderer (`b"…"` display form; matching the `bytes`
    /// crate's `Debug`) — `\n \r \t \\ \" \0` and `\xNN` (two lowercase-or-uppercase hex digits) —
    /// so a rendered byte sequence reads back to the same value (round-trips). A printable ASCII
    /// byte stands for itself. Because there is no new node kind, the canonical tree is identical to
    /// the one the explicit `(Bytes.of (list …))` form produces.
    fn read_byte_string(&mut self) -> Result<Node, ReadError> {
        self.bump(); // the `b` sigil
        self.bump(); // opening quote
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match self.bump() {
                None => return Err(ReadError("unterminated byte string".into())),
                Some(b'"') => break,
                Some(b'\\') => match self.bump() {
                    Some(b'n') => bytes.push(b'\n'),
                    Some(b'r') => bytes.push(b'\r'),
                    Some(b't') => bytes.push(b'\t'),
                    Some(b'\\') => bytes.push(b'\\'),
                    Some(b'"') => bytes.push(b'"'),
                    Some(b'0') => bytes.push(0),
                    Some(b'x') => {
                        // `\xNN` — exactly two hex digits, denoting one byte (any value 0..=255).
                        let hi = self.bump().and_then(hex_digit);
                        let lo = self.bump().and_then(hex_digit);
                        match (hi, lo) {
                            (Some(h), Some(l)) => bytes.push(h * 16 + l),
                            _ => return Err(ReadError("malformed \\x byte escape".into())),
                        }
                    }
                    Some(_) => return Err(ReadError("unknown byte-string escape".into())),
                    None => return Err(ReadError("unterminated escape".into())),
                },
                Some(b) => bytes.push(b),
            }
        }
        // Build `(Bytes.of (list b0 b1 …))` — the dotted head `Bytes.of` is the member-access tree
        // `(. Bytes of)`, exactly as the reader expands the dotted token, so `b"…"` and the explicit
        // form produce byte-identical canonical trees.
        let dotted_of = Node::List(vec![
            Node::Name(".".to_string()),
            Node::Name("Bytes".to_string()),
            Node::Name("of".to_string()),
        ]);
        let mut list_items = vec![Node::Name("list".to_string())];
        list_items.extend(bytes.into_iter().map(|b| Node::Int(b as i64)));
        Ok(Node::List(vec![dotted_of, Node::List(list_items)]))
    }

    fn read_atom_or_name(&mut self) -> Result<Node, ReadError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b';') {
                break;
            }
            self.pos += 1;
        }
        let tok = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| ReadError("non-utf8 token".into()))?;
        Ok(classify_token(tok))
    }
}

/// The value 0..=15 of a single ASCII hex digit (`0-9`, `a-f`, `A-F`), or None. Used by the
/// `b"…"` byte-string reader to decode a `\xNN` escape into one byte.
fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Classify a whitespace-delimited token into an atom or a name.
///
/// A **dotted** token `a.b.c` is display sugar for nested member access: the reader
/// expands it to `(. (. a b) c)` so the canonical tree carries only the explicit `.`
/// form and there is no dotted-atom ambiguity downstream (options/code-shape/
/// homoiconic-decoupled-display.md §"Member access"). A qualified name like `Sign.Neg`
/// therefore becomes `(. Sign Neg)` — `Sign` is an ordinary prelude binding, and `.`
/// looks `Neg` up in it — exactly like a field projection `p.x`.
fn classify_token(tok: &str) -> Node {
    match tok {
        "true" => return Node::Bool(true),
        "false" => return Node::Bool(false),
        _ => {}
    }
    if looks_like_int(tok) {
        if let Some(i) = parse_int_literal(tok) {
            return Node::Int(i);
        }
    }
    if looks_like_float(tok) {
        let cleaned: String = tok.chars().filter(|&c| c != '_').collect();
        if let Ok(fl) = cleaned.parse::<f64>() {
            return Node::Float(fl);
        }
    }
    // Dotted-name sugar → nested member access, but only for a "value.member" shape
    // (a segmented identifier with non-empty segments). A leading/trailing/doubled dot
    // is left as a plain name (e.g. the `.` operator token itself, or a float already
    // handled above).
    if is_dotted_name(tok) {
        let mut segs = tok.split('.');
        let mut node = Node::Name(segs.next().unwrap().to_string());
        for seg in segs {
            node = Node::List(vec![Node::Name(".".to_string()), node, Node::Name(seg.to_string())]);
        }
        return node;
    }
    Node::Name(tok.to_string())
}

/// True for a `a.b`(`.c…`) identifier: at least one dot, every segment non-empty and not
/// itself numeric (so a float like `3.5` — already parsed above — never reaches here, and
/// a bare `.` operator token is not treated as dotted).
fn is_dotted_name(tok: &str) -> bool {
    if !tok.contains('.') {
        return false;
    }
    let segs: Vec<&str> = tok.split('.').collect();
    if segs.len() < 2 {
        return false;
    }
    segs.iter().all(|s| {
        !s.is_empty() && s.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_')
    })
}

fn looks_like_int(tok: &str) -> bool {
    let body = tok.strip_prefix('-').or_else(|| tok.strip_prefix('+')).unwrap_or(tok);
    // A radix-prefixed literal `0x…`/`0b…` is a hex/binary integer: it starts with a digit
    // (`0`), so it is numeric in shape, and its body digits are drawn from the radix's alphabet
    // (01-literals.sexp §"a hexadecimal integer literal"). Recognize these before the plain
    // decimal shape so the `x`/`b` and hex letters are not mistaken for a name.
    if let Some(radix_body) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0b")) {
        let is_hex = body.as_bytes().get(1) == Some(&b'x');
        // A radix literal needs at least one radix digit after the prefix, and every body char
        // is a radix digit or a between-digits separator (well-formed placement).
        return !radix_body.is_empty()
            && radix_body.chars().next().map_or(false, |c| is_radix_digit(c, is_hex))
            && radix_body.chars().all(|c| is_radix_digit(c, is_hex) || c == '_')
            && separators_between_digits(radix_body, |c| is_radix_digit(c, is_hex));
    }
    // A digit separator `_` is only valid BETWEEN digits, so an integer literal must START
    // with a digit — otherwise `_1` (a leading underscore) is an identifier, not the int 1 —
    // and must not have a TRAILING or DOUBLED separator (`1_`, `1__2` are malformed, NOT the
    // digits with `_` dropped — 01-literals.sexp §"a trailing digit separator is a malformed
    // literal"). Rejecting the shape here leaves such a token as a `Node::Name`, which the
    // compiler reports as a malformed numeric literal (CDZ0201) via `looks_like_numeric_literal`.
    body.chars().next().map_or(false, |c| c.is_ascii_digit())
        && body.chars().all(|c| c.is_ascii_digit() || c == '_')
        && separators_between_digits(body, |c| c.is_ascii_digit())
}

/// True iff every `_` digit separator in `body` sits BETWEEN two `is_digit` characters — i.e. no
/// leading, trailing, or doubled separator. `body` is the digit sequence (radix prefix already
/// stripped for `0x…`/`0b…`, sign already stripped). A `_` is well-formed only when both its
/// immediate neighbors are digits, matching the language's between-digits separator rule in BOTH
/// directions (so `1_`, `_1`, `1__2` are all rejected, while `1_000_000` is accepted).
fn separators_between_digits(body: &str, is_digit: impl Fn(char) -> bool) -> bool {
    let chars: Vec<char> = body.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            let prev_ok = i > 0 && is_digit(chars[i - 1]);
            let next_ok = i + 1 < chars.len() && is_digit(chars[i + 1]);
            if !(prev_ok && next_ok) {
                return false;
            }
        }
    }
    true
}

/// True if `c` is a digit of the given radix — decimal-or-hex when `is_hex`, else binary.
fn is_radix_digit(c: char, is_hex: bool) -> bool {
    if is_hex {
        c.is_ascii_hexdigit()
    } else {
        c == '0' || c == '1'
    }
}

/// Parse an integer literal token — decimal, `0x…` hexadecimal, or `0b…` binary — into an i64,
/// stripping the `_` digit separators first. Returns `None` if the value is outside the i64 range
/// (the caller leaves such a token as a `Node::Name`, which the compiler later reports as an
/// out-of-range malformed literal — 01-literals.sexp §"a hexadecimal literal past Int64.max").
/// A radix literal is strict non-negative: it denotes its face value, so a `0x…` that fills all 64
/// bits is out of range rather than a two's-complement pattern. The optional leading sign is kept
/// attached and handed to the parser (both `parse` and `from_str_radix` accept a leading `-`/`+`),
/// so `Int64.min` — whose magnitude is one past `Int64.max` — parses as the whole signed token
/// rather than overflowing an intermediate positive magnitude.
fn parse_int_literal(tok: &str) -> Option<i64> {
    // Split an optional leading sign from the body so the `0x`/`0b` prefix check sees the body,
    // then re-attach the sign to the digits (dropping the radix prefix) for the parser.
    let (sign, body) = match tok.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", tok.strip_prefix('+').unwrap_or(tok)),
    };
    let body: String = body.chars().filter(|&c| c != '_').collect();
    if let Some(hex) = body.strip_prefix("0x") {
        i64::from_str_radix(&format!("{sign}{hex}"), 16).ok()
    } else if let Some(bin) = body.strip_prefix("0b") {
        i64::from_str_radix(&format!("{sign}{bin}"), 2).ok()
    } else {
        format!("{sign}{body}").parse::<i64>().ok()
    }
}

fn looks_like_float(tok: &str) -> bool {
    let body = tok.strip_prefix('-').or_else(|| tok.strip_prefix('+')).unwrap_or(tok);
    // A float literal must START with a digit, so a leading-underscore token like `_1.0` is
    // an identifier, not a float (matching the integer rule).
    if !body.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return false;
    }
    let has_point_or_exp = body.contains('.') || body.contains('e') || body.contains('E');
    has_point_or_exp
        && body.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-' | '_'))
        // A digit separator `_` is only valid BETWEEN two digits — the SAME rule integers use
        // (`separators_between_digits`), applied to floats too. So `1._5` (a `_` after the decimal
        // point, whose left neighbor `.` is not a digit), `1.5_`, `1_.5`, `1.5__0`, `1.5e_10` are
        // NOT floats: they fail this check, so `looks_like_float` returns false and the token is left
        // as a `Node::Name`, which the compiler reports as a malformed numeric literal (CDZ0201) via
        // `looks_like_numeric_literal`. Without it the reader stripped EVERY `_` regardless of
        // position and silently read `1._5` as `1.5` (01-literals.sexp §"a digit separator adjacent
        // to a float's decimal point is a malformed literal"). The valid `1.2_5` (between digits) is
        // still accepted.
        && separators_between_digits(body, |c| c.is_ascii_digit())
}

// ============================================================================
// Binary AST codec — the canonical STORED form (spec/contracts/ast-encoding.md;
// options/ast-encoding/binary-sexpr.md).
//
// The compiler is supplied its input as the binary AST, not as text
// (ast-encoding.md §"A program MUST be supplied to the compiler as its binary AST";
// §"The compiler MUST accept the canonical binary AST directly"). The seed provides
// this codec (reading/serializing the canonical form is the seed's job, kept out of the
// compiler's own logic — ast-encoding.md §"Parsing And Printing Are Not In The Compiler's
// Trusted Path"; bootstrap-interpreter.md glue "decodes the embedded binary-AST bytes
// into a Node value").
//
// The stored file is the triple `[container-version, prelude, root]` serialized as
// deterministic CBOR (binary-sexpr.md §"Concrete encoding"). The prelude is the
// canonically-sorted list of the distinct symbol names the tree references; a node names
// its kind by referencing a prelude index rather than carrying the symbol inline
// (ast-encoding.md §"The File Carries Its Own Symbol Prelude"). In node position an atom
// is a CBOR scalar and an application is a CBOR array `[head-index, ...children]`
// (binary-sexpr.md structural schema), so the two never collide.
//
// SEED SCOPING (recorded in implementation/DECISIONS.md): symbols carry a name only — the
// namespace/version fields binary-sexpr.md permits are simplified to a single implicit
// namespace with no version, sufficient for the ignition corpus and the compiler's own
// source; a later generation realizes the full namespaced/versioned prelude. The
// load-bearing contract properties hold: one canonical byte form per tree, equal trees
// encode identically, and decode∘encode is the identity (ast-encoding.md §"The Encoding
// Is A Bijection With One Canonical Byte Form").
// ============================================================================

/// The container encoding version this codec implements (ast-encoding.md §"The Encoding
/// Is Versioned"). A reader refuses a version it does not implement.
pub const CONTAINER_VERSION: u64 = 1;

/// Serialize a `Node` to its canonical binary AST bytes.
//= spec/contracts/ast-encoding.md#the-encoding-is-a-bijection-with-one-canonical-byte-form
//# Each abstract syntax tree MUST have exactly one canonical binary encoding.
pub fn encode(root: &Node) -> Vec<u8> {
    // Collect the distinct symbols (the head names of applications and the names of
    // nullary references), then sort canonically so equal trees produce identical
    // preludes and thus identical indices (ast-encoding.md §"The Prelude Order Is Canonical").
    let mut symbols: Vec<String> = Vec::new();
    collect_symbols(root, &mut symbols);
    symbols.sort();
    symbols.dedup();

    let index_of = |name: &str| -> u64 {
        symbols.binary_search(&name.to_string()).expect("symbol present") as u64
    };

    let prelude = ciborium::Value::Array(symbols.iter().map(|s| ciborium::Value::Text(s.clone())).collect());
    let root_cbor = node_to_cbor(root, &index_of);
    let file = ciborium::Value::Array(vec![
        ciborium::Value::Integer(CONTAINER_VERSION.into()),
        prelude,
        root_cbor,
    ]);
    let mut out = Vec::new();
    ciborium::ser::into_writer(&file, &mut out).expect("cbor encode of binary AST");
    out
}

/// Decode a canonical binary AST back to a `Node`.
//= spec/contracts/ast-encoding.md#the-encoding-is-a-bijection-with-one-canonical-byte-form
//# Decoding a canonical binary encoding MUST yield the abstract syntax tree it was encoded from.
pub fn decode(bytes: &[u8]) -> Result<Node, ReadError> {
    // Decode over a cursor so we can detect TRAILING bytes: a canonical encoding is exactly one
    // CBOR container and nothing more, so any bytes left unconsumed after the container are an
    // error (deterministic-value-form.md #Decoding Refuses — "consumed the whole input exactly" is
    // part of a successful decode of untrusted external bytes; trailing bytes are NOT silently
    // dropped). `from_reader` stops at the end of the first value, so the cursor's position after it
    // tells us how many bytes the container used.
    let mut cursor = std::io::Cursor::new(bytes);
    let file: ciborium::Value = ciborium::de::from_reader(&mut cursor)
        .map_err(|e| ReadError(format!("binary AST decode: {e}")))?;
    if (cursor.position() as usize) != bytes.len() {
        return Err(ReadError("binary AST has trailing bytes after the canonical encoding".into()));
    }
    let items = match &file {
        ciborium::Value::Array(a) if a.len() == 3 => a,
        _ => return Err(ReadError("binary AST is not a [version, prelude, root] triple".into())),
    };
    // Container version: refuse a version this reader does not implement.
    //= spec/contracts/ast-encoding.md#the-encoding-is-versioned
    //# A reader MUST refuse a binary AST whose container encoding version it does not implement rather than misinterpret it.
    match &items[0] {
        ciborium::Value::Integer(v) if i128::from(*v) == CONTAINER_VERSION as i128 => {}
        ciborium::Value::Integer(_) => return Err(ReadError("unsupported container version".into())),
        _ => return Err(ReadError("malformed container version".into())),
    }
    let symbols: Vec<String> = match &items[1] {
        ciborium::Value::Array(a) => a
            .iter()
            .map(|s| match s {
                ciborium::Value::Text(t) => Ok(t.clone()),
                _ => Err(ReadError("prelude entry is not a symbol name".into())),
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(ReadError("prelude is not an array".into())),
    };
    cbor_to_node(&items[2], &symbols)
}

fn collect_symbols(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Name(n) => out.push(n.clone()),
        Node::List(items) => {
            // The head, if a Name, is the application's symbol; otherwise the list is a
            // computed-callee application and gets the reserved `apply` head symbol.
            match items.first() {
                Some(Node::Name(h)) => out.push(h.clone()),
                _ => out.push(APPLY_SYMBOL.to_string()),
            }
            for child in items {
                collect_symbols(child, out);
            }
        }
        _ => {}
    }
}

/// The reserved head symbol for an application whose callee is a computed expression
/// (a list-headed form like `((fn (x) x) 5)`), so every application has a symbol head as
/// binary-sexpr.md requires ("In node position an array is always an application").
const APPLY_SYMBOL: &str = "apply";

/// CBOR tag distinguishing a bare name-reference (a `Name` node) from an application
/// array in node position, so `main` (a reference) and `(main)` (a nullary application)
/// are distinct trees rather than colliding on `[index]`.
const NAME_TAG: u64 = 39;

fn node_to_cbor(node: &Node, index_of: &impl Fn(&str) -> u64) -> ciborium::Value {
    use ciborium::Value as C;
    match node {
        Node::Int(i) => C::Integer((*i).into()),
        Node::Float(f) => C::Float(*f),
        Node::Bool(b) => C::Bool(*b),
        Node::Str(s) => C::Text(s.clone()),
        // A bare name reference: a tagged symbol index, distinct from an application array.
        Node::Name(n) => C::Tag(NAME_TAG, Box::new(C::Integer(index_of(n).into()))),
        Node::List(items) => match items.first() {
            Some(Node::Name(h)) => {
                // [head-index, ...children] — an application of a symbol head. A nullary
                // list `(main)` is a 1-element array `[index]`, distinct from the tagged
                // bare-name reference above.
                let mut arr = vec![C::Integer(index_of(h).into())];
                for child in &items[1..] {
                    arr.push(node_to_cbor(child, index_of));
                }
                C::Array(arr)
            }
            _ => {
                // Computed-callee application: [apply-index, ...all-elements].
                let mut arr = vec![C::Integer(index_of(APPLY_SYMBOL).into())];
                for child in items {
                    arr.push(node_to_cbor(child, index_of));
                }
                C::Array(arr)
            }
        },
    }
}

fn cbor_to_node(v: &ciborium::Value, symbols: &[String]) -> Result<Node, ReadError> {
    use ciborium::Value as C;
    let symbol_at = |idx: usize| -> Result<String, ReadError> {
        //= spec/contracts/ast-encoding.md#the-file-carries-its-own-symbol-prelude
        //# A node MUST name its kind by referencing a symbol in the prelude by index rather than by carrying the symbol inline.
        symbols
            .get(idx)
            .cloned()
            .ok_or_else(|| ReadError(format!("symbol index {idx} out of range")))
    };
    match v {
        C::Integer(i) => Ok(Node::Int(i128::from(*i) as i64)),
        C::Float(f) => Ok(Node::Float(*f)),
        C::Bool(b) => Ok(Node::Bool(*b)),
        C::Text(s) => Ok(Node::Str(s.clone())),
        // A tagged symbol index is a bare name reference.
        C::Tag(NAME_TAG, inner) => match inner.as_ref() {
            C::Integer(i) => Ok(Node::Name(symbol_at(i128::from(*i) as usize)?)),
            _ => Err(ReadError("name tag payload is not a symbol index".into())),
        },
        // An array is always an application `[head-index, ...children]`.
        C::Array(arr) => {
            if arr.is_empty() {
                return Err(ReadError("application node with no head index".into()));
            }
            let head_index = match &arr[0] {
                C::Integer(i) => i128::from(*i) as usize,
                _ => return Err(ReadError("application head is not a symbol index".into())),
            };
            let symbol = symbol_at(head_index)?;
            let children: Vec<Node> =
                arr[1..].iter().map(|c| cbor_to_node(c, symbols)).collect::<Result<_, _>>()?;
            if symbol == APPLY_SYMBOL {
                // Computed-callee application: the elements are the whole list.
                Ok(Node::List(children))
            } else {
                // [head-symbol, children…] → a Name-headed list form (children may be
                // empty for a nullary form like `(main)`, which stays a List).
                let mut items = Vec::with_capacity(children.len() + 1);
                items.push(Node::Name(symbol.clone()));
                items.extend(children);
                Ok(Node::List(items))
            }
        }
        _ => Err(ReadError("unexpected CBOR value in binary AST".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The binary AST round-trips: decode∘encode is the identity, for every node kind the
    /// ignition uses (ast-encoding.md §"The Encoding Is A Bijection With One Canonical Byte
    /// Form"). A cited behavioral test that fails if the codec stops round-tripping.
    //= spec/contracts/ast-encoding.md#the-encoding-is-a-bijection-with-one-canonical-byte-form
    //= type=test
    //# Decoding a canonical binary encoding MUST yield the abstract syntax tree it was encoded from.
    #[test]
    fn binary_ast_round_trips() {
        let cases = [
            "42",
            "-7",
            "\"hello\"",
            "true",
            "x",
            "(+ 2 3)",
            "(module m (def (main) 42))",
            "(let ((p (record (x 1) (y 2)))) (. p x))",
            "((fn (x) (+ x 1)) 5)",
        ];
        for src in cases {
            let node = read(src).expect("read");
            let bytes = encode(&node);
            let decoded = decode(&bytes).expect("decode");
            assert_eq!(node, decoded, "round-trip failed for {src:?}");
            // Determinism: encoding the decoded tree reproduces the same bytes.
            assert_eq!(bytes, encode(&decoded), "encoding not deterministic for {src:?}");
        }
    }

    /// Equal trees encode identically regardless of construction path.
    //= spec/contracts/ast-encoding.md#the-encoding-is-a-bijection-with-one-canonical-byte-form
    //= type=test
    //# Two abstract syntax trees that are equal MUST have identical binary encodings.
    #[test]
    fn equal_trees_encode_identically() {
        let a = read("(+ (* 1 2) 3)").unwrap();
        let b = read("(+ (* 1 2) 3)").unwrap();
        assert_eq!(encode(&a), encode(&b));
    }

    /// `b"…"` reader sugar reads to the SAME canonical tree as the explicit `(Bytes.of (list …))`,
    /// so the two spellings are one program (options/binary-syntax; the `#"…"`/`a.b` sugar pattern).
    #[test]
    fn byte_string_sugar_reads_to_bytes_of() {
        // Printable ASCII: `b"ABC"` = `(Bytes.of (list 65 66 67))`.
        assert_eq!(read("b\"ABC\"").unwrap(), read("(Bytes.of (list 65 66 67))").unwrap());
        // Empty: `b""` = `(Bytes.of (list))`.
        assert_eq!(read("b\"\"").unwrap(), read("(Bytes.of (list))").unwrap());
        // Hex escapes: `b"\x89PNG"` = the PNG magic prefix `(Bytes.of (list 137 80 78 71))`.
        assert_eq!(read("b\"\\x89PNG\"").unwrap(), read("(Bytes.of (list 137 80 78 71))").unwrap());
        // Special escapes: newline, tab, carriage return, NUL, quote, backslash.
        assert_eq!(
            read("b\"\\n\\t\\r\\0\\\"\\\\\"").unwrap(),
            read("(Bytes.of (list 10 9 13 0 34 92))").unwrap()
        );
    }

    /// The `b` sigil is a byte-string ONLY directly before a `"`. A bare `b`, and a name that merely
    /// starts with `b` (like the `bin` binary form's head, or `bytes`), stay ordinary names — so the
    /// literal does not collide with the `(bin …)` / `(bytes …)` grammar (16-binary-matching.sexp).
    #[test]
    fn byte_string_sigil_does_not_capture_names() {
        assert_eq!(read("b").unwrap(), Node::Name("b".into()));
        assert_eq!(read("bin").unwrap(), Node::Name("bin".into()));
        assert_eq!(read("bytes").unwrap(), Node::Name("bytes".into()));
        // `(bin (bytes b"\x89PNG"))` — a byte-string literal spliced into a bin form parses as a
        // structured application whose leaf is the Bytes value, i.e. b"…" composes with bin.
        let composed = read("(bin (bytes b\"\\x89PNG\"))").unwrap();
        let explicit = read("(bin (bytes (Bytes.of (list 137 80 78 71))))").unwrap();
        assert_eq!(composed, explicit);
    }

    /// A malformed `b"…"` is a read error, never a silently-wrong value: an unterminated literal, a
    /// short `\x`, and an unknown escape each fail.
    #[test]
    fn byte_string_malformed_is_error() {
        assert!(read("b\"unterminated").is_err());
        assert!(read("b\"\\x8\"").is_err()); // one hex digit
        assert!(read("b\"\\q\"").is_err()); // unknown escape
    }
}
