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

use crate::ast::{Arenas, Builder, Leaf, Struct, StructId};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug)]
pub struct ReadError(pub String);

/// Parse a single s-expression from `text` into its own `Arenas` (rooted at the parsed form).
pub fn read(text: &str) -> Result<Arenas, ReadError> {
    let mut b = Builder::new();
    let mut p = Reader::new(text, &mut b);
    p.skip_ws();
    let root = p.read_node()?;
    p.skip_ws();
    if p.peek().is_some() {
        return Err(ReadError(format!("trailing input at byte {}", p.pos)));
    }
    Ok(b.finish(root))
}

/// Parse every top-level s-expression from `text`, each as an element of a synthetic `(do …)` root
/// — convenient for reading a corpus file, whose top level is a sequence of `(case …)` forms.
pub fn read_all(text: &str) -> Result<Arenas, ReadError> {
    let mut b = Builder::new();
    let mut roots = Vec::new();
    {
        let mut p = Reader::new(text, &mut b);
        loop {
            p.skip_ws();
            if p.peek().is_none() {
                break;
            }
            roots.push(p.read_node()?);
        }
    }
    let do_head = b.name("do");
    let mut items = Vec::with_capacity(roots.len() + 1);
    items.push(do_head);
    items.extend(roots);
    let root = b.list(items);
    Ok(b.finish(root))
}

// ============================================================================
// Printer: Arenas -> s-expression text. The direct code-as-data rendering, and the dual of the
// reader above (it re-reads to a structurally-equal arena).
// ============================================================================

/// Render `arenas` as an s-expression string.
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
        // A name is written verbatim. (The s-expr surface has no reserved words — `let`, `+`, `|`
        // are all ordinary atoms — so no escaping is needed here, unlike the ML surface.)
        Leaf::Name(n) => out.push_str(n),
    }
}

struct Reader<'a, 'b> {
    src: &'a [u8],
    pos: usize,
    b: &'b mut Builder,
}

impl<'a, 'b> Reader<'a, 'b> {
    fn new(text: &'a str, b: &'b mut Builder) -> Reader<'a, 'b> {
        Reader {
            src: text.as_bytes(),
            pos: 0,
            b,
        }
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
            // `` ` `` / `,` / `,@` sigils, matching the corpus quasiquote display.
            Some(b'`') => {
                self.bump();
                let inner = self.read_node()?;
                let head = self.b.name("quasiquote");
                Ok(self.b.list(vec![head, inner]))
            }
            Some(b',') => {
                self.bump();
                if self.peek() == Some(b'@') {
                    self.bump();
                    let inner = self.read_node()?;
                    let head = self.b.name("unquote-splicing");
                    Ok(self.b.list(vec![head, inner]))
                } else {
                    let inner = self.read_node()?;
                    let head = self.b.name("unquote");
                    Ok(self.b.list(vec![head, inner]))
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
                    let dot = self.b.name(".");
                    let key = self.b.name(seg);
                    node = self.b.list(vec![dot, node, key]);
                }
                _ => break,
            }
        }
        Ok(node)
    }

    fn read_list(&mut self) -> Result<StructId, ReadError> {
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
        Ok(self.b.list(items))
    }

    fn read_string(&mut self) -> Result<StructId, ReadError> {
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
        // NFC-normalize string contents (the value form normalizes text).
        let s: String = s.chars().nfc().collect();
        Ok(self.b.atom_leaf(Leaf::Str(s)))
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
        Ok(self.classify_token(tok))
    }

    /// Classify a whitespace-delimited token into a leaf occurrence. A dotted token `a.b.c` is
    /// display sugar for nested member access `(. (. a b) c)`; otherwise the shared
    /// [`crate::literal::classify_word`] decides Int / Float / Bool / Name — the SAME layer the ML
    /// surface uses, so literal values are byte-identical across surfaces.
    fn classify_token(&mut self, tok: &str) -> StructId {
        // A segmented identifier (`Sign.Neg`, `a.b.c`) desugars to nested member access. This is
        // checked before `classify_word` because a numeric literal (`3.5`) is not a dotted name
        // (its segments start with digits), so the two never conflict.
        if is_dotted_name(tok) {
            let mut segs = tok.split('.');
            let mut node = self.b.name(segs.next().unwrap());
            for seg in segs {
                let dot = self.b.name(".");
                let seg_id = self.b.name(seg);
                node = self.b.list(vec![dot, node, seg_id]);
            }
            return node;
        }
        // Classify the word. A NUMBER/BOOL is a non-Name leaf (interned by value); a NAME is interned
        // by its `&str` slice via `leaf_name` — allocating an owned `String` only on a dedup MISS, not
        // for every occurrence (`classify_word` would `to_string()` the name eagerly and discard it on
        // a hit). `classify_word_nonname` returns `Some` only for the number/bool kinds, so a bare name
        // never allocates on the common repeated-identifier path.
        match crate::literal::classify_word_nonname(tok) {
            Some(leaf) => self.b.atom_leaf(leaf),
            None => {
                let id = self.b.leaf_name(tok);
                self.b.atom(id)
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
}
