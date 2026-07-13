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

/// Classify the WORD of a char literal (the text after `#\`) into a [`Leaf::Char`] (a valid Unicode
/// scalar) or a [`Leaf::BadChar`] MARKER (a surrogate / out-of-range code point / unknown name — the
/// compiler turns it into CDZ0002). Three spellings, shared by both surfaces so a char literal reads
/// identically:
/// - a single scalar: `a`, `é` (exactly one `char`);
/// - a named control char: `space`, `newline`, `tab`, `return`, `null` (the common Scheme names);
/// - a hex code point: `u+HHHH` (case-insensitive `u+`, 1+ hex digits) — a value outside the scalar
///   range (past `U+10FFFF` or a surrogate `U+D800..=U+DFFF`) is a `BadChar`.
///
/// The `word` never contains a delimiter (the reader stops at whitespace/paren/`;`); the raw-delimiter
/// spellings (`#\(`, `#\ `) are handled by the reader before this and never reach here. A char value is
/// NOT NFC-normalized — a char is one scalar, and normalization is a property of scalar *sequences*.
pub fn char_leaf(word: &str) -> Leaf {
    // A single scalar — the common case (`#\a`, `#\é`).
    let mut chars = word.chars();
    if let Some(c) = chars.next()
        && chars.next().is_none()
    {
        return Leaf::Char(c);
    }
    // A `u+HHHH` code-point spelling (case-insensitive prefix).
    if let Some(hex) = word.strip_prefix("u+").or_else(|| word.strip_prefix("U+"))
        && !hex.is_empty()
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
        && let Ok(cp) = u32::from_str_radix(hex, 16)
    {
        return match char::from_u32(cp) {
            Some(c) => Leaf::Char(c),
            None => Leaf::BadChar(word.to_string()), // surrogate or > U+10FFFF
        };
    }
    // A named control char.
    match word {
        "space" => Leaf::Char(' '),
        "newline" => Leaf::Char('\n'),
        "tab" => Leaf::Char('\t'),
        "return" => Leaf::Char('\r'),
        "null" => Leaf::Char('\0'),
        // Anything else — an unknown multi-char name — is malformed.
        _ => Leaf::BadChar(word.to_string()),
    }
}

/// Render a char scalar as a `#\…` literal that re-reads (via [`char_leaf`]) to the SAME scalar — the
/// round-trip law. A common control char uses its NAME (`space`/`newline`/`tab`/`return`/`null`); any
/// other control or non-printable char uses the `u+HHHH` code-point form; everything else is written
/// as the bare scalar (`#\a`, `#\é`, `#\(` — a raw delimiter is handled by the reader's delimiter path).
pub fn render_char(c: char) -> String {
    match c {
        ' ' => "#\\space".to_string(),
        '\n' => "#\\newline".to_string(),
        '\t' => "#\\tab".to_string(),
        '\r' => "#\\return".to_string(),
        '\0' => "#\\null".to_string(),
        // Any other control / non-printable char: the unambiguous hex code-point form.
        c if c.is_control() => format!("#\\u+{:04X}", c as u32),
        c => format!("#\\{c}"),
    }
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

/// Unescape a string literal's INNER content (between the quotes) and NFC-normalize it — the shared
/// escape table both surfaces use, so a string leaf is identical however it was written. The escape set
/// is CLOSED (`\n \t \r \\ \"`); an unrecognized `\x` is a lexical defect — `Err(x)` names the first
/// offending escape char (the caller turns it into a `Leaf::BadEscape` marker the compiler rejects
/// CDZ0001). `Ok(s)` is the normalized text when every escape is valid.
pub fn unescape_string(inner: &str) -> Result<String, char> {
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
                // An UNRECOGNIZED escape — the set is closed. Report the offending char (first one wins).
                Some(other) => return Err(other),
                None => {} // trailing backslash: drop
            }
        } else {
            out.push(c);
        }
    }
    Ok(out.nfc().collect())
}

/// Unescape a `"…"` string TOKEN (quotes included, as the lexer spans it) into its `Leaf` — a
/// [`Leaf::Str`] on a valid escape set, or a [`Leaf::BadEscape`] MARKER carrying the offending char when
/// an escape is not in the closed set (`\q`). Both surfaces produce the SAME leaf so the round-trip and
/// the s-expr↔ML agreement hold. Returns an empty `Str` if the token is not quote-delimited.
pub fn unescape_string_token(token: &str) -> Leaf {
    let inner = token
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or("");
    match unescape_string(inner) {
        Ok(s) => Leaf::Str(s),
        Err(c) => Leaf::BadEscape(c),
    }
}

/// Unescape a symbol-literal TOKEN into a [`Leaf::Sym`] — the interned-name value form. Two surface
/// spellings both reach here: the QUOTED `#"…"` (the `#` + quotes included, as the lexer spans it) and
/// the UNQUOTED `#name` sugar (a `#` glued to a bare identifier — the quotes are only needed when the
/// content is not an identifier). The quoted form reuses the STRING escape set and NFC normalization
/// ([`unescape_string`]), so its content is lexed exactly as a string body; only the leaf kind and the
/// `#"` prefix differ. An unrecognized escape keeps the raw char (a symbol names arbitrary content —
/// the closed-escape-set contract is a string concern), so this never yields a `BadEscape`. The
/// unquoted form's body is an identifier (no escapes), so it is just NFC-normalized. Returns an empty
/// `Sym` if the token is neither `#"…"`- nor `#name`-shaped.
pub fn unescape_sym_token(token: &str) -> Leaf {
    // `#name` (no quote after the `#`) is the unquoted sugar — the body is a bare identifier, so there
    // are no escapes to process; NFC-normalize it to match the quoted form's normalized-content identity.
    if let Some(body) = token.strip_prefix('#')
        && !body.starts_with('"')
    {
        return Leaf::Sym(body.nfc().collect());
    }
    let inner = token
        .strip_prefix("#\"")
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or("");
    // Reuse the string unescape; on an unrecognized escape keep the raw text (a symbol is content-typed,
    // not subject to the closed-escape diagnostic) by falling back to the inner NFC-normalized text.
    let content = match unescape_string(inner) {
        Ok(s) => s,
        Err(_) => inner.nfc().collect(),
    };
    Leaf::Sym(content)
}

/// Unescape a byte-string TOKEN (`b"…"`, the `b` + quotes included, as the ml lexer spans it) into
/// the raw bytes it denotes. The INVERSE of [`escape_bytes`] (the render side) and identical to the
/// sexpr `read_byte_string` reader, so `b"…"` produces byte-identical `Leaf::Bytes` on both surfaces:
/// `\n \t \r \\ \"` are the named byte escapes, `\xNN` is a two-hex-digit byte, any other `\c` keeps
/// `c` verbatim, and a raw byte stands for itself. Returns `vec![]` if the token is not `b"…"`-shaped.
pub fn unescape_byte_string_token(token: &str) -> Vec<u8> {
    let inner = token
        .strip_prefix("b\"")
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or("");
    let bytes = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'\\' => out.push(b'\\'),
                b'"' => out.push(b'"'),
                // `\xNN` — exactly two hex digits, the byte they name; otherwise keep `x` verbatim.
                b'x' if i + 2 < bytes.len() => {
                    match (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                        (Some(h), Some(l)) => {
                            out.push((h << 4) | l);
                            i += 2;
                        }
                        _ => out.push(b'x'),
                    }
                }
                other => out.push(other),
            }
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    out
}

/// A single hex digit `0-9a-fA-F` to its nibble value.
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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
            assert_eq!(unescape_string(&escape_string(s)).as_deref(), Ok(s));
        }
    }
}
