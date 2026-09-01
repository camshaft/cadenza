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

use cadenza_syntax::ast::{
    Arenas, Builder, CompoundCtor, Decimal, IntValue, Leaf, Radix, Struct, StructId,
};
use cadenza_syntax::sexpr;
use std::sync::Arc;
use wasmtime::component::Val;

/// Render `v` to its canonical text (the type-blind render — a `Val` already tags its shape by variant).
pub fn render_val(v: &Val) -> String {
    let mut b = Builder::new();
    let root = build_val(&mut b, v, None);
    sexpr::print(&b.finish(root))
}

/// Render `v` using the guest RESULT-TYPE `ty` — the full-fidelity structured type payload (a
/// `cadenza-ast` arena rooted at the guest export's compiled result `Ty`, decoded from the component's
/// `cdz-result-type` section, seq-284 binary-AST wire). We WALK the type subtree HERE (the render
/// projection belongs to cdz-run, the WIT/render owner — the shared codec hands us the raw arena, not a
/// lossy enum) to DISAMBIGUATE the WIT-erased leaves the raw `Val` cannot: a `list<u8>` is a `Bytes`
/// (`b"…"`) vs a `List (UInt 8)` (`#list(…)`), a `list` is a `List` vs a `Set` (`#set(…)`), a `string` is a
/// `String` (`"…"`) vs a `Symbol` (`#"…"`). The type is the GUEST's compiled result type (never the corpus
/// EXPECTED type), so a genuine Bytes-vs-List mismatch still surfaces. Any head the walk does not
/// special-case (a scalar, `Map`, `Nominal`, an unrecognized sum) falls back to the type-blind mapping.
pub fn render_val_typed(v: &Val, ty: &Arenas) -> String {
    let mut b = Builder::new();
    let root = build_val(&mut b, v, Some((ty, ty.root)));
    sexpr::print(&b.finish(root))
}

/// The head NAME + child type-nodes of a type subtree: `(head child…)` → `(Some(head), &[child…])`; a bare
/// leaf-name atom (`Bytes`, `Int64`) → `(Some(name), &[])`. Threads `(arena, child)` to recurse the render
/// disambiguation into element/field/payload types. Mirrors the old `split_type` string parse, now over the
/// decoded arena (structural, not a render-name string).
fn ty_parts(a: &Arenas, id: StructId) -> (Option<&str>, &[StructId]) {
    match a.get(id) {
        Struct::Atom(_) => (a.as_name(id), &[]),
        Struct::List(kids) => (kids.first().and_then(|&h| a.as_name(h)), &kids[1..]),
    }
}

/// The child type-node at `args[i]` paired with its arena, for threading into a nested `build_val`. `None`
/// when `ty` is absent or the index is out of range (→ the child renders type-blind).
fn nth_ty<'a>(
    ty: Option<(&'a Arenas, StructId)>,
    args: &[StructId],
    i: usize,
) -> Option<(&'a Arenas, StructId)> {
    ty.and_then(|(a, _)| args.get(i).map(|&n| (a, n)))
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

/// A Float64 VALUE leaf: NaN → `Leaf::FloatNan` (renders `nan` — the ONE canonical-form change from the old
/// `NaN`), ±inf → `Leaf::FloatInf` (`inf`/`-inf`, = the old Rust `{}` fallthrough), else a `Decimal` from the
/// f64 (integral → `N.0`, `-0.0` sign preserved) → the printer's `render_decimal`.
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

/// A Float32 VALUE leaf rendered at the f32's OWN shortest decimal (`Decimal::from_f32` = `{:e}` on the
/// binary32), NOT the f32→f64-PROMOTED shortest: promoting rendered `28.29` as `28.290000915527344` — a
/// DIFFERENT number (operator ruling: "why would we promote the f32 to f64? those are different values
/// entirely"). NaN/±inf as `float_atom`. Mirrors the wasm value_codec `float32_leaf`, the rcdzc const-fold
/// value render (`const_value_ast` `from_f32`), and the Rust backend — the one-canonical shortest-f32
/// (seq-283): a direct-return Float32 scalar now displays identically to a runtime-heap / const one.
fn float32_atom(b: &mut Builder, f: f32) -> StructId {
    if f.is_nan() {
        b.atom_leaf(Leaf::FloatNan)
    } else if f.is_infinite() {
        b.atom_leaf(Leaf::FloatInf {
            negative: f.is_sign_negative(),
        })
    } else {
        b.atom_leaf(Leaf::Float(
            Decimal::from_f32(f).expect("finite f32 has a Decimal"),
        ))
    }
}

/// Map a wasmtime `Val` (+ optional guest result-`Ty` s-expr for leaf disambiguation) into a `cadenza-ast`
/// VALUE arena node. The printer then renders it canonically. `ty = None` is the type-blind render (a `Val`
/// tags its shape by variant); `ty = Some(...)` supplies the per-leaf CHOICE the WIT boundary erased.
fn build_val(b: &mut Builder, v: &Val, ty: Option<(&Arenas, StructId)>) -> StructId {
    // Peel a closure-FACTORY arrow result-type `(-> p r)` to its result arm BEFORE the Val match: the
    // export's Ty is the function type but the rendered VALUE is the closure's CALL RESULT (of type `r`),
    // so a WIT-erased leaf result (Bytes/Symbol) still disambiguates. Must peel FIRST — a `Val::List`/
    // `Val::String` result would otherwise match its own arm (type-blind) before seeing the arrow. Curried
    // `(-> p (-> q r))` peels one arrow per recursion. (This is why no `DecodedTy::Arrow` was needed — the
    // full arrow is on the wire and we walk the result arm directly.)
    if let Some((a, id)) = ty
        && let (Some("->"), aargs) = ty_parts(a, id)
        && let Some(&r) = aargs.last()
    {
        return build_val(b, v, Some((a, r)));
    }
    // The head constructor + child type-nodes of the (already arrow-peeled) type, for leaf disambiguation.
    let (head, args): (Option<&str>, &[StructId]) = match ty {
        Some((a, id)) => ty_parts(a, id),
        None => (None, &[]),
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
        Val::Float32(f) => float32_atom(b, *f),
        Val::Float64(f) => float_atom(b, *f),
        Val::Char(c) => b.atom_leaf(Leaf::Char(*c)),
        // A `Symbol` crosses as a WIT `string` (a `Val::String`); the guest result-type `Symbol` renders the
        // canonical `#"…"` (`Leaf::Sym`), else a plain `String` renders `"…"` (`Leaf::Str`). Same escape codec.
        Val::String(s) if head == Some("Symbol") => b.atom_leaf(Leaf::Sym(Arc::from(s.as_str()))),
        Val::String(s) => b.atom_leaf(Leaf::Str(Arc::from(s.as_str()))),
        // A `list<u8>` that is a `Bytes` renders `b"…"` (a `Leaf::Bytes` atom), not `#list(…)`. Both cross the
        // WIT boundary as `list<u8>` → `Val::List` of `U8`; the guest type `Bytes` disambiguates.
        Val::List(xs) if head == Some("Bytes") => {
            let bytes: Vec<u8> = xs
                .iter()
                .map(|e| match e {
                    Val::U8(x) => *x,
                    _ => 0,
                })
                .collect();
            b.atom_leaf(Leaf::Bytes(Arc::from(&bytes[..])))
        }
        // A `list` that is a `Set` renders `#set(…)`; else `#list(…)`. `(List e)`/`(Set e)` → the element
        // type is the single arg; thread it so a nested Bytes/Symbol still disambiguates.
        Val::List(xs) => {
            let (elem_ty, ctor) = match head {
                Some("Set") => (nth_ty(ty, args, 0), CompoundCtor::Set),
                Some("List") => (nth_ty(ty, args, 0), CompoundCtor::List),
                _ => (None, CompoundCtor::List),
            };
            let children: Vec<StructId> = xs.iter().map(|x| build_val(b, x, elem_ty)).collect();
            b.compound(ctor, &children)
        }
        Val::Tuple(xs) => {
            // `(Tuple e…)` → element `i`'s type is `args[i]`.
            let children: Vec<StructId> = xs
                .iter()
                .enumerate()
                .map(|(i, x)| build_val(b, x, nth_ty(ty, args, i)))
                .collect();
            b.compound(CompoundCtor::Tuple, &children)
        }
        // A record renders `#record((= name value) …)` — each field a `(= key value)` FieldPair, key a bare
        // `Leaf::Name` (renders `x`, not `"x"`), in field order. The field TYPE is the `(: name T)` ascription.
        Val::Record(fields) => {
            let children: Vec<StructId> = fields
                .iter()
                .map(|(n, val)| {
                    let ft = record_field_ty(ty, args, n);
                    let key = name_atom(b, n);
                    let value = build_val(b, val, ft);
                    b.field_pair(key, value)
                })
                .collect();
            b.compound(CompoundCtor::Record, &children)
        }
        // A guest-ADT sum renders as a plain list with a Name head (NOT a `#ctor`): `(None unit)` / `(Some v)`
        // / `(Ok p)` / `(Err p)` / `(case payload)` / `(enumcase unit)` — matching the recorded value forms.
        // The payload type threads from the `(Sum Option <decl> T)` / `(Sum Result <decl> Ok Err)` args.
        Val::Option(None) => {
            let head = name_atom(b, "None");
            let u = unit_atom(b);
            b.list(vec![head, u])
        }
        Val::Option(Some(x)) => {
            let h = name_atom(b, "Some");
            let inner = build_val(b, x, sum_arg(ty, head, args, "Option", 2));
            b.list(vec![h, inner])
        }
        Val::Result(Ok(p)) => {
            let h = name_atom(b, "Ok");
            let inner = payload_or_unit(b, p.as_deref(), sum_arg(ty, head, args, "Result", 2));
            b.list(vec![h, inner])
        }
        Val::Result(Err(p)) => {
            let h = name_atom(b, "Err");
            let inner = payload_or_unit(b, p.as_deref(), sum_arg(ty, head, args, "Result", 3));
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
        // An unhandled `Val` (Flags / Resource / …) — a debug fallback, as the old renderer did.
        other => b.atom_leaf(Leaf::Str(Arc::from(format!("{other:?}").as_str()))),
    }
}

/// A sum payload node, or the `unit` marker when the payload is absent (`Ok`/`Err`/variant nullary payload).
fn payload_or_unit(b: &mut Builder, p: Option<&Val>, ty: Option<(&Arenas, StructId)>) -> StructId {
    match p {
        Some(v) => build_val(b, v, ty),
        None => unit_atom(b),
    }
}

/// The `idx`-th arg of a `(Sum <name> <decl> arg…)` type subtree, iff its head is `Sum` and its nominal
/// name matches `sum_name` — the Option/Result payload-type threading (`(Sum Option <decl> T)` → T at
/// `args[2]`; `(Sum Result <decl> Ok Err)` → Ok at `args[2]`, Err at `args[3]`). `None` otherwise → the
/// payload renders type-blind.
fn sum_arg<'a>(
    ty: Option<(&'a Arenas, StructId)>,
    head: Option<&str>,
    args: &[StructId],
    sum_name: &str,
    idx: usize,
) -> Option<(&'a Arenas, StructId)> {
    if head != Some("Sum") {
        return None;
    }
    let (a, _) = ty?;
    if a.as_name(*args.first()?)? != sum_name {
        return None;
    }
    args.get(idx).map(|&n| (a, n))
}

/// The field TYPE node for record field `name` from a `(Record (: name T)…)` type's field args — each arg
/// is a `(: name T)` ascription (`as_form(f, ":")` → `[name, T]`). `None` → the field renders type-blind.
fn record_field_ty<'a>(
    ty: Option<(&'a Arenas, StructId)>,
    args: &[StructId],
    name: &str,
) -> Option<(&'a Arenas, StructId)> {
    let (a, _) = ty?;
    for &f in args {
        if let Some(fk) = a.as_form(f, ":")
            && fk.len() == 2
            && a.as_name(fk[0]) == Some(name)
        {
            return Some((a, fk[1]));
        }
    }
    None
}

/// An integer VALUE leaf from a signed `i64` magnitude (decimal radix).
fn int_atom(b: &mut Builder, v: i64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(v),
        radix: Radix::Dec,
    })
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

    /// Build a type-payload `Arenas` rooted at `build`'s node (the `encode_ty_payload` shape the producer
    /// emits), so a typed-render test can supply the structured guest result type the same way the
    /// `cdz-result-type` section does.
    fn ty_arena(build: impl FnOnce(&mut Builder) -> StructId) -> Arenas {
        let mut b = Builder::new();
        let root = build(&mut b);
        b.finish(root)
    }

    /// `render_val_typed` walks the guest result-type arena to disambiguate the WIT-erased leaves.
    #[test]
    fn typed_render_disambiguates_bytes_and_recurses() {
        let u8s = |xs: &[u8]| xs.iter().map(|&b| Val::U8(b)).collect::<Vec<_>>();
        // `Bytes` (a bare leaf) → b"…" for a list<u8>.
        assert_eq!(
            render_val_typed(&Val::List(u8s(&[5, 6])), &ty_arena(|b| b.name("Bytes"))),
            format!("b\"{}\"", cadenza_syntax::literal::escape_bytes(&[5, 6]))
        );
        // `(List (UInt 8))` — a byte list that is NOT Bytes → #list.
        assert_eq!(
            render_val_typed(
                &Val::List(u8s(&[5, 6])),
                &ty_arena(|b| {
                    let head = b.name("List");
                    let uh = b.name("UInt");
                    let w = b.atom_leaf(Leaf::Int {
                        value: IntValue::from_i64(8),
                        radix: Radix::Dec,
                    });
                    let elem = b.list(vec![uh, w]);
                    b.list(vec![head, elem])
                })
            ),
            "#list(5 6)"
        );
        // `(Set Int64)` → #set.
        assert_eq!(
            render_val_typed(
                &Val::List(vec![Val::S64(3), Val::S64(6)]),
                &ty_arena(|b| {
                    let head = b.name("Set");
                    let elem = b.name("Int64");
                    b.list(vec![head, elem])
                })
            ),
            "#set(3 6)"
        );
        // `(Record (: ct Bytes) (: n Int64))` → the ct field disambiguates to b"…".
        assert_eq!(
            render_val_typed(
                &Val::Record(vec![
                    ("ct".into(), Val::List(u8s(&[1, 2]))),
                    ("n".into(), Val::S64(7))
                ]),
                &ty_arena(|b| {
                    let head = b.name("Record");
                    let ct = {
                        let c = b.name(":");
                        let n = b.name("ct");
                        let t = b.name("Bytes");
                        b.list(vec![c, n, t])
                    };
                    let nf = {
                        let c = b.name(":");
                        let n = b.name("n");
                        let t = b.name("Int64");
                        b.list(vec![c, n, t])
                    };
                    b.list(vec![head, ct, nf])
                })
            ),
            format!(
                "#record((= ct b\"{}\") (= n 7))",
                cadenza_syntax::literal::escape_bytes(&[1, 2])
            )
        );
        // A scalar type → type-blind.
        assert_eq!(
            render_val_typed(&Val::S64(42), &ty_arena(|b| b.name("Int64"))),
            "42"
        );
        // An unrecognized head → type-blind (#list, not b"…").
        assert_eq!(
            render_val_typed(
                &Val::List(u8s(&[1, 2])),
                &ty_arena(|b| b.name("SomethingUnknown"))
            ),
            "#list(1 2)"
        );
    }

    /// A `Symbol` result crosses as a WIT `string` (a `Val::String`); the guest result-type `Symbol` renders
    /// the canonical `#"…"`, `String` stays `"…"`.
    #[test]
    fn typed_render_disambiguates_symbol_from_string() {
        assert_eq!(
            render_val_typed(&Val::String("go".into()), &ty_arena(|b| b.name("Symbol"))),
            format!("#\"{}\"", cadenza_syntax::literal::escape_string("go"))
        );
        assert_eq!(
            render_val_typed(&Val::String("go".into()), &ty_arena(|b| b.name("String"))),
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
