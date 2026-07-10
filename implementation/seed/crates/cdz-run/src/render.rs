//! Render a wasmtime component `Val` to canonical text — the observable form a test compares.
//!
//! Scalars render directly; a string uses `cadenza-syntax`'s escape table (the dual of the reader's
//! unescape), so a rendered string is byte-identical to what the front-end prints and reads back.
//! Floats follow the corpus value form (`-0.0`, `NaN`, integral floats as `N.0`).

use cadenza_syntax::literal;
use wasmtime::component::Val;

/// Render `v` to its canonical text.
pub fn render_val(v: &Val) -> String {
    match v {
        Val::Bool(b) => b.to_string(),
        Val::S8(i) => i.to_string(),
        Val::U8(i) => i.to_string(),
        Val::S16(i) => i.to_string(),
        Val::U16(i) => i.to_string(),
        Val::S32(i) => i.to_string(),
        Val::U32(i) => i.to_string(),
        Val::S64(i) => i.to_string(),
        Val::U64(i) => i.to_string(),
        Val::Float32(f) => display_float(*f as f64),
        Val::Float64(f) => display_float(*f),
        Val::Char(c) => c.to_string(),
        // Closed-escape-set render (the dual of the reader's unescape), so a non-printable scalar
        // renders verbatim and reads back to the same value.
        Val::String(s) => format!("\"{}\"", literal::escape_string(s)),
        Val::List(xs) | Val::Tuple(xs) => {
            let inner: Vec<String> = xs.iter().map(render_val).collect();
            format!("({})", inner.join(" "))
        }
        other => format!("{other:?}"),
    }
}

/// Canonical float rendering, matching the corpus value form: `-0.0`, `NaN`, and integral floats
/// as `N.0`.
pub fn display_float(f: f64) -> String {
    if f == 0.0 && f.is_sign_negative() {
        "-0.0".into()
    } else if f.is_nan() {
        "NaN".into()
    } else if f.fract() == 0.0 && f.is_finite() {
        // `{:.0}` prints the exact integer value of the whole float injectively — unlike `f as i64`,
        // which saturates at i64::MAX so every whole float ≥ 2^63 would collapse to one string.
        format!("{f:.0}.0")
    } else {
        format!("{f}")
    }
}
