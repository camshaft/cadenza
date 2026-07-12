//! Literal-value parsing: the single `token-text -> Leaf` layer shared by both surfaces.
//!
//! Whichever surface a program is written in, a numeric or word token is turned into a leaf by the
//! SAME functions here, so `42`, `0x2A`, `1.5e10`, `true`, `foo` produce byte-identical leaves — the
//! integer/float representation is defined in exactly one place.
//!
//! The classification is strict: a malformed numeric literal (a trailing/doubled `_` separator, a
//! bad radix digit) is NOT silently repaired — it fails the numeric parse and falls through to a
//! `Name`, which downstream rejects, rather than being read as a different value. Digit-separator
//! (`_`) positions are not preserved; the integer's base (dec/hex/bin) IS, so the printed form
//! re-reads to the same leaf.

use crate::ast::{Decimal, Leaf, Radix};
use num_bigint::BigInt;
use std::str::FromStr;
use unicode_normalization::UnicodeNormalization;

/// Classify a bare word/number token into a leaf value. `true`/`false` are booleans; a well-formed
/// integer or float is that literal; anything else (including a malformed number) is a `Name`.
///
/// Keywords are NOT handled here — that is the parser's job (`token::keyword`); a word like `let`
/// classifies as `Leaf::Name("let")` and only becomes a keyword in grammatical position.
pub fn classify_word(text: &str) -> Leaf {
    classify_word_nonname(text).unwrap_or_else(|| Leaf::Name(text.to_string()))
}

/// Classify a word into a NON-NAME leaf — `Bool` / `Int` / `Float` — or `None` if it is a plain
/// identifier (a `Name`). Split out of [`classify_word`] so a caller that interns names by their
/// `&str` slice (`ast::Builder::leaf_name`, the hot parse path) can decide "is this a number/bool?"
/// WITHOUT allocating a `Leaf::Name(String)` it would discard on a dedup hit. `classify_word` layers
/// the owning `Name` fallback back on for callers that want the full `Leaf`.
pub fn classify_word_nonname(text: &str) -> Option<Leaf> {
    match text {
        "true" => return Some(Leaf::Bool(true)),
        "false" => return Some(Leaf::Bool(false)),
        _ => {}
    }
    // FAST PATH: a number literal ALWAYS begins with `[0-9+-]` — `parse_int`/`parse_float` both strip a
    // leading `+`/`-` and then require the body to start with an ASCII digit (`0x`/`0b` start with `0`).
    // So a token whose first byte is anything else (a letter, `_`, `.`, a sigil) cannot be a number, and
    // the two parse attempts below would just scan it and fail. Identifiers/keywords are the vast
    // majority of tokens, so this guard skips ~all of the per-name number-parsing (parse_int + parse_float
    // were ~9% of front-end parse time). A token that IS a number still takes the full path unchanged.
    match text.as_bytes().first() {
        Some(b'0'..=b'9' | b'+' | b'-') => {}
        _ => return None,
    }
    if let Some((value, radix)) = parse_int(text) {
        return Some(Leaf::Int { value, radix });
    }
    if let Some(d) = parse_float(text) {
        return Some(Leaf::Float(d));
    }
    None
}

/// Parse a decimal / `0x…` / `0b…` integer token into its exact value and the base its text used,
/// or `None` if it is not a well-formed integer literal. No magnitude ceiling.
pub fn parse_int(tok: &str) -> Option<(BigInt, Radix)> {
    let (neg, body) = match tok.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, tok.strip_prefix('+').unwrap_or(tok)),
    };
    // Radix-prefixed literal.
    if let Some(radix_body) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0b")) {
        let is_hex = body.as_bytes().get(1) == Some(&b'x');
        let well_formed = !radix_body.is_empty()
            && radix_body
                .chars()
                .next()
                .is_some_and(|c| is_radix_digit(c, is_hex))
            && radix_body
                .chars()
                .all(|c| is_radix_digit(c, is_hex) || c == '_')
            && separators_between_digits(radix_body, |c| is_radix_digit(c, is_hex));
        if !well_formed {
            return None;
        }
        let digits: String = radix_body.chars().filter(|&c| c != '_').collect();
        let radix = if is_hex { 16 } else { 2 };
        let mag = BigInt::parse_bytes(digits.as_bytes(), radix)?;
        let value = if neg { -mag } else { mag };
        return Some((value, if is_hex { Radix::Hex } else { Radix::Bin }));
    }
    // Plain decimal: must start with a digit, only digits + between-digits `_`.
    let starts_digit = body.chars().next().is_some_and(|c| c.is_ascii_digit());
    let only_digits_seps = body.chars().all(|c| c.is_ascii_digit() || c == '_');
    if !(starts_digit
        && only_digits_seps
        && separators_between_digits(body, |c| c.is_ascii_digit()))
    {
        return None;
    }
    let digits: String = body.chars().filter(|&c| c != '_').collect();
    let mag = BigInt::from_str(&digits).ok()?;
    Some((if neg { -mag } else { mag }, Radix::Dec))
}

/// Parse a float token into an exact `Decimal`, or `None`. A float must start with a digit and
/// contain a `.` or exponent; `_` separators must sit between digits. Captures the value EXACTLY
/// (no `f64`): `significand * 10^exponent`.
pub fn parse_float(tok: &str) -> Option<Decimal> {
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
        Some(i) => (clean[..i].to_string(), Some(clean[i + 1..].to_string())),
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
        let e = e.strip_prefix('+').unwrap_or(&e);
        let e_val = i64::from_str(e).ok()?;
        exponent = exponent.checked_add(e_val)?;
    }
    Some(normalize_decimal(neg, significand, exponent))
}

/// Put a decimal in canonical form: one representation per value, so render∘parse is identity.
/// Trailing zeros of the significand move into the exponent (`150 * 10^-1` == `15 * 10^0`), and a
/// zero significand canonicalizes to exponent 0 (preserving the sign, so `-0.0` stays negative).
fn normalize_decimal(negative: bool, mut significand: BigInt, mut exponent: i64) -> Decimal {
    use num_bigint::Sign;
    if significand.sign() == Sign::NoSign {
        return Decimal {
            negative,
            significand,
            exponent: 0,
        };
    }
    let ten = BigInt::from(10);
    while (&significand % &ten).sign() == Sign::NoSign {
        significand /= &ten;
        exponent += 1;
    }
    Decimal {
        negative,
        significand,
        exponent,
    }
}

/// Unescape a string literal's INNER content (between the quotes) and NFC-normalize it — the
/// shared escape table both surfaces use, so a string leaf is identical however it was written. The
/// escape set is `\n \t \r \\ \"`; any other `\x` passes the following char through verbatim.
pub fn unescape_string(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => out.push(other),
                None => {} // trailing backslash: drop
            }
        } else {
            out.push(c);
        }
    }
    out.nfc().collect()
}

/// Unescape a `"…"` string TOKEN (quotes included, as the lexer spans it). Strips the surrounding
/// quotes then delegates to [`unescape_string`]. Returns `""` if the token is not quote-delimited.
pub fn unescape_string_token(token: &str) -> String {
    let inner = token
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or("");
    unescape_string(inner)
}

/// Decode a backtick-name TOKEN (`` `…` ``, backticks included) to the escaped name it denotes.
/// Inside backticks, `\`` and `\\` are the only escapes; anything else passes through.
pub fn unescape_backtick_name(token: &str) -> String {
    let inner = token
        .strip_prefix('`')
        .and_then(|s| s.strip_suffix('`'))
        .unwrap_or("");
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(e) = chars.next() {
                out.push(e);
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ============================================================================
// Rendering — the duals of the parsers above, shared by both surface printers so a leaf renders to
// text that re-reads to the same leaf (round-trip).
// ============================================================================

/// Render an integer in the base its text used, so it re-reads to the same `Int` leaf. Hex/bin get
/// their `0x`/`0b` prefix (with the sign, if any, before the prefix, as the reader accepts).
pub fn render_int(value: &BigInt, radix: Radix) -> String {
    use num_bigint::Sign;
    let (sign, mag) = value.to_bytes_be();
    let neg = matches!(sign, Sign::Minus);
    let digits = match radix {
        Radix::Dec => BigInt::from_bytes_be(num_bigint::Sign::Plus, &mag).to_str_radix(10),
        Radix::Hex => format!(
            "0x{}",
            BigInt::from_bytes_be(num_bigint::Sign::Plus, &mag).to_str_radix(16)
        ),
        Radix::Bin => format!(
            "0b{}",
            BigInt::from_bytes_be(num_bigint::Sign::Plus, &mag).to_str_radix(2)
        ),
    };
    if neg { format!("-{digits}") } else { digits }
}

/// Render an exact `Decimal` as the shortest text that re-parses to the same value. Always contains
/// a `.` or exponent so it re-lexes as a Float, never an Int. `nan`/`inf` are not `Decimal`s (they
/// are names), so this only ever renders a finite value; `-0.0` prints with its sign.
pub fn render_decimal(d: &Decimal) -> String {
    let sign = if d.negative { "-" } else { "" };
    let digits = d.significand.to_str_radix(10); // non-negative magnitude
    // Place the decimal point per the base-10 exponent: value = digits * 10^exponent.
    let text = if d.exponent == 0 {
        // integer-valued: force a fractional part so it lexes as a float
        format!("{digits}.0")
    } else if d.exponent > 0 {
        // shift left: append zeros, then `.0`
        let zeros = "0".repeat(d.exponent as usize);
        format!("{digits}{zeros}.0")
    } else {
        // exponent < 0: place a decimal point `-exponent` digits from the right
        let frac = (-d.exponent) as usize;
        if digits.len() > frac {
            let point = digits.len() - frac;
            format!("{}.{}", &digits[..point], &digits[point..])
        } else {
            let pad = "0".repeat(frac - digits.len());
            format!("0.{pad}{digits}")
        }
    };
    format!("{sign}{text}")
}

/// Escape a string's contents for a `"…"` literal (the dual of [`unescape_string`]).
pub fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

/// Escape a byte sequence's contents for a `b"…"` literal — the byte-string form
/// (`options/binary-syntax`). A printable ASCII byte (`0x20..=0x7e`) stands for itself; `\n \r \t \\
/// \"` use their named escapes; every other byte is a two-lowercase-hex `\xNN`. So `[1,2,3]` →
/// `\x01\x02\x03` and `[65,10,66]` → `A\nB`. The dual of the `b"…"` reader's unescape; a byte
/// sequence's canonical observable form.
pub fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    for &b in bytes {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\r' => out.push_str("\\r"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            // Printable ASCII stands for itself; every other byte is a `\xNN` (two lowercase hex).
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out
}

/// True iff every `_` in `body` sits BETWEEN two `is_digit` chars — no leading, trailing, or
/// doubled separator. The between-digits rule, applied in both directions.
pub fn separators_between_digits(body: &str, is_digit: impl Fn(char) -> bool) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_with_base() {
        assert_eq!(parse_int("42"), Some((BigInt::from(42), Radix::Dec)));
        assert_eq!(parse_int("0x2A"), Some((BigInt::from(42), Radix::Hex)));
        assert_eq!(parse_int("0b101010"), Some((BigInt::from(42), Radix::Bin)));
        assert_eq!(parse_int("-0x10"), Some((BigInt::from(-16), Radix::Hex)));
        assert_eq!(
            parse_int("1_000_000"),
            Some((BigInt::from(1_000_000), Radix::Dec))
        );
    }

    #[test]
    fn malformed_int_is_none() {
        assert_eq!(parse_int("1_"), None); // trailing separator
        assert_eq!(parse_int("1__0"), None); // doubled
        assert_eq!(parse_int("0x"), None); // no digits
        assert_eq!(parse_int("_1"), None); // leading underscore is not an int
    }

    #[test]
    fn floats_exact() {
        assert_eq!(
            parse_float("1.5"),
            Some(Decimal {
                negative: false,
                significand: BigInt::from(15),
                exponent: -1
            })
        );
        assert_eq!(
            parse_float("1.5e10"),
            Some(Decimal {
                negative: false,
                significand: BigInt::from(15),
                exponent: 9
            })
        );
        assert_eq!(
            parse_float("-0.25"),
            Some(Decimal {
                negative: true,
                significand: BigInt::from(25),
                exponent: -2
            })
        );
    }

    #[test]
    fn classify_word_dispatch() {
        assert_eq!(classify_word("true"), Leaf::Bool(true));
        assert_eq!(
            classify_word("42"),
            Leaf::Int {
                value: BigInt::from(42),
                radix: Radix::Dec
            }
        );
        assert!(matches!(classify_word("1.5"), Leaf::Float(_)));
        assert_eq!(classify_word("foo"), Leaf::Name("foo".to_string()));
        // A malformed number stays a Name (rejected downstream), never silently repaired.
        assert_eq!(classify_word("1_"), Leaf::Name("1_".to_string()));
        // Keywords are ordinary names here — the parser decides keyword-ness.
        assert_eq!(classify_word("let"), Leaf::Name("let".to_string()));
    }

    #[test]
    fn int_render_reparses() {
        for (v, r) in [
            (42i64, Radix::Dec),
            (42, Radix::Hex),
            (42, Radix::Bin),
            (-16, Radix::Hex),
            (0, Radix::Dec),
            (255, Radix::Hex),
            (-1, Radix::Dec),
        ] {
            let value = BigInt::from(v);
            let text = render_int(&value, r);
            assert_eq!(
                parse_int(&text),
                Some((value, r)),
                "render {v} base {r:?} -> {text}"
            );
        }
    }

    #[test]
    fn float_render_reparses() {
        for d in [
            Decimal {
                negative: false,
                significand: BigInt::from(15),
                exponent: -1,
            }, // 1.5
            Decimal {
                negative: false,
                significand: BigInt::from(15),
                exponent: 9,
            }, // 15e9
            Decimal {
                negative: true,
                significand: BigInt::from(25),
                exponent: -2,
            }, // -0.25
            Decimal {
                negative: false,
                significand: BigInt::from(5),
                exponent: 0,
            }, // 5.0
            Decimal {
                negative: true,
                significand: BigInt::from(0u32),
                exponent: 0,
            }, // -0.0
            Decimal {
                negative: false,
                significand: BigInt::from(1),
                exponent: -10,
            }, // 0.0000000001
        ] {
            let text = render_decimal(&d);
            assert_eq!(
                parse_float(&text),
                Some(d.clone()),
                "render {d:?} -> {text}"
            );
        }
    }

    #[test]
    fn string_escape_reparses() {
        for s in [
            "hello",
            "a\nb",
            "tab\there",
            "quote\"inside",
            "back\\slash",
            "",
        ] {
            assert_eq!(unescape_string(&escape_string(s)), s);
        }
    }
}
