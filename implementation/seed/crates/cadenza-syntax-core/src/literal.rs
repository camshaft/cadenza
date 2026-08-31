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

use crate::ast::{Decimal, IntValue, Leaf, Radix, SuffixBody, SuffixKind};
use num_bigint::BigInt;
use std::str::FromStr;
use unicode_normalization::UnicodeNormalization;

/// Classify a bare word/number token into a leaf value. `true`/`false` are booleans; a well-formed
/// integer or float is that literal; anything else (including a malformed number) is a `Name`.
///
/// Keywords are NOT handled here — that is the parser's job (`token::keyword`); a word like `let`
/// classifies as `Leaf::Name("let")` and only becomes a keyword in grammatical position.
pub fn classify_word(text: &str) -> Leaf {
    classify_word_nonname(text).unwrap_or_else(|| Leaf::Name(text.into()))
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
    // A TYPE-SUFFIXED numeric literal (`100N`, `0.5R`): the body failed the bare int/float parse only
    // because of a trailing suffix letter. Peel a single `N`/`R` and re-parse the body; the body must
    // itself be a well-formed integer (for `N`) or integer-or-float (for `R`). This runs LAST so a bare
    // number (the common case) never pays for it, and so a body that fails to parse falls through to a
    // `Name` (rejected downstream) exactly as a malformed bare number does. A suffix is CASE-SENSITIVE
    // (`100n` is not a suffix — it stays a `Name`), keeping one canonical spelling.
    if let Some(leaf) = classify_suffixed(text) {
        return Some(leaf);
    }
    None
}

/// A numeric literal with a trailing `N`/`R` type suffix → a [`Leaf::Suffixed`], or `None` if `text`
/// is not a suffixed numeric literal (no suffix char, or a body that is not a well-formed literal of a
/// shape the suffix admits). `N` (→ `BigInt`) admits only an INTEGER body; `R` (→ `Rational`) admits
/// an integer OR a decimal body (`5R` = 5/1, `0.5R` = 1/2). Split out of [`classify_word_nonname`] so
/// the bare-number fast path is unaffected.
fn classify_suffixed(text: &str) -> Option<Leaf> {
    let last = text.chars().next_back()?;
    let kind = SuffixKind::from_char(last)?;
    let body = &text[..text.len() - last.len_utf8()];
    // The body must be a well-formed literal on its own. An integer body serves both suffixes; a float
    // body serves only `R` (a fractional `BigInt` is meaningless — `N` over a decimal is not a literal).
    if let Some((value, radix)) = parse_int(body) {
        return Some(Leaf::Suffixed {
            value: SuffixBody::Int { value, radix },
            kind,
        });
    }
    if kind == SuffixKind::Rational
        && let Some(d) = parse_float(body)
    {
        return Some(Leaf::Suffixed {
            value: SuffixBody::Float(d),
            kind,
        });
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
            None => Leaf::BadChar(word.into()), // surrogate or > U+10FFFF
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
        _ => Leaf::BadChar(word.into()),
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
pub fn parse_int(tok: &str) -> Option<(IntValue, Radix)> {
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
        return Some((
            IntValue::from_bigint(&value),
            if is_hex { Radix::Hex } else { Radix::Bin },
        ));
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
    Some((
        IntValue::from_bigint(&if neg { -mag } else { mag }),
        Radix::Dec,
    ))
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
            significand: IntValue::from_bigint(&significand).magnitude,
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
        // The Decimal significand is a non-negative byte magnitude; the sign lives in `negative`.
        significand: IntValue::from_bigint(&significand).magnitude,
        exponent,
    }
}

/// Unescape a string literal's INNER content (between the quotes) and NFC-normalize it — the shared
/// escape table both surfaces use, so a string leaf is identical however it was written. The escape set
/// is CLOSED (`\n \t \r \\ \"`); an unrecognized `\x` is a lexical defect — `Err(x)` names the first
/// offending escape char (the caller turns it into a `Leaf::BadEscape` marker the compiler rejects
/// CDZ0001). `Ok(s)` is the normalized text when every escape is valid.
/// The parsed pieces of a tagged-template token `tag"…{expr}…"`: the tag name, the literal CHUNKS
/// (unescaped: `\n`/`\t`/`\r`/`\\`/`\"` string escapes AND `{{`/`}}` → `{`/`}` brace escapes applied),
/// and the raw SOURCE TEXT of each hole (the text between a hole's outer `{` and matching `}`, which the
/// PARSER re-parses as an ordinary expression). Invariant: `chunks.len() == holes.len() + 1` — a body
/// with N holes has N+1 literal chunks (some possibly empty, e.g. `"{x}"` → chunks `["",""]`, holes
/// `["x"]`). Hole nesting is brace-balanced, and a `"…"` inside a hole shields its braces (so
/// `f("}")` is one hole). This is the shared split the lexer's `read_template_body` scan mirrors; the
/// lexer already validated termination, so this assumes a well-formed body (a stray unmatched brace
/// closes/ignores gracefully rather than panicking).
pub struct TemplateBody {
    pub tag: String,
    pub chunks: Vec<String>,
    pub holes: Vec<String>,
}

/// Split a tagged-template TOKEN (`tag"…"`, the whole lexed span) into its [`TemplateBody`]. Returns
/// `None` if `token` is not `<ident>"…"`-shaped.
//
//= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
//# The reader MUST, when lexing a tagged template, only split the string body into literal chunks and `{…}` holes.
pub fn split_template_body(token: &str) -> Option<TemplateBody> {
    let q = token.find('"')?;
    let tag = token[..q].to_string();
    let body = token[q + 1..].strip_suffix('"').unwrap_or(&token[q + 1..]);
    let mut chunks: Vec<String> = Vec::new();
    let mut holes: Vec<String> = Vec::new();
    let mut chunk = String::new();
    let mut it = body.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            // A string escape in the LITERAL text — decode into the chunk (the closed string set).
            '\\' => match it.next() {
                Some('n') => chunk.push('\n'),
                Some('t') => chunk.push('\t'),
                Some('r') => chunk.push('\r'),
                Some('\\') => chunk.push('\\'),
                Some('"') => chunk.push('"'),
                Some(other) => chunk.push(other), // unknown escape: keep the char (lexer accepted it)
                None => {}
            },
            // `{{` / `}}` — a literal brace in the chunk (not a hole).
            '{' if it.peek() == Some(&'{') => {
                it.next();
                chunk.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                chunk.push('}');
            }
            // `{` opens a hole: the current chunk ends, and the hole's raw source is collected up to the
            // matching `}` (brace-balanced, with `"…"` inside the hole shielding its braces).
            '{' => {
                chunks.push(std::mem::take(&mut chunk));
                let mut hole = String::new();
                let mut depth: u32 = 1;
                let mut in_str = false;
                while let Some(h) = it.next() {
                    match h {
                        // A backslash ESCAPES the next char anywhere — matching the lexer's
                        // `read_template_body` scan. Crucially, an escaped quote (`\"`) must NOT toggle
                        // string mode, and an escaped brace must not adjust depth: consume both chars
                        // verbatim. Without this, a hole like `g("\"}")` mis-tracked `in_str` and its `}`
                        // could prematurely close the hole/template (PR #409).
                        '\\' => {
                            hole.push(h);
                            if let Some(esc) = it.next() {
                                hole.push(esc);
                            }
                        }
                        '"' => {
                            in_str = !in_str;
                            hole.push(h);
                        }
                        '{' if !in_str => {
                            depth += 1;
                            hole.push(h);
                        }
                        '}' if !in_str => {
                            depth -= 1;
                            if depth == 0 {
                                break; // end of this hole (the matching `}` is not part of its text)
                            }
                            hole.push(h);
                        }
                        _ => hole.push(h),
                    }
                }
                holes.push(hole);
            }
            _ => chunk.push(c),
        }
    }
    // The trailing chunk is always pushed once, after every hole has closed its preceding chunk — so a
    // body with N holes yields exactly N+1 chunks (`chunks.len() == holes.len() + 1`), letting the
    // chunks and holes reconstruct the original text in order.
    //
    //= spec/capabilities/metaprogramming.md#a-tagged-template-is-a-binding-dispatched-compile-time-macro-over-literal-chunks-and-holes
    //# The count of literal chunks in a tagged template MUST be exactly one greater than the count of holes, so that the chunks and holes reconstruct the original text in order.
    chunks.push(chunk); // the trailing chunk (always one more chunk than holes)
    Some(TemplateBody { tag, chunks, holes })
}

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
        Ok(s) => Leaf::Str(s.into()),
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
        return Leaf::Sym(body.nfc().collect::<String>().into());
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
    Leaf::Sym(content.into())
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
pub fn render_int(value: &IntValue, radix: Radix) -> String {
    use num_bigint::Sign;
    let value = value.to_bigint();
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

/// Render a TYPE-SUFFIXED numeric literal (`100N`, `0.5R`) — the body in its own canonical form
/// (int in its base, decimal shortest) followed by the suffix character. The dual of the classifier's
/// suffix peel, so a suffixed leaf round-trips to text that re-reads to the same leaf.
pub fn render_suffixed(value: &SuffixBody, kind: SuffixKind) -> String {
    let mut s = match value {
        SuffixBody::Int { value, radix } => render_int(value, *radix),
        SuffixBody::Float(d) => render_decimal(d),
    };
    s.push(kind.suffix_char());
    s
}

/// Render an exact `Decimal` as the shortest text that re-parses to the same value. Always contains
/// a `.` or exponent so it re-lexes as a Float, never an Int. `nan`/`inf` are not `Decimal`s (they
/// are names), so this only ever renders a finite value; `-0.0` prints with its sign.
pub fn render_decimal(d: &Decimal) -> String {
    let sign = if d.negative { "-" } else { "" };
    // The significand is a non-negative byte magnitude; bridge to BigInt for the decimal digits.
    let digits = IntValue {
        negative: false,
        magnitude: d.significand.clone(),
    }
    .to_bigint()
    .to_str_radix(10); // non-negative magnitude
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

/// Like [`escape_string`], but emits a line-feed (`\n`) as a REAL newline instead of the `\n` escape.
/// Every other escape is unchanged (`\t`/`\r`/`\\`/`"` stay escaped), so the string's bytes are
/// preserved EXACTLY — the reader accepts a literal newline inside a `"…"` string, and re-reading yields
/// the identical content. Used by a MULTI-LINE surface rendering (the s-expr pretty printer) to keep a
/// multi-line `(doc "…")` doc-comment readable instead of collapsing it to one `\n`-laden line (seq-282
/// multi-line comment preservation). Round-trip-safe precisely because ONLY the line break is
/// literalized: a continuation line's own leading whitespace is string CONTENT and is emitted verbatim,
/// so the printer never re-indents inside the literal (which would change its bytes).
pub fn escape_string_multiline(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\n' => out.push('\n'),
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
    fn byte_escape_round_trips_every_byte_value() {
        // `escape_bytes` then `unescape_byte_string_token` is the identity for EVERY byte value — the
        // b"…" surface is a lossless byte encoding (named escapes for `\n\t\r\\"`, `\xNN` for the rest,
        // printable ASCII verbatim). A regression in either direction (e.g. an off-by-one on a trailing
        // `\xNN`) would flip some byte, so sweeping all 256 pins the inverse-pair contract.
        for b in 0u8..=255 {
            let tok = format!("b\"{}\"", escape_bytes(&[b]));
            assert_eq!(
                unescape_byte_string_token(&tok),
                vec![b],
                "byte {b} did not round-trip via {tok:?}"
            );
        }
        // Multi-byte sequences, including a non-printable at the END (the position most likely to trip
        // a boundary bug) and the two chars that have both a named escape and are ASCII (`\\`, `"`).
        for seq in [
            vec![0x41u8, 0x00],
            vec![0x00, 0x41],
            vec![0xff, 0xfe, 0x00],
            vec![10, 0xff],
            vec![b'\\', b'"', b'\n'],
            vec![],
        ] {
            let tok = format!("b\"{}\"", escape_bytes(&seq));
            assert_eq!(
                unescape_byte_string_token(&tok),
                seq,
                "seq {seq:?} via {tok:?}"
            );
        }
    }

    #[test]
    fn malformed_byte_escapes_never_panic() {
        // Untrusted `b"…"` content: a bare/short/non-hex `\x`, a trailing backslash, an unknown escape,
        // and an empty body must all decode to SOME bytes without panicking (the lexer can hand these
        // through on odd input). The exact bytes are not the contract here — no-crash is.
        for t in [
            "b\"\\x\"",    // \x with no digits
            "b\"\\x4\"",   // \x with one digit
            "b\"\\xzz\"",  // \x with non-hex
            "b\"\\x4g\"",  // \x with one hex + one non-hex
            "b\"\\\"",     // trailing backslash
            "b\"\\q\"",    // unknown escape
            "b\"\"",       // empty
            "not-a-token", // not b"…"-shaped
        ] {
            let _ = unescape_byte_string_token(t); // must not panic
        }
    }

    #[test]
    fn string_escape_round_trips() {
        // `escape_string` then `unescape_string` is the identity for the closed escape set + arbitrary
        // text (the named escapes and any other char). Covers the chars that MUST escape and some that
        // must not, including unicode.
        for s in [
            "",
            "plain",
            "a\nb\tc\rd",
            "quote\"here",
            "back\\slash",
            "λ中🎉",
            "\"\\\n\t\r",
        ] {
            let round = unescape_string(&escape_string(s)).expect("escaped text re-unescapes");
            assert_eq!(round, s, "string {s:?} did not round-trip");
        }
        // An unrecognized escape is reported (closed set): `\q` → Err('q').
        assert_eq!(unescape_string("\\q"), Err('q'));
        // A trailing backslash is dropped (documented).
        assert_eq!(unescape_string("ab\\"), Ok("ab".to_string()));
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
    }

    #[test]
    fn escape_unescape_is_the_identity_over_generated_strings() {
        // The inverse-pair CONTRACT `unescape_string(escape_string(s)) == s`, swept over random strings —
        // the law all string round-tripping rests on (a printer emits `escape_string`, the reader applies
        // `unescape_string`). The hand-picked cases above cover the obvious chars; this sweeps the whole
        // space, weighted toward the escape-significant chars (`\n \t \r \\ "`) and the brace/quote/unicode
        // neighbours where an escape-table asymmetry would hide, so a regression that made the two sides
        // disagree on some char (e.g. an escape emitted but not recognized, or vice-versa) is caught as a
        // non-identity round-trip. `escape_string` is TOTAL, so every generated string must round-trip.
        // The alphabet includes chars that MUST escape, chars that must NOT, `{`/`}` (template-brace
        // neighbours), control chars, and multi-byte unicode scalars.
        let alphabet: &[char] = &[
            '\n', '\t', '\r', '\\', '"', // the five that must escape
            'a', 'Z', '0', ' ', '{', '}', '\'', '/', // must-not-escape neighbours
            '\0', '\u{7f}', '\u{1b}', // control chars (stand for themselves in a string)
            'λ', '中', '🎉', '\u{a0}', // multi-byte unicode scalars
        ];
        let mut rng = Rng(0xe5ca_9e5c_a9e5_ca01);
        for _ in 0..50_000 {
            let len = (rng.next() % 12) as usize; // 0..=11 chars
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            let escaped = escape_string(&s);
            let round = unescape_string(&escaped).unwrap_or_else(|c| {
                panic!("escape_string({s:?})={escaped:?} failed to unescape at {c:?}")
            });
            assert_eq!(
                round, s,
                "escape→unescape not the identity for {s:?} (via {escaped:?})"
            );
        }
    }

    #[test]
    fn int_render_parse_round_trips_across_radices_and_signs() {
        // `render_int` then `parse_int` preserves value AND radix (the radix is part of a leaf's
        // identity — see the codec). `-0` normalizes to `0` (no signed-zero int).
        for tok in ["42", "-42", "0x2a", "0b101", "-0xff", "0", "255", "-1"] {
            let (v, radix) = parse_int(tok).unwrap_or_else(|| panic!("parse {tok:?}"));
            let rendered = render_int(&v, radix);
            let (v2, radix2) =
                parse_int(&rendered).unwrap_or_else(|| panic!("reparse {rendered:?}"));
            assert_eq!(
                (v.clone(), radix),
                (v2, radix2),
                "int {tok:?} → {rendered:?}"
            );
        }
        // `-0` in any radix renders as an unsigned zero.
        let (z, r) = parse_int("-0").unwrap();
        assert_eq!(render_int(&z, r), "0");
    }

    #[test]
    fn int_render_parse_is_the_identity_over_generated_values_and_radices() {
        // `parse_int(render_int(v, radix)) == (v, radix)` — the number-side inverse-pair, the analogue of
        // the string escape sweep. The radix is part of a leaf's identity (the codec preserves it so the
        // printed form re-reads to the SAME leaf), so BOTH value and radix must survive. The fixed test
        // above pins ~8 points; this sweeps random BigInts (arbitrary magnitude 0..~2^128, both signs)
        // across ALL THREE radices — the whole render/parse space, so a radix-prefix or sign/magnitude
        // regression (e.g. a `0x`/`0b` prefix the parser doesn't round-trip, or a to_str_radix width
        // quirk) surfaces as a non-identity round-trip that no fixed value necessarily hits.
        use num_bigint::Sign;
        let radices = [Radix::Dec, Radix::Hex, Radix::Bin];
        let mut rng = Rng(0x1237_c0de_1237_c0de);
        for _ in 0..30_000 {
            // A random magnitude of 0..=16 bytes (up to 128 bits), and a random sign.
            let nbytes = (rng.next() % 17) as usize;
            let mag: Vec<u8> = (0..nbytes).map(|_| (rng.next() & 0xff) as u8).collect();
            let base = BigInt::from_bytes_be(Sign::Plus, &mag); // non-negative magnitude
            // Negate roughly half the time — but a zero value is never negative (no signed-zero int).
            let value = IntValue::from_bigint(&if base != BigInt::from(0) && rng.next() & 1 == 0 {
                -base
            } else {
                base
            });
            let radix = radices[(rng.next() as usize) % radices.len()];
            let rendered = render_int(&value, radix);
            let (v2, radix2) = parse_int(&rendered).unwrap_or_else(|| {
                panic!("render_int({value:?}, {radix:?})={rendered:?} did not parse")
            });
            assert_eq!(
                (&value, radix),
                (&v2, radix2),
                "int {value:?} @ {radix:?} → {rendered:?} did not round-trip"
            );
        }
    }

    #[test]
    fn ints_with_base() {
        assert_eq!(parse_int("42"), Some((IntValue::from_i64(42), Radix::Dec)));
        assert_eq!(
            parse_int("0x2A"),
            Some((IntValue::from_i64(42), Radix::Hex))
        );
        assert_eq!(
            parse_int("0b101010"),
            Some((IntValue::from_i64(42), Radix::Bin))
        );
        assert_eq!(
            parse_int("-0x10"),
            Some((IntValue::from_i64(-16), Radix::Hex))
        );
        assert_eq!(
            parse_int("1_000_000"),
            Some((IntValue::from_i64(1_000_000), Radix::Dec))
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
    fn type_suffix_classifies_and_round_trips() {
        // `N` selects BigInt over an integer body; `R` selects Rational over an int OR decimal body.
        for (src, want_kind) in [
            ("100N", SuffixKind::BigInt),
            ("0xFFN", SuffixKind::BigInt),
            ("5R", SuffixKind::Rational),
            ("0.5R", SuffixKind::Rational),
            ("1.25R", SuffixKind::Rational),
        ] {
            let leaf = classify_word(src);
            let Leaf::Suffixed { value, kind } = &leaf else {
                panic!("{src} did not classify as Suffixed: {leaf:?}");
            };
            assert_eq!(*kind, want_kind, "{src} suffix kind");
            // Round-trip: render∘classify is identity (the printed form re-reads to the same leaf).
            let printed = render_suffixed(value, *kind);
            assert_eq!(
                classify_word(&printed),
                leaf,
                "{src} → {printed} round-trip"
            );
        }
        // A non-suffix trailing letter, a lowercase suffix, or a fractional `N` is NOT a suffixed
        // literal — it stays a bare `Name` (rejected downstream), never silently mis-read.
        assert!(
            matches!(classify_word("100n"), Leaf::Name(_)),
            "lowercase n"
        );
        assert!(
            matches!(classify_word("0.5N"), Leaf::Name(_)),
            "fractional N"
        );
        assert!(
            matches!(classify_word("100X"), Leaf::Name(_)),
            "unknown suffix"
        );
    }

    #[test]
    fn floats_exact() {
        assert_eq!(
            parse_float("1.5"),
            Some(Decimal {
                negative: false,
                significand: IntValue::from_i64(15).magnitude,
                exponent: -1
            })
        );
        assert_eq!(
            parse_float("1.5e10"),
            Some(Decimal {
                negative: false,
                significand: IntValue::from_i64(15).magnitude,
                exponent: 9
            })
        );
        assert_eq!(
            parse_float("-0.25"),
            Some(Decimal {
                negative: true,
                significand: IntValue::from_i64(25).magnitude,
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
                value: IntValue::from_i64(42),
                radix: Radix::Dec
            }
        );
        assert!(matches!(classify_word("1.5"), Leaf::Float(_)));
        assert_eq!(classify_word("foo"), Leaf::Name("foo".into()));
        // A malformed number stays a Name (rejected downstream), never silently repaired.
        assert_eq!(classify_word("1_"), Leaf::Name("1_".into()));
        // Keywords are ordinary names here — the parser decides keyword-ness.
        assert_eq!(classify_word("let"), Leaf::Name("let".into()));
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
            let value = IntValue::from_i64(v);
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
                significand: IntValue::from_i64(15).magnitude,
                exponent: -1,
            }, // 1.5
            Decimal {
                negative: false,
                significand: IntValue::from_i64(15).magnitude,
                exponent: 9,
            }, // 15e9
            Decimal {
                negative: true,
                significand: IntValue::from_i64(25).magnitude,
                exponent: -2,
            }, // -0.25
            Decimal {
                negative: false,
                significand: IntValue::from_i64(5).magnitude,
                exponent: 0,
            }, // 5.0
            Decimal {
                negative: true,
                significand: IntValue::from_i64(0).magnitude,
                exponent: 0,
            }, // -0.0
            Decimal {
                negative: false,
                significand: IntValue::from_i64(1).magnitude,
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

    /// The exact rational value a `Decimal` denotes, as `(numerator, denominator)` in lowest-common
    /// terms enough for equality: `sig * 10^exp`, expressed with a non-negative power of ten as the
    /// denominator so two decimals compare by cross-multiplication. Sign folded into the numerator.
    fn decimal_value(d: &Decimal) -> (BigInt, BigInt) {
        let ten = BigInt::from(10u32);
        let sig_mag = IntValue {
            negative: false,
            magnitude: d.significand.clone(),
        }
        .to_bigint();
        let sig = if d.negative { -sig_mag } else { sig_mag };
        if d.exponent >= 0 {
            (sig * ten.pow(d.exponent as u32), BigInt::from(1u32))
        } else {
            (sig, ten.pow((-d.exponent) as u32))
        }
    }

    #[test]
    fn float_render_parse_preserves_value_over_generated_decimals() {
        // `parse_float(render_decimal(d))` denotes the SAME rational value as `d` — the float inverse-pair
        // (companion to the int + string sweeps). It is NOT structurally the identity: `parse_float`
        // strips trailing zeros from the significand (150e-2 → 15e-1, 100e0 → 1e2), so the round-trip
        // NORMALIZES. So assert (a) VALUE equality (sig·10^exp equal as rationals, via cross-multiply),
        // and (b) parse_float's output is a FIXED POINT — re-render+re-parse it and get the identical
        // Decimal (the normalized form is stable). Swept over random sign/significand(0..2^80)/exponent
        // (−12..=12), the whole render/parse space, so a placement or trailing-zero-normalization
        // regression surfaces as a value mismatch no fixed decimal necessarily hits.
        use num_bigint::Sign;
        let mut rng = Rng(0xf10a_7c0d_ef10_a701);
        for _ in 0..30_000 {
            let nbytes = (rng.next() % 11) as usize; // significand 0..~2^80
            let mag: Vec<u8> = (0..nbytes).map(|_| (rng.next() & 0xff) as u8).collect();
            let significand = BigInt::from_bytes_be(Sign::Plus, &mag);
            // A zero significand is never negative (no signed-zero decimal — parse yields {0,0}).
            let negative = significand != BigInt::from(0u32) && rng.next() & 1 == 0;
            let exponent = (rng.next() % 25) as i64 - 12; // -12..=12
            let d = Decimal {
                negative,
                significand: IntValue::from_bigint(&significand).magnitude,
                exponent,
            };
            let text = render_decimal(&d);
            let parsed = parse_float(&text).unwrap_or_else(|| {
                panic!("render_decimal({d:?})={text:?} did not parse as a float")
            });
            // (a) VALUE preserved: d and parsed denote equal rationals (cross-multiply the (num,den) forms).
            let (dn, dd) = decimal_value(&d);
            let (pn, pd) = decimal_value(&parsed);
            assert_eq!(
                &dn * &pd,
                &pn * &dd,
                "value not preserved: {d:?} → {text:?} → {parsed:?}"
            );
            // (b) parse_float's output is a FIXED POINT (the normalized form re-renders + re-parses to
            // itself), so any downstream re-normalization cannot drift it.
            assert_eq!(
                parse_float(&render_decimal(&parsed)),
                Some(parsed.clone()),
                "parse_float output is not a fixed point for {d:?} (normalized {parsed:?})"
            );
        }
    }

    #[test]
    fn char_render_parse_is_the_identity_over_generated_scalars() {
        // The char inverse-pair CONTRACT — `char_leaf(render_char(c) without its "#\\") == Char(c)` — swept
        // over random Unicode scalars. `render_char` is what a printer emits for a `Char` leaf; `char_leaf`
        // is what the reader applies to a `#\<word>` token (see parser.rs). They must be inverse: a bug in
        // the `#\u+HHHH` hex form (padding / case / char::from_u32) or the named-control mapping
        // (space/newline/tab/return/null) would silently corrupt a char literal on round-trip. The
        // hand tests cover the obvious chars; this sweeps the whole scalar space, weighted toward the
        // named controls, the hex-escaped control range, ASCII, and multi-byte scalars where an asymmetry
        // hides. (`render_char` strips the `#\` prefix conceptually; `char_leaf` takes the word AFTER it,
        // exactly as the parser does — so we strip it here.)
        let named: &[char] = &[' ', '\n', '\t', '\r', '\0']; // the five with named forms
        let mut rng = Rng(0xc4a5_c0de_c4a5_c0de);
        let mut checked = 0usize;
        // (a) the named controls — hit them deterministically (a fixed set, not generator-luck).
        for &c in named {
            let word = render_char(c)
                .strip_prefix("#\\")
                .expect("render_char emits the #\\ prefix")
                .to_string();
            assert_eq!(
                char_leaf(&word),
                Leaf::Char(c),
                "named control {c:?} did not round-trip (via {word:?})"
            );
        }
        // (b) a broad random sweep over the scalar space (ASCII, Latin-1, control, BMP, astral).
        for _ in 0..50_000 {
            // A random valid Unicode scalar: draw a code point, skip the surrogate gap.
            let cp = (rng.next() % 0x11_0000) as u32;
            let Some(c) = char::from_u32(cp) else {
                continue;
            }; // surrogates → None, skip
            let rendered = render_char(c);
            assert!(
                rendered.starts_with("#\\"),
                "render_char({c:?})={rendered:?} must start with #\\"
            );
            let word = &rendered["#\\".len()..];
            assert_eq!(
                char_leaf(word),
                Leaf::Char(c),
                "char {c:?} (U+{cp:04X}) did not round-trip via {rendered:?}"
            );
            checked += 1;
        }
        assert!(
            checked > 40_000,
            "swept a meaningful scalar space, got {checked}"
        );
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

    #[test]
    fn multiline_escape_literalizes_only_the_newline_and_reparses() {
        // `escape_string_multiline` emits `\n` as a REAL newline (for a multi-line surface rendering) but
        // keeps every OTHER escape, and always round-trips through `unescape_string` to the same content.
        for s in [
            "a\nb",                    // the newline becomes literal
            "one; still\n  two; also", // continuation indentation + `;` are content, preserved
            "tab\there",               // a tab stays escaped (only the newline is literalized)
            "quote\"inside",           // a quote stays escaped
            "back\\slash",             // a backslash stays escaped
            "no newline",              // single-line: identical to escape_string
            "",
        ] {
            let m = escape_string_multiline(s);
            assert_eq!(
                unescape_string(&m).as_deref(),
                Ok(s),
                "multiline escape must reparse to the same content: {s:?} -> {m:?}"
            );
        }
        // The line feed is literal (a real newline), NOT the `\n` escape.
        assert_eq!(escape_string_multiline("a\nb"), "a\nb");
        assert!(!escape_string_multiline("a\nb").contains("\\n"));
        // Other escapes are unchanged vs `escape_string` (only the newline differs).
        assert_eq!(escape_string_multiline("t\tq\"s"), escape_string("t\tq\"s"));
    }

    #[test]
    fn a_string_literal_is_nfc_normalized_at_parse() {
        // A STRING LITERAL's content is NFC-normalized on the way in (unescape_string ends in `.nfc()`),
        // so a decomposed spelling (`e` + combining acute, U+0065 U+0301) and the precomposed `é`
        // (U+00E9) both intern to the SAME NFC bytes. This is the "normalized-construction path" the
        // operator's 2026-07-18 String.from-bytes ruling affirmed (equality follows normalization for
        // LITERALS; from-bytes intentionally does NOT normalize — construction-path-dependent identity).
        // Pin the literal side so a regression that drops the parse-time NFC (making `café` literals
        // construction-spelling-dependent) can't slip through. (Whether identifier NAMES also normalize
        // is a separate open ruling — filed; this pins ONLY the settled literal behavior.)
        let decomposed = "cafe\u{0301}"; // e + combining acute
        let precomposed = "caf\u{00e9}"; // é
        assert_ne!(
            decomposed, precomposed,
            "the two spellings differ BEFORE normalization"
        );
        assert_eq!(
            unescape_string(decomposed).unwrap(),
            unescape_string(precomposed).unwrap(),
            "a decomposed and precomposed string literal normalize to the same NFC content"
        );
        assert_eq!(
            unescape_string(decomposed).unwrap(),
            precomposed,
            "the normalized form is the precomposed (NFC) `café`"
        );
        // The token path (what the reader actually calls) agrees.
        assert_eq!(
            unescape_string_token("\"cafe\u{0301}\""),
            Leaf::Str(precomposed.into()),
            "the string-token reader NFC-normalizes too"
        );
    }

    /// A random tagged-template HOLE's raw source text (bounded by `depth`): identifiers, calls, and
    /// QUOTED STRINGS that carry braces/escaped-quotes (which must shield those braces from the hole's
    /// depth scan), plus nested balanced `{…}` brace groups. Every `{` this emits is matched by a `}`, and
    /// every string is closed — so the text is top-level brace-balanced and the split's hole scan collects
    /// it verbatim up to the template's own closing `}`. This is exactly the shape the PR #409 fix hardened
    /// (a `}` inside a hole's string must NOT prematurely close the hole).
    fn gen_hole(rng: &mut Rng, depth: usize) -> String {
        // Leaves: plain code, plus strings whose interior braces / escaped quote stress the shield.
        let leaves = [
            "x",
            "y",
            "foo",
            "g(a)",
            "\"s\"",      // a quoted string
            "\"a}b{c\"",  // a string whose braces must be shielded from the depth counter
            "\"q\\\"z\"", // a string with an escaped quote (\\" must not toggle string mode)
        ];
        if depth == 0 || rng.next().is_multiple_of(2) {
            return leaves[(rng.next() as usize) % leaves.len()].to_string();
        }
        // A hole must NOT begin with `{`: the hole-opener `{` immediately followed by another `{` reads as
        // a `{{` brace-ESCAPE in the chunk scanner (a hole can never start with `{` — the grammar forbids
        // it), which would desync this test's oracle. So the FIRST part is always a leaf; only SUBSEQUENT
        // parts may be nested balanced `{ … }` groups (a `}` inside the hole is handled by the depth
        // counter, so a trailing group is fine).
        let mut s = leaves[(rng.next() as usize) % leaves.len()].to_string();
        let more = (rng.next() as usize) % 3; // 0..=2 additional parts
        for _ in 0..more {
            if rng.next().is_multiple_of(2) {
                s.push('{');
                s.push_str(&gen_hole(rng, depth - 1));
                s.push('}');
            } else {
                s.push_str(&gen_hole(rng, depth - 1));
            }
        }
        s
    }

    #[test]
    fn split_template_body_round_trips_generated_chunks_and_holes() {
        // `split_template_body` is a PUBLIC tagged-template parser (the parser re-scans a `tag"…"` token
        // through it) with a precise contract — `chunks.len() == holes.len() + 1`, and chunks+holes must
        // reconstruct the source in order — plus the subtle string-/escape-shielded brace hole scan that
        // was the PR #409 bug (a `}` inside a hole's string wrongly closed the hole). It had NO direct unit
        // test. Build a token from KNOWN parts, split it back, and assert we recover exactly those parts:
        //   * chunks come from a SAFE alphabet (no `{}\"` `"` `\\`) so they pass through the escape/brace
        //     decode verbatim — an empty chunk (leading `{`, or two adjacent holes) is included;
        //   * holes are brace-balanced raw source with quoted strings carrying braces (the shield case).
        // A regression in the count invariant, the hole boundary, or the string shield shows as a mismatch.
        let tags = ["t", "sql", "html", "x1"];
        let chunk_alphabet: &[char] = &['a', 'b', ' ', 'Z', '0', '9', '.', '_'];
        let mut rng = Rng(0x7e11_a7e5_c0de_0001);
        for _ in 0..5000 {
            let tag = tags[(rng.next() as usize) % tags.len()];
            let nholes = (rng.next() as usize) % 4; // 0..=3 holes
            let gen_chunk = |rng: &mut Rng| -> String {
                let len = (rng.next() as usize) % 4; // 0..=3 chars (0 → an empty chunk)
                (0..len)
                    .map(|_| chunk_alphabet[(rng.next() as usize) % chunk_alphabet.len()])
                    .collect()
            };
            let mut want_chunks: Vec<String> = Vec::new();
            let mut want_holes: Vec<String> = Vec::new();
            let mut body = String::new();
            for _ in 0..nholes {
                let c = gen_chunk(&mut rng);
                body.push_str(&c);
                want_chunks.push(c);
                let h = gen_hole(&mut rng, 2);
                body.push('{');
                body.push_str(&h);
                body.push('}');
                want_holes.push(h);
            }
            let last = gen_chunk(&mut rng);
            body.push_str(&last);
            want_chunks.push(last);

            let token = format!("{tag}\"{body}\"");
            let parsed = split_template_body(&token)
                .unwrap_or_else(|| panic!("well-formed template {token:?} must split"));
            assert_eq!(parsed.tag, tag, "tag recovered for {token:?}");
            // The count invariant the spec pins.
            assert_eq!(
                parsed.chunks.len(),
                parsed.holes.len() + 1,
                "chunks must be exactly one more than holes for {token:?}"
            );
            assert_eq!(
                parsed.chunks, want_chunks,
                "literal chunks did not round-trip for {token:?}"
            );
            assert_eq!(
                parsed.holes, want_holes,
                "hole raw source did not round-trip for {token:?} (a string-shield or brace-depth bug)"
            );
        }
    }

    #[test]
    fn split_template_body_is_total_on_arbitrary_tokens() {
        // The parser hands `split_template_body` a lexer-VALIDATED token, but the fn must still be TOTAL —
        // never panic — on any input (a defensive contract, and it's `pub`). Sweep a delimiter-rich
        // alphabet (the chars that drive its state machine: braces, quotes, backslash) and assert it only
        // ever returns — and whenever it returns `Some`, the `chunks == holes + 1` invariant still holds
        // (the code pushes a trailing chunk unconditionally, so this must never break, even on a body with
        // an unmatched brace / dangling escape / unterminated string).
        let alphabet: &[char] = &['{', '}', '"', '\\', 'a', ' ', 'n', 't', '(', ')'];
        let mut rng = Rng(0x0bad_7e11_0a7e_0002);
        for _ in 0..20_000 {
            let len = (rng.next() as usize) % 14;
            let core: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            // Both a bare core (may lack a quote → None) and a quote-wrapped `t"…"` shape.
            for token in [core.clone(), format!("t\"{core}\"")] {
                if let Some(parsed) = split_template_body(&token) {
                    assert_eq!(
                        parsed.chunks.len(),
                        parsed.holes.len() + 1,
                        "the chunks==holes+1 invariant must hold even on malformed {token:?}"
                    );
                }
            }
        }
        // A few hand-picked pathological shapes — the exact edges a scan can mis-handle — must not panic.
        for token in [
            "t\"",          // just the opening quote, no closing
            "t\"{",         // an unterminated hole
            "t\"{\"}",      // a hole opened, an unterminated string inside, no close
            "t\"}}}}\"",    // only escaped braces
            "t\"\\\"",      // a dangling backslash before the closing quote
            "t\"{{{{\"",    // only open-brace escapes
            "\"noident\"",  // empty tag
            "notatemplate", // no quote at all → None
        ] {
            let _ = split_template_body(token); // must merely return
        }
    }
}
