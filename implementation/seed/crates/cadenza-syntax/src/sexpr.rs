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

use crate::ast::{Arenas, Builder, Decimal, Leaf, StructId};
use num_bigint::BigInt;
use std::str::FromStr;
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
    /// display sugar for nested member access `(. (. a b) c)`.
    fn classify_token(&mut self, tok: &str) -> StructId {
        match tok {
            "true" => return self.b.atom_leaf(Leaf::Bool(true)),
            "false" => return self.b.atom_leaf(Leaf::Bool(false)),
            _ => {}
        }
        if let Some(i) = parse_int_literal(tok) {
            return self.b.atom_leaf(Leaf::Int(i));
        }
        if let Some(d) = parse_float_literal(tok) {
            return self.b.atom_leaf(Leaf::Float(d));
        }
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
        self.b.name(tok)
    }
}

// ============================================================================
// Numeric classification — the strict rule (ported and generalized to BigInt/Decimal).
// ============================================================================

/// True iff every `_` in `body` sits BETWEEN two `is_digit` chars — no leading, trailing, or
/// doubled separator. Matches the between-digits rule in both directions.
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

fn is_radix_digit(c: char, is_hex: bool) -> bool {
    if is_hex {
        c.is_ascii_hexdigit()
    } else {
        c == '0' || c == '1'
    }
}

/// Parse a decimal / `0x…` / `0b…` integer token into an arbitrary-precision `BigInt`, or `None`
/// if the token is not a well-formed integer literal (leaving it to be read as a name/float).
/// There is NO magnitude ceiling — a value of any size parses.
fn parse_int_literal(tok: &str) -> Option<BigInt> {
    let (neg, body) = match tok.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, tok.strip_prefix('+').unwrap_or(tok)),
    };
    // Radix-prefixed literal.
    if let Some(radix_body) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0b")) {
        let is_hex = body.as_bytes().get(1) == Some(&b'x');
        let well_formed = !radix_body.is_empty()
            && radix_body.chars().next().is_some_and(|c| is_radix_digit(c, is_hex))
            && radix_body.chars().all(|c| is_radix_digit(c, is_hex) || c == '_')
            && separators_between_digits(radix_body, |c| is_radix_digit(c, is_hex));
        if !well_formed {
            return None;
        }
        let digits: String = radix_body.chars().filter(|&c| c != '_').collect();
        let radix = if is_hex { 16 } else { 2 };
        let mag = BigInt::parse_bytes(digits.as_bytes(), radix)?;
        return Some(if neg { -mag } else { mag });
    }
    // Plain decimal: must start with a digit, only digits + between-digits `_`.
    let starts_digit = body.chars().next().is_some_and(|c| c.is_ascii_digit());
    let only_digits_seps = body.chars().all(|c| c.is_ascii_digit() || c == '_');
    if !(starts_digit && only_digits_seps && separators_between_digits(body, |c| c.is_ascii_digit()))
    {
        return None;
    }
    let digits: String = body.chars().filter(|&c| c != '_').collect();
    let mag = BigInt::from_str(&digits).ok()?;
    Some(if neg { -mag } else { mag })
}

/// Parse a float token into an exact `Decimal`, or `None`. A float must start with a digit and
/// contain a `.` or exponent; `_` separators must sit between digits. Captures the value EXACTLY
/// (no `f64`): `significand * 10^exponent`.
fn parse_float_literal(tok: &str) -> Option<Decimal> {
    let (neg, body) = match tok.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, tok.strip_prefix('+').unwrap_or(tok)),
    };
    if !body.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let has_point_or_exp = body.contains('.') || body.contains('e') || body.contains('E');
    if !has_point_or_exp {
        return None;
    }
    if !body
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-' | '_'))
    {
        return None;
    }
    if !separators_between_digits(body, |c| c.is_ascii_digit()) {
        return None;
    }
    // Split mantissa and exponent.
    let clean: String = body.chars().filter(|&c| c != '_').collect();
    let (mantissa, exp_part) = match clean.find(['e', 'E']) {
        Some(i) => (clean[..i].to_string(), Some(&clean[i + 1..])),
        None => (clean.clone(), None),
    };
    // The mantissa's fractional digits become negative exponent.
    let (int_digits, frac_digits) = match mantissa.find('.') {
        Some(i) => (mantissa[..i].to_string(), mantissa[i + 1..].to_string()),
        None => (mantissa.clone(), String::new()),
    };
    // A trailing `.` with no fraction (`1.`) or a stray extra `.` -> not a well-formed float here.
    if int_digits.contains('.') || frac_digits.contains('.') {
        return None;
    }
    let mut digits = String::new();
    digits.push_str(&int_digits);
    digits.push_str(&frac_digits);
    if digits.is_empty() {
        return None;
    }
    let significand = BigInt::from_str(&digits).ok()?;
    let mut exponent: i64 = -(frac_digits.len() as i64);
    if let Some(e) = exp_part {
        let e = e.strip_prefix('+').unwrap_or(e);
        let e_val = i64::from_str(e).ok()?;
        exponent = exponent.checked_add(e_val)?;
    }
    Some(Decimal { negative: neg, significand, exponent })
}

/// True for an `a.b`(`.c…`) segmented identifier: at least one dot, every segment non-empty and
/// starting with a letter or `_` (so a float like `3.5` — parsed above — never reaches here).
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
    use crate::ast::{Leaf, Struct};

    #[test]
    fn reads_a_form() {
        let a = read("(+ 1 2)").unwrap();
        assert_eq!(a.head_name(a.root), Some("+"));
    }

    #[test]
    fn bigint_no_ceiling() {
        let a = read("123456789012345678901234567890").unwrap();
        let Struct::Atom(l) = a.get(a.root) else { panic!() };
        match a.leaf(*l) {
            Leaf::Int(n) => {
                assert_eq!(n, &BigInt::from_str("123456789012345678901234567890").unwrap())
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn radix_literals() {
        for (src, val) in [("0x2A", 42), ("0b101010", 42), ("-0x10", -16)] {
            let a = read(src).unwrap();
            let Struct::Atom(l) = a.get(a.root) else { panic!() };
            assert_eq!(a.leaf(*l), &Leaf::Int(BigInt::from(val)), "src {src}");
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
        assert_eq!(a.leaf(*l), &Leaf::Int(BigInt::from(1_000_000)));
    }
}
