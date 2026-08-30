//! Render a wasmtime component `Val` to canonical text — the observable form a test compares.
//!
//! seq-283/284/287 (operator: "Binary AST is THE data exchange format, no bespoke value renderers"):
//! this maps a wasmtime `Val` (+ its guest result `Ty`, to disambiguate the WIT-erased leaves) into a
//! `cadenza-ast` VALUE arena and routes it through the ONE canonical printer, `cadenza_syntax::sexpr::print`
//! — the SAME renderer `cdz-rust-render` uses. The Ty picks the arena NODE (a `list<u8>` → a `Leaf::Bytes`
//! atom → `b"…"`; a generic list → `compound(List)` → `#list`; a `Set` → `compound(Set)` → `#set`; a
//! `Symbol` → `Leaf::Sym` → `#"…"`); the printer renders it canonically. This replaces the old ~255-line
//! bespoke Val→text renderer (which duplicated the printer's escaping / `#ctor` forms / float form).
//!
//! Byte-identical to the prior bespoke render for every leaf/compound/sum EXCEPT a NaN float: the printer's
//! canonical (round-trippable) value form is `nan`, where the old `display_float` printed `NaN` (re-pinned,
//! v-corpus-harness-verified sole flip 01-literals:298).

use cadenza_syntax::ast::{Builder, CompoundCtor, Decimal, IntValue, Leaf, Radix, StructId};
use cadenza_syntax::sexpr;
use std::sync::Arc;
use wasmtime::component::Val;

/// Render `v` to its canonical text (the type-blind render — a `Val` already tags its shape by variant).
pub fn render_val(v: &Val) -> String {
    let mut b = Builder::new();
    let root = build_val(&mut b, v, None);
    sexpr::print(&b.finish(root))
}

/// Render `v` using the guest RESULT-TYPE s-expr `ty` (the `Ty::render_name` form — e.g. `Bytes`,
/// `(List Int64)`, `(Record (: b1 Int64) (: b2 Bytes))`, `(Tuple Int64 Bytes)`) to DISAMBIGUATE the
/// WIT-erased leaves the raw `Val` cannot: a `list<u8>` is a `Bytes` (`b"…"`) vs a `List UInt8`
/// (`#list(…)`), a `list` is a `List` vs a `Set` (`#set(…)`), a `string` is a `String` (`"…"`) vs a
/// `Symbol` (`#"…"`). The type is the GUEST's compiled result type (threaded from the compile — never the
/// corpus EXPECTED type), so a genuine Bytes-vs-List mismatch still surfaces. Falls back to the type-blind
/// mapping for a scalar, a sum, an unhandled head (`Map`), or a shape/type mismatch — never worse.
pub fn render_val_typed(v: &Val, ty: &str) -> String {
    let mut b = Builder::new();
    let root = build_val(&mut b, v, Some(ty));
    sexpr::print(&b.finish(root))
}

/// A bare-name atom (`Leaf::Name`) — a sum-ctor head (`Some`/`None`/`Ok`/…), an enum/variant case name, a
/// record field key (renders `x`, not `"x"`), or the `unit` payload marker.
fn name_atom(b: &mut Builder, name: &str) -> StructId {
    b.atom_leaf(Leaf::Name(Arc::from(name)))
}

/// The canonical `unit` marker atom — the absent/empty payload (`(None unit)` / `(case unit)`).
fn unit_atom(b: &mut Builder) -> StructId {
    name_atom(b, "unit")
}

/// A float VALUE leaf, matching the old `display_float(*f as f64)`: NaN → `Leaf::FloatNan` (renders `nan` —
/// the ONE canonical-form change from the old `NaN`), ±inf → `Leaf::FloatInf` (`inf`/`-inf`, = the old Rust
/// `{}` fallthrough), else a `Decimal` from the f64 (integral → `N.0`, `-0.0` sign preserved) → the printer's
/// `render_decimal`. Both Float32 and Float64 promote to f64 first (the old renderer did `*f as f64`), so a
/// Float32 renders its f64-promoted shortest form (NOT `Decimal::from_f32`, which would diverge).
fn float_atom(b: &mut Builder, f: f64) -> StructId {
    if f.is_nan() {
        b.atom_leaf(Leaf::FloatNan)
    } else if f.is_infinite() {
        b.atom_leaf(Leaf::FloatInf {
            negative: f.is_sign_negative(),
        })
    } else {
        // `from_f64` is `Some` for every finite f64 (it declines only on non-finite, handled above).
        b.atom_leaf(Leaf::Float(
            Decimal::from_f64(f).expect("finite f64 has a Decimal"),
        ))
    }
}

/// Map a wasmtime `Val` (+ optional guest result-`Ty` s-expr for leaf disambiguation) into a `cadenza-ast`
/// VALUE arena node. The printer then renders it canonically. `ty = None` is the type-blind render (a `Val`
/// tags its shape by variant); `ty = Some(...)` supplies the per-leaf CHOICE the WIT boundary erased.
fn build_val(b: &mut Builder, v: &Val, ty: Option<&str>) -> StructId {
    // Split the type (if any) into its head constructor + argument sub-exprs, for the leaf disambiguation.
    let (head, args): (&str, Vec<&str>) = match ty {
        Some(t) => split_type(t),
        None => ("", Vec::new()),
    };
    match v {
        Val::Bool(x) => b.atom_leaf(Leaf::Bool(*x)),
        Val::S8(i) => int_atom(b, *i as i64),
        Val::U8(i) => int_atom(b, *i as i64),
        Val::S16(i) => int_atom(b, *i as i64),
        Val::U16(i) => int_atom(b, *i as i64),
        Val::S32(i) => int_atom(b, *i as i64),
        Val::U32(i) => int_atom(b, *i as i64),
        Val::S64(i) => int_atom(b, *i),
        // A `u64` above `i64::MAX` needs the unsigned magnitude (a signed cast would render negative).
        Val::U64(i) => b.atom_leaf(Leaf::Int {
            value: IntValue::from_u128(*i as u128),
            radix: Radix::Dec,
        }),
        Val::Float32(f) => float_atom(b, *f as f64),
        Val::Float64(f) => float_atom(b, *f),
        Val::Char(c) => b.atom_leaf(Leaf::Char(*c)),
        // A `Symbol` crosses as a WIT `string` (a `Val::String`); the guest result-type `Symbol` renders the
        // canonical `#"…"` (`Leaf::Sym`), else a plain `String` renders `"…"` (`Leaf::Str`). Same escape codec.
        Val::String(s) if head == "Symbol" => b.atom_leaf(Leaf::Sym(Arc::from(s.as_str()))),
        Val::String(s) => b.atom_leaf(Leaf::Str(Arc::from(s.as_str()))),
        // A `list<u8>` that is a `Bytes` renders `b"…"` (a `Leaf::Bytes` atom), not `#list(…)`. Both cross the
        // WIT boundary as `list<u8>` → `Val::List` of `U8`; the guest type `Bytes` disambiguates.
        Val::List(xs) if head == "Bytes" => {
            let bytes: Vec<u8> = xs
                .iter()
                .map(|e| match e {
                    Val::U8(x) => *x,
                    _ => 0,
                })
                .collect();
            b.atom_leaf(Leaf::Bytes(Arc::from(&bytes[..])))
        }
        // A `list` that is a `Set` renders `#set(…)`; else `#list(…)`. Elements thread the element type.
        Val::List(xs) => {
            let elem_ty = if head == "Set" || head == "List" {
                args.first().copied()
            } else {
                None
            };
            let ctor = if head == "Set" {
                CompoundCtor::Set
            } else {
                CompoundCtor::List
            };
            let children: Vec<StructId> = xs.iter().map(|x| build_val(b, x, elem_ty)).collect();
            b.compound(ctor, &children)
        }
        Val::Tuple(xs) => {
            let children: Vec<StructId> = xs
                .iter()
                .enumerate()
                .map(|(i, x)| build_val(b, x, args.get(i).copied()))
                .collect();
            b.compound(CompoundCtor::Tuple, &children)
        }
        // A record renders `#record((= name value) …)` — each field a `(= key value)` FieldPair, key a bare
        // `Leaf::Name` (renders `x`, not `"x"`), in field order.
        Val::Record(fields) => {
            let children: Vec<StructId> = fields
                .iter()
                .map(|(n, val)| {
                    let ft = ty.and_then(|_| record_field_type(&args, n));
                    let key = name_atom(b, n);
                    let value = build_val(b, val, ft);
                    b.field_pair(key, value)
                })
                .collect();
            b.compound(CompoundCtor::Record, &children)
        }
        // A guest-ADT sum renders as a plain list with a Name head (NOT a `#ctor`): `(None unit)` / `(Some v)`
        // / `(Ok p)` / `(Err p)` / `(case payload)` / `(enumcase unit)` — matching the recorded value forms.
        Val::Option(None) => {
            let head = name_atom(b, "None");
            let u = unit_atom(b);
            b.list(vec![head, u])
        }
        Val::Option(Some(x)) => {
            let h = name_atom(b, "Some");
            let inner = build_val(b, x, args.first().copied());
            b.list(vec![h, inner])
        }
        Val::Result(Ok(p)) => {
            let h = name_atom(b, "Ok");
            let inner = payload_or_unit(b, p.as_deref(), args.first().copied());
            b.list(vec![h, inner])
        }
        Val::Result(Err(p)) => {
            let h = name_atom(b, "Err");
            let inner = payload_or_unit(b, p.as_deref(), args.get(1).copied());
            b.list(vec![h, inner])
        }
        // A VARIANT renders `(<case> <payload>)` / `(<case> unit)`; an ENUM (no payload) `(<case> unit)`. The
        // case NAME is the WIT/component-model (kebab) spelling — the recorded canonical form.
        Val::Variant(case, payload) => {
            let h = name_atom(b, case);
            let inner = payload_or_unit(b, payload.as_deref(), None);
            b.list(vec![h, inner])
        }
        Val::Enum(case) => {
            let h = name_atom(b, case);
            let u = unit_atom(b);
            b.list(vec![h, u])
        }
        // A CLOSURE result-type `(-> param… result)`: the export is a closure FACTORY, so its result-Ty is
        // the function type — but the VALUE is the closure's CALL RESULT. Render as the arrow's LAST arm.
        _ if head == "->" => build_val(b, v, args.last().copied()),
        // An unhandled `Val` (Flags / Resource / …) — a debug fallback, as the old renderer did.
        other => b.atom_leaf(Leaf::Str(Arc::from(format!("{other:?}").as_str()))),
    }
}

/// A sum payload node, or the `unit` marker when the payload is absent (`Ok`/`Err`/variant nullary payload).
fn payload_or_unit(b: &mut Builder, p: Option<&Val>, ty: Option<&str>) -> StructId {
    match p {
        Some(v) => build_val(b, v, ty),
        None => unit_atom(b),
    }
}

/// An integer VALUE leaf from a signed `i64` magnitude (decimal radix).
fn int_atom(b: &mut Builder, v: i64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(v),
        radix: Radix::Dec,
    })
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
    for (i, &bb) in bytes.iter().enumerate() {
        match bb {
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

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::component::Val;

    /// Compounds render in the native `#ctor(…)` forms (the operator-ruled #ctor-everywhere value render):
    /// `#list`/`#tuple` are DISTINCT (not the old head-less `(x y)`), a record is `#record((= n v)…)`.
    /// Sums/variants keep the `(case …)` value-form. (Byte-equivalence gate for the arena rewrite — these
    /// strings are exactly what the old bespoke renderer produced.)
    #[test]
    fn compounds_render_native_ctor_forms() {
        let ints = |xs: &[i64]| xs.iter().map(|&i| Val::S64(i)).collect::<Vec<_>>();
        assert_eq!(render_val(&Val::List(ints(&[1, 2]))), "#list(1 2)");
        assert_eq!(render_val(&Val::Tuple(ints(&[1, 2]))), "#tuple(1 2)");
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
        assert_eq!(
            render_val(&Val::Record(vec![
                ("pair".into(), Val::Tuple(ints(&[3, 4]))),
                ("xs".into(), Val::List(ints(&[3, 6]))),
            ])),
            "#record((= pair #tuple(3 4)) (= xs #list(3 6)))"
        );
        assert_eq!(render_val(&Val::Option(None)), "(None unit)");
        assert_eq!(
            render_val(&Val::Option(Some(Box::new(Val::S64(5))))),
            "(Some 5)"
        );
    }

    /// `render_val_typed` uses the guest result type to disambiguate the WIT-erased leaves.
    #[test]
    fn typed_render_disambiguates_bytes_and_recurses() {
        let u8s = |xs: &[u8]| xs.iter().map(|&b| Val::U8(b)).collect::<Vec<_>>();
        assert_eq!(
            render_val_typed(&Val::List(u8s(&[5, 6])), "Bytes"),
            format!("b\"{}\"", cadenza_syntax::literal::escape_bytes(&[5, 6]))
        );
        assert_eq!(
            render_val_typed(&Val::List(u8s(&[5, 6])), "(List UInt8)"),
            "#list(5 6)"
        );
        assert_eq!(
            render_val_typed(&Val::List(vec![Val::S64(3), Val::S64(6)]), "(Set Int64)"),
            "#set(3 6)"
        );
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
                cadenza_syntax::literal::escape_bytes(&[1, 2])
            )
        );
        assert_eq!(render_val_typed(&Val::S64(42), "Int64"), "42");
        assert_eq!(
            render_val_typed(&Val::List(u8s(&[1, 2])), "SomethingUnknown"),
            "#list(1 2)"
        );
    }

    /// A `Symbol` result crosses as a WIT `string` (a `Val::String`); the guest result-type `Symbol` renders
    /// the canonical `#"…"`, `String` stays `"…"`.
    #[test]
    fn typed_render_disambiguates_symbol_from_string() {
        assert_eq!(
            render_val_typed(&Val::String("go".into()), "Symbol"),
            format!("#\"{}\"", cadenza_syntax::literal::escape_string("go"))
        );
        assert_eq!(
            render_val_typed(&Val::String("go".into()), "String"),
            format!("\"{}\"", cadenza_syntax::literal::escape_string("go"))
        );
    }

    /// Float VALUE forms match the old `display_float(*f as f64)`: integral `N.0`, `-0.0` sign preserved,
    /// ±inf `inf`/`-inf` — and NaN → the canonical `nan` (the SOLE change from the old `NaN`, re-pinned).
    #[test]
    fn floats_render_canonical_value_forms() {
        assert_eq!(render_val(&Val::Float64(3.0)), "3.0");
        assert_eq!(render_val(&Val::Float64(-0.0)), "-0.0");
        assert_eq!(render_val(&Val::Float64(1.5)), "1.5");
        assert_eq!(render_val(&Val::Float64(f64::INFINITY)), "inf");
        assert_eq!(render_val(&Val::Float64(f64::NEG_INFINITY)), "-inf");
        assert_eq!(render_val(&Val::Float64(f64::NAN)), "nan");
        // Float32 promotes to f64 (its f64-promoted shortest form), like the old renderer.
        assert_eq!(render_val(&Val::Float32(2.0)), "2.0");
    }
}
