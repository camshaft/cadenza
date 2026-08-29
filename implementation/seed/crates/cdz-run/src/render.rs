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

/// Render `v` using the guest RESULT-TYPE s-expr `ty` (the `Ty::render_name` form — e.g. `Bytes`,
/// `(List Int64)`, `(Record (: b1 Int64) (: b2 Bytes))`, `(Tuple Int64 Bytes)`) to DISAMBIGUATE the
/// WIT-erased leaves the raw `Val` cannot: a `list<u8>` is a `Bytes` (`b"…"`) vs a `List UInt8`
/// (`#list(…)`), a `list` is a `List` vs a `Set` (`#set(…)`), a `string` is a `String` (`"…"`) vs a
/// `Symbol` (`#"…"`). The `Val` already tags List/Tuple/Record/Option/Result by variant, so `ty` only
/// supplies the per-leaf render CHOICE + threads through the compounds to each leaf. MIRRORS
/// `cadenza-syntax`'s printer per leaf (`b"…"` via `literal::escape_bytes`, `#list`/`#set`/`#tuple`/`#record`),
/// so the direct-WIT-lifted grade render and the resource-escape decode-print render are byte-identical. The
/// type is the GUEST's compiled result type (threaded from the compile — never the corpus EXPECTED type), so
/// a genuine Bytes-vs-List mismatch still surfaces. Falls back to the type-blind [`render_val`] for a scalar,
/// a variant/sum, an unhandled head (`Map`), or a shape/type mismatch — never worse than the untyped render.
pub fn render_val_typed(v: &Val, ty: &str) -> String {
    let (head, args) = split_type(ty);
    match (v, head) {
        // THE KEY DISAMBIGUATION: a `list<u8>` that is a `Bytes` renders `b"…"`, not `#list(…)`. Both cross
        // the WIT boundary as `list<u8>` -> `Val::List` of `U8`.
        (Val::List(xs), "Bytes") => {
            let bytes: Vec<u8> = xs
                .iter()
                .map(|e| match e {
                    Val::U8(b) => *b,
                    _ => 0,
                })
                .collect();
            format!("b\"{}\"", literal::escape_bytes(&bytes))
        }
        (Val::List(xs), "List") => {
            let et = args.first().copied().unwrap_or("");
            let inner: Vec<String> = xs.iter().map(|x| render_val_typed(x, et)).collect();
            format!("#list({})", inner.join(" "))
        }
        (Val::List(xs), "Set") => {
            let et = args.first().copied().unwrap_or("");
            let inner: Vec<String> = xs.iter().map(|x| render_val_typed(x, et)).collect();
            format!("#set({})", inner.join(" "))
        }
        (Val::Tuple(xs), "Tuple") => {
            let inner: Vec<String> = xs
                .iter()
                .enumerate()
                .map(|(i, x)| render_val_typed(x, args.get(i).copied().unwrap_or("")))
                .collect();
            format!("#tuple({})", inner.join(" "))
        }
        (Val::Record(fields), "Record") => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(n, val)| {
                    let ft = record_field_type(&args, n).unwrap_or("");
                    format!("(= {n} {})", render_val_typed(val, ft))
                })
                .collect();
            format!("#record({})", inner.join(" "))
        }
        (Val::Option(Some(x)), "Option") => {
            format!(
                "(Some {})",
                render_val_typed(x, args.first().copied().unwrap_or(""))
            )
        }
        (Val::Result(Ok(p)), "Result") => format!(
            "(Ok {})",
            p.as_deref()
                .map(|x| render_val_typed(x, args.first().copied().unwrap_or("")))
                .unwrap_or_else(|| "unit".into())
        ),
        (Val::Result(Err(p)), "Result") => format!(
            "(Err {})",
            p.as_deref()
                .map(|x| render_val_typed(x, args.get(1).copied().unwrap_or("")))
                .unwrap_or_else(|| "unit".into())
        ),
        // A SYMBOL crosses as a WIT `string` (a `Val::String`), like a plain `String` — the type-blind render
        // can't tell them apart. The canonical Symbol value-form is `#"…"` (`cadenza_syntax` `Leaf::Sym`);
        // disambiguate by the guest result-type `Symbol`. Symbols + Strings share the string-escape codec.
        (Val::String(s), "Symbol") => format!("#\"{}\"", literal::escape_string(s)),
        // A CLOSURE result-type `(-> param… result)`: the export is a closure FACTORY, so its result-Ty is
        // the function type — but the VALUE here is the closure's CALL RESULT. Render as the arrow's LAST arm.
        (_, "->") => render_val_typed(v, args.last().copied().unwrap_or("")),
        // Scalars, `(None …)`, a variant/enum, an unhandled head (`Map`), or a shape/type mismatch -> the
        // type-blind render (correct there; a mismatch is never worse).
        _ => render_val(v),
    }
}

/// Split a canonical type s-expr into its HEAD constructor + its balanced-paren ARGUMENT sub-exprs. A bare
/// atom (`Bytes`, `Int64`) has no args. `(List Int64)` -> `("List", ["Int64"])`; `(Tuple Int64 (List Bytes))`
/// -> `("Tuple", ["Int64", "(List Bytes)"])`; `(Record (: a Int64) (: b Bytes))` -> the two ascriptions.
fn split_type(ty: &str) -> (&str, Vec<&str>) {
    let t = ty.trim();
    let Some(inner) = t.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return (t, Vec::new());
    };
    let inner = inner.trim();
    let bytes = inner.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b' ' | b'\t' | b'\n' if depth == 0 => {
                if i > start {
                    parts.push(inner[start..i].trim());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < inner.len() {
        let last = inner[start..].trim();
        if !last.is_empty() {
            parts.push(last);
        }
    }
    match parts.split_first() {
        Some((head, rest)) => (head, rest.to_vec()),
        None => (inner, Vec::new()),
    }
}

/// The field TYPE for record field `name` from a `(Record …)` type's arg list — each arg is a `(: name Type)`
/// ascription (the `Ty::render_name` record form). `None` -> the caller renders that field type-blind.
fn record_field_type<'a>(fields: &[&'a str], name: &str) -> Option<&'a str> {
    for f in fields {
        let (fh, fargs) = split_type(f);
        if fh == ":" && fargs.first().copied() == Some(name) {
            return fargs.get(1).copied();
        }
    }
    None
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
    /// `render_val_typed` uses the guest result type to disambiguate the WIT-erased leaves the raw `Val`
    /// cannot: a `list<u8>` is `Bytes` (`b"…"`) vs `List UInt8` (`#list`), a `list` is `List` vs `Set`
    /// (`#set`) — threading the type through compounds to reach a nested Bytes leaf. The `b"…"` matches
    /// `cadenza-syntax`'s `literal::escape_bytes`.
    #[test]
    fn typed_render_disambiguates_bytes_and_recurses() {
        let u8s = |xs: &[u8]| xs.iter().map(|&b| Val::U8(b)).collect::<Vec<_>>();
        assert_eq!(
            render_val_typed(&Val::List(u8s(&[5, 6])), "Bytes"),
            format!("b\"{}\"", literal::escape_bytes(&[5, 6]))
        );
        assert_eq!(
            render_val_typed(&Val::List(u8s(&[5, 6])), "(List UInt8)"),
            "#list(5 6)"
        );
        assert_eq!(
            render_val_typed(&Val::List(vec![Val::S64(3), Val::S64(6)]), "(Set Int64)"),
            "#set(3 6)"
        );
        // NESTED: a record with a Bytes field + an Int field — the type reaches the Bytes leaf.
        assert_eq!(
            render_val_typed(
                &Val::Record(vec![
                    ("ct".into(), Val::List(u8s(&[1, 2]))),
                    ("n".into(), Val::S64(7))
                ]),
                "(Record (: ct Bytes) (: n Int64))"
            ),
            format!(
                "#record((= ct b\"{}\") (= n 7))",
                literal::escape_bytes(&[1, 2])
            )
        );
        // scalar / mismatch → type-blind render (never worse).
        assert_eq!(render_val_typed(&Val::S64(42), "Int64"), "42");
        assert_eq!(
            render_val_typed(&Val::List(u8s(&[1, 2])), "SomethingUnknown"),
            "#list(1 2)"
        );
    }

    /// A `Symbol` result crosses as a WIT `string` (a `Val::String`), like a plain `String`; the guest
    /// result-type `Symbol` renders the canonical `#"…"` value-form, `String` stays `"…"`.
    #[test]
    fn typed_render_disambiguates_symbol_from_string() {
        assert_eq!(
            render_val_typed(&Val::String("go".into()), "Symbol"),
            format!("#\"{}\"", literal::escape_string("go"))
        );
        assert_eq!(
            render_val_typed(&Val::String("go".into()), "String"),
            format!("\"{}\"", literal::escape_string("go"))
        );
    }
}
