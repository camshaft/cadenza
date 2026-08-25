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
        // A RECORD renders in the corpus value-form `(record (= name value) …)`, in field order — the same
        // spelling the resource-escape / codec path prints, so a typed interface-export result (rendered
        // here) matches a `(wit-world …)` case's `(output …)` clause.
        Val::Record(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(n, v)| format!("(= {n} {})", render_val(v)))
                .collect();
            format!("(record {})", inner.join(" "))
        }
        // An OPTION renders `(Some <value>)` / `(None unit)` — the corpus sum value-form (the `unit` payload
        // marks the absent case, matching the reader).
        Val::Option(None) => "(None unit)".to_string(),
        Val::Option(Some(v)) => format!("(Some {})", render_val(v)),
        // A RESULT renders `(Ok <value>)` / `(Err <value>)`; an empty payload renders `unit`.
        Val::Result(Ok(p)) => {
            format!(
                "(Ok {})",
                p.as_deref()
                    .map(render_val)
                    .unwrap_or_else(|| "unit".into())
            )
        }
        Val::Result(Err(p)) => {
            format!(
                "(Err {})",
                p.as_deref()
                    .map(render_val)
                    .unwrap_or_else(|| "unit".into())
            )
        }
        // A VARIANT renders `(<case> <payload>)` / `(<case> unit)` for the nullary case — the case NAME is
        // the WIT/component-model spelling (kebab, e.g. `continue`/`close`), which is the recorded canonical
        // form a corpus `(output …)` matches. An ENUM (no payloads) renders `(<case> unit)` likewise.
        Val::Variant(case, payload) => {
            format!(
                "({case} {})",
                payload
                    .as_deref()
                    .map(render_val)
                    .unwrap_or_else(|| "unit".into())
            )
        }
        Val::Enum(case) => format!("({case} unit)"),
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
