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
        Reader { src: text.as_bytes(), pos: 0, b }
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

    fn read_node(&mut self) -> Result<StructId, ReadError> {
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
        self.b.atom_leaf(crate::literal::classify_word(tok))
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
        !s.is_empty() && s.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
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
            assert_eq!(print(&b), printed, "print∘read stable for {src:?} (printed {printed:?})");
        }
    }

    #[test]
    fn bigint_no_ceiling() {
        let a = read("123456789012345678901234567890").unwrap();
        let Struct::Atom(l) = a.get(a.root) else { panic!() };
        match a.leaf(*l) {
            Leaf::Int { value, radix } => {
                assert_eq!(value, &BigInt::from_str("123456789012345678901234567890").unwrap());
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
            let Struct::Atom(l) = a.get(a.root) else { panic!() };
            assert_eq!(
                a.leaf(*l),
                &Leaf::Int { value: BigInt::from(val), radix },
                "src {src}"
            );
        }
    }

    #[test]
    fn exact_float() {
        let a = read("1.5").unwrap();
        let Struct::Atom(l) = a.get(a.root) else { panic!() };
        assert_eq!(
            a.leaf(*l),
            &Leaf::Float(Decimal { negative: false, significand: BigInt::from(15), exponent: -1 })
        );
    }

    #[test]
    fn exponent_float() {
        let a = read("1.5e10").unwrap();
        let Struct::Atom(l) = a.get(a.root) else { panic!() };
        // 15 * 10^(10-1) = 15e9
        assert_eq!(
            a.leaf(*l),
            &Leaf::Float(Decimal { negative: false, significand: BigInt::from(15), exponent: 9 })
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
    fn digit_separators_ok() {
        let a = read("1_000_000").unwrap();
        let Struct::Atom(l) = a.get(a.root) else { panic!() };
        assert_eq!(a.leaf(*l), &Leaf::Int { value: BigInt::from(1_000_000), radix: Radix::Dec });
    }
}
