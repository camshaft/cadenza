//! Render a wasmtime component `Val` to canonical text — the observable form a test compares.
//!
//! Scalars render directly; a string uses `cadenza-syntax`'s escape table (the dual of the reader's
//! unescape), so a rendered string is byte-identical to what the front-end prints and reads back.
//! Floats follow the corpus value form (`-0.0`, `NaN`, integral floats as `N.0`).
//!
//! Compounds render in the native `#ctor(…)` value-forms (`#list`/`#tuple`/`#record`) — the operator-ruled
//! canonical VALUE render (`#ctor` everywhere for values; TYPE descriptors stay name-head per seq-206). This
//! is the SAME spelling the value-encode / rust-backend / nullary path emits (#5586), so an arg-taking typed-
//! interface-export result (rendered here from a live wasmtime `Val`) matches a nullary program's
//! value-form-encoded `(output …)` — one uniform #ctor value spelling across every face.
//!
//! (A follow-up `render_val_typed` slice adds the guest-result-Ty leaf disambiguation — a `list<u8>` as
//! `Bytes` `b"…"` vs `List UInt8` `#list(…)`, a `string` as a `Symbol` `#"…"` — threaded from the compile
//! via a `cdz-result-type` component custom section.)

use cadenza_syntax::literal;
use wasmtime::component::Val;

/// Render `v` to its canonical text (the type-blind render — a `Val` already tags its shape by variant).
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
        // LIST + TUPLE render in the native `#list(…)` / `#tuple(…)` value-forms — DISTINCT (the native forms
        // disambiguate what the old head-less `(x y)` conflated), matching the value-encode / rust-backend
        // spelling.
        Val::List(xs) => {
            let inner: Vec<String> = xs.iter().map(render_val).collect();
            format!("#list({})", inner.join(" "))
        }
        Val::Tuple(xs) => {
            let inner: Vec<String> = xs.iter().map(render_val).collect();
            format!("#tuple({})", inner.join(" "))
        }
        // A RECORD renders the native `#record((= name value) …)` value-form, in field order — the same
        // spelling the value-encode / resource-escape path prints, so a typed interface-export result
        // (rendered here) matches a `(wit-world …)` case's `(output …)` clause after the #ctor unification.
        Val::Record(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(n, v)| format!("(= {n} {})", render_val(v)))
                .collect();
            format!("#record({})", inner.join(" "))
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

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::component::Val;

    /// Compounds render in the native `#ctor(…)` forms (the operator-ruled #ctor-everywhere value render):
    /// `#list`/`#tuple` are DISTINCT (not the old head-less `(x y)`), a record is `#record((= n v)…)`.
    /// Sums/variants keep the `(case …)` value-form.
    #[test]
    fn compounds_render_native_ctor_forms() {
        let ints = |xs: &[i64]| xs.iter().map(|&i| Val::S64(i)).collect::<Vec<_>>();
        assert_eq!(render_val(&Val::List(ints(&[1, 2]))), "#list(1 2)");
        assert_eq!(render_val(&Val::Tuple(ints(&[1, 2]))), "#tuple(1 2)");
        // list vs tuple of the same elements are now DISTINGUISHABLE.
        assert_ne!(
            render_val(&Val::List(ints(&[3, 4]))),
            render_val(&Val::Tuple(ints(&[3, 4])))
        );
        assert_eq!(
            render_val(&Val::Record(vec![
                ("x".into(), Val::S64(3)),
                ("y".into(), Val::S64(13)),
            ])),
            "#record((= x 3) (= y 13))"
        );
        // nested: a record holding a tuple + a list.
        assert_eq!(
            render_val(&Val::Record(vec![
                ("pair".into(), Val::Tuple(ints(&[3, 4]))),
                ("xs".into(), Val::List(ints(&[3, 6]))),
            ])),
            "#record((= pair #tuple(3 4)) (= xs #list(3 6)))"
        );
        // Sums stay the (case …) value-form (NOT a #ctor).
        assert_eq!(render_val(&Val::Option(None)), "(None unit)");
        assert_eq!(
            render_val(&Val::Option(Some(Box::new(Val::S64(5))))),
            "(Some 5)"
        );
    }
}
