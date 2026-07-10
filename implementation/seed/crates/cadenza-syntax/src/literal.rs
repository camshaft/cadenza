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

/// Classify a bare word/number token into a leaf value. `true`/`false` are booleans; a well-formed
/// integer or float is that literal; anything else (including a malformed number) is a `Name`.
///
/// Keywords are NOT handled here — that is the parser's job (`token::keyword`); a word like `let`
/// classifies as `Leaf::Name("let")` and only becomes a keyword in grammatical position.
pub fn classify_word(text: &str) -> Leaf {
    match text {
        "true" => return Leaf::Bool(true),
        "false" => return Leaf::Bool(false),
        _ => {}
    }
    if let Some((value, radix)) = parse_int(text) {
        return Leaf::Int { value, radix };
    }
    if let Some(d) = parse_float(text) {
        return Leaf::Float(d);
    }
    Leaf::Name(text.to_string())
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
            && radix_body.chars().next().is_some_and(|c| is_radix_digit(c, is_hex))
            && radix_body.chars().all(|c| is_radix_digit(c, is_hex) || c == '_')
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
    if !(starts_digit && only_digits_seps && separators_between_digits(body, |c| c.is_ascii_digit()))
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
    Some(Decimal { negative: neg, significand, exponent })
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
        assert_eq!(parse_int("1_000_000"), Some((BigInt::from(1_000_000), Radix::Dec)));
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
            Some(Decimal { negative: false, significand: BigInt::from(15), exponent: -1 })
        );
        assert_eq!(
            parse_float("1.5e10"),
            Some(Decimal { negative: false, significand: BigInt::from(15), exponent: 9 })
        );
        assert_eq!(
            parse_float("-0.25"),
            Some(Decimal { negative: true, significand: BigInt::from(25), exponent: -2 })
        );
    }

    #[test]
    fn classify_word_dispatch() {
        assert_eq!(classify_word("true"), Leaf::Bool(true));
        assert_eq!(classify_word("42"), Leaf::Int { value: BigInt::from(42), radix: Radix::Dec });
        assert!(matches!(classify_word("1.5"), Leaf::Float(_)));
        assert_eq!(classify_word("foo"), Leaf::Name("foo".to_string()));
        // A malformed number stays a Name (rejected downstream), never silently repaired.
        assert_eq!(classify_word("1_"), Leaf::Name("1_".to_string()));
        // Keywords are ordinary names here — the parser decides keyword-ness.
        assert_eq!(classify_word("let"), Leaf::Name("let".to_string()));
    }
}
