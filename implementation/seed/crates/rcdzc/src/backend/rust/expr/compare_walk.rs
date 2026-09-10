//! Value-tree comparison/equality/ordering walkers — extracted from `expr.rs` to keep it under
//! `xtask_support::MAX_SOURCE_BYTES` (512 KiB). Pure code move, behavior-neutral; the emitted Rust is
//! byte-identical. `use super::*` brings the parent `expr` module items (Db, Ty, NameCtx, enums::*, the
//! emit helpers, ...) into scope, exactly as the `wasm/select/*` submodules do for `select.rs`. The
//! moved fns are `pub(super)` so the parent's `use compare_walk::*;` re-imports them, leaving every
//! call site in `expr.rs` unchanged.
use super::*;

/// Whether `ty` is a compound whose leaves are each either native-`Eq` (via [`enums::ty_supports_eq`]) or a
/// FLOAT — so a structural walk ([`emit_value_eq_walk`]) can compare it (float leaves by the canonical byte
/// form, the rest by `==`). Only the shapes the walk emits qualify: a TUPLE/RECORD/NOMINAL/LIST of walkable
/// leaves, or a bare float. A sum/map/fn does NOT (a runtime sum eq folds or declines at `lower`; admitting
/// them here would emit an unhandled shape). Returns false when the type is ALREADY native-Eq (that path is
/// taken before this) or carries a non-Eq-non-float leaf (a function, an unknown var).
pub(super) fn ty_float_walkable(db: &mut Db, ty: &Ty) -> bool {
    ty_float_walkable_seen(db, ty, &mut Vec::new())
}

/// Whether a float-carrying MONOMORPHIC sum can be given a hand-written `impl Ord` (so it is usable as a
/// `BTreeSet`/`BTreeMap` key) via [`emit_value_ord_walk`]. TRUE iff: it is a `Ty::Sum`; it is NOT already
/// native-`Ord` (that path derives `Ord` directly — this is only for the ELSE); every payload is
/// float-walkable (`ty_float_walkable` — the eq/ord walks render exactly this domain); it carries NO
/// flip-order `Option` (the ord-walk's native-`.cmp()` fast path would give the WRONG order for an Option
/// leaf — excluded here, declined rather than miscompiled); and it is MONOMORPHIC (no type args — a generic
/// helper signature is a follow-up). `Ast` (a `Float`+`List Ast` sum, no Option, monomorphic) qualifies.
pub(crate) fn sum_is_custom_ord(db: &mut Db, ty: &Ty) -> bool {
    let Ty::Sum { args, .. } = ty else {
        return false;
    };
    if !args.is_empty() {
        return false; // generic — a generic __ord_ helper signature is a follow-up
    }
    // Native-Ord sums derive Ord (handled elsewhere); this predicate is only for the float-carrying ELSE.
    if crate::backend::rust::enums::ty_supports_eq(db, ty) {
        return false;
    }
    if ty_uses_flip_order_option(db, ty) {
        return false; // an Option leaf's native .cmp() order is the reverse of Cadenza's — decline
    }
    ty_float_walkable(db, ty)
}

/// `ty_float_walkable` with a recursion guard for the SUM descent — a self-referential sum (e.g. `Ast` via
/// `Ast.List (List Ast)`) closes on `seen` so the walk terminates. `emit_value_eq_walk`'s `Ty::Sum` arm
/// renders EXACTLY this domain (the doc invariant that both backends route the same types).
pub(super) fn ty_float_walkable_seen(
    db: &mut Db,
    ty: &Ty,
    seen: &mut Vec<crate::ast::StructId>,
) -> bool {
    match ty {
        // A bare float leaf — walkable (canonical-byte compare).
        Ty::Float(_) => true,
        // A native-Eq leaf — walkable (plain `==`). Checked here so a tuple element that is itself Eq
        // (an Int, a Bytes, a nested all-Eq tuple) passes without needing the float path.
        _ if crate::backend::rust::enums::ty_supports_eq(db, ty) => true,
        // A tuple/record is walkable iff every element/field is. A nominal newtype walks its inner.
        Ty::Tuple(elems) => {
            let elems: Vec<Ty> = elems.to_vec();
            elems.iter().all(|e| ty_float_walkable_seen(db, e, seen))
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().all(|v| ty_float_walkable_seen(db, v, seen))
        }
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            ty_float_walkable_seen(db, &inner, seen)
        }
        // A Qty erases to its inner magnitude (unit is compile-time); walk the inner.
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_float_walkable_seen(db, &inner, seen)
        }
        // A LIST — walkable iff its element is (a `List<Float>` compares element-wise via the `.iter().zip()`
        // walk, each float element by canonical byte form). This is what `Core::ValueEqShaped` routes here:
        // a list spine that native `==` (`Vec: PartialEq`) would compare with the wrong NaN/-0.0 answer.
        Ty::List(elem) => {
            let elem = (**elem).clone();
            ty_float_walkable_seen(db, &elem, seen)
        }
        // A MAP — walkable iff its KEY is an ord-key (so keys compare by `==`: an Int/String/… natively, a
        // float KEY via the `Eq` `__CdzF64` wrapper) AND its VALUE is walkable. Reached only when the Map is
        // NOT already native-`Eq` (a float / float-carrying VALUE — a float KEY alone keeps the Map native-`Eq`
        // via the wrapper, #7419, and takes the `==` fast-path). `emit_value_eq_walk`'s Map arm zips the sorted
        // `(k, v)` pairs: keys by `==`, the value by the value walk (a float value by canonical byte form —
        // `{5: NaN} == {5: NaN}`, matching wasm's map value-eq, NOT `BTreeMap`'s derived `PartialEq`).
        Ty::Map(k, v) => {
            let k = (**k).clone();
            let v = (**v).clone();
            types::ty_is_ord_key(db, &k) && ty_float_walkable_seen(db, &v, seen)
        }
        // A SUM whose payloads carry a Float and/or a List (so it is NOT already native-Eq — that path was
        // taken above) — walkable iff every variant's payload is. `emit_value_eq_walk` renders a Sum through
        // a generated recursive helper `fn __eq_<Ident>` (call-indirection), so a self-referential sum (e.g.
        // `Ast` via `Ast.List (List Ast)`, or a `Box`ed `Tree.Node Tree Tree`) is fine: a recursive back-edge
        // returns TRUE (the helper's runtime recursion terminates over the finite value), matching the wasm
        // `eq_shaped_walkable` (whose runtime walk is likewise iterative/recursive). A GENERIC recursive sum
        // is the one exception the emit still declines (a generic helper signature is a follow-up), but the
        // back-edge here can't see the args cheaply and a false-positive just yields a clean emit-time decline
        // downstream (reject-don't-miscompile), so admit it and let `emit_value_eq_walk` draw that line.
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return true; // recursive back-edge — the helper `fn` breaks the cycle at runtime (see emit)
            }
            seen.push(*decl);
            let variant_count = db.type_decl_by_occ(*decl).map(|t| t.variants.len());
            let mut ok = variant_count.is_some();
            if let Some(vc) = variant_count {
                for disc in 0..vc as u32 {
                    // A nullary variant (no payload) is walkable; a payload variant's payload must be.
                    if let Some(payload_ty) = variant_payload_ty(db, ty, disc)
                        && !ty_float_walkable_seen(db, &payload_ty, seen)
                    {
                        ok = false;
                        break;
                    }
                }
            }
            seen.pop();
            ok
        }
        // A map, function, or unknown var — not walked by this slice.
        _ => false,
    }
}

/// Emit a boolean Rust expression comparing the two OWNED Rust expressions `l`/`r` (both of type `ty`) by
/// STRUCTURAL value equality — the recursive walk for a compound carrying a FLOAT leaf that cannot use a
/// derived `==`. Each leaf compares as: a native-`Eq` leaf → `(l == r)`; a FLOAT leaf → the canonical byte
/// form (NaN-canonicalizing bit compare, `nan==nan`, `-0.0 != +0.0`, byte-identical to `FloatCompare`'s
/// emit + the wasm heap walk); a TUPLE/RECORD → the `&&`-chain of its projected fields (`.0`/`.1`… in
/// rust_type's element order, which for a record is sorted-key order); a NOMINAL → its inner (the newtype is
/// transparent). `l`/`r` are already-bound identifiers (or field projections built on them), so re-reading
/// them per leaf is sound (a projection of a bound value; the enclosing bind is done once by the caller).
pub(crate) fn emit_value_eq_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    emit_value_eq_walk_seen(db, ty, l, r, &mut Vec::new(), helpers)
}

/// The ORD twin of [`emit_value_eq_walk`]: build a `core::cmp::Ordering` expression comparing two values of a
/// FLOAT-CARRYING type (one that is NOT native-`Ord`-derivable — a float leaf makes the derive impossible)
/// in the blessed lexicographic order, with each float leaf ordered by its CANONICAL BIT FORM (NaN folded to
/// one form, matching the runtime's canonical-byte float order and `emit_value_eq_walk`'s float arm). Used to
/// give a float-carrying sum (`Ast`) a hand-written `impl Ord` so it can be a `BTreeSet`/`BTreeMap` key —
/// `enums.rs` wraps the generated `__ord_<Ident>` helper in the trait impl. Mirrors the eq-walk's shape
/// (native-`Ord` fast path, tuple/record lexicographic, list element-wise-then-length, sum via a recursive
/// helper). REQUIRES the type carry no flip-order `Option` (the admission gate ensures this): the native
/// `.cmp()` fast path for an Option-free native-Ord leaf is then the correct Cadenza order.
pub(crate) fn emit_value_ord_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    emit_value_ord_walk_seen(db, ty, l, r, &mut Vec::new(), helpers)
}

pub(super) fn emit_value_ord_walk_seen(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    // A NATIVELY-`Ord` leaf (Int/Bool/Bytes/String/BigInt/… and any all-`Ord`-DERIVING compound/sum) — a
    // plain `.cmp()`. Checked FIRST so an `Ord` sub-tree compares in one `.cmp()` rather than being walked;
    // it is also the ONLY spelling for a map/set leaf the walk does not descend. Gate on `ty_derives_eq`
    // (the DERIVE condition), NOT `ty_is_ord`: a CUSTOM-ord sum (`Ast`) satisfies `ty_is_ord` but its `.cmp()`
    // IS this very `__ord_` helper — short-circuiting to `l.cmp(&r)` would recurse forever. Such a sum must
    // fall through to the `Ty::Sum` walk arm. A flip-order Option would also be wrong here, but the admission
    // gate (`sum_is_custom_ord`) excludes it. The `&` handles a non-Copy compound.
    if crate::backend::rust::enums::ty_supports_eq(db, ty) {
        // `ty_supports_eq` = the enum/compound DERIVES Eq (hence Ord) — a native `.cmp()` is the Cadenza
        // order. BigInt/Rational reach here too: `ty_derives_eq`'s tail arm returns `true` for them (they map
        // to `cdz_num::Big`/`Rational`, which derive `Ord`), so this one branch covers them — no separate
        // BigInt/Rational arm is needed (an earlier one was dead + its comment self-contradicting; dropped per
        // github-liaison's PR#1617 review). A custom-ord sum is NOT Eq-deriving (it carries a float), so it
        // does not take this path — it falls through to the `Ty::Sum` walk arm (whose `.cmp()` IS this
        // `__ord_` helper; short-circuiting here would recurse forever). The `&` handles a non-Copy compound.
        return Ok(format!("{l}.cmp(&{r})"));
    }
    match ty {
        // A FLOAT leaf — order by the canonical bit pattern (NaN folded to one form), mirroring the eq-walk's
        // float arm but with `.cmp()` on the bits (a total order over the canonicalized `u{32,64}`). This
        // matches the runtime's canonical-byte float order (so a float set/map key agrees with wasm).
        Ty::Float(ft) => {
            let (canon_nan, bits_ty) = if ft.ground_width() == 32 {
                ("0x7FC0_0000u32", "u32")
            } else {
                ("0x7FF8_0000_0000_0000u64", "u64")
            };
            let canon = |v: &str| {
                format!(
                    "({{ let __f = {v}; if __f.is_nan() {{ {canon_nan} }} else {{ __f.to_bits() as {bits_ty} }} }})"
                )
            };
            Ok(format!("{}.cmp(&{})", canon(l), canon(r)))
        }
        // A TUPLE — lexicographic: compare element 0, and only on `Equal` fall through (`.then_with`).
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let mut acc: Option<String> = None;
            for (i, e) in elems.iter().enumerate() {
                let part = emit_value_ord_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A RECORD — a tuple in sorted-key order; same lexicographic chain over `.i`.
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            let mut acc: Option<String> = None;
            for (i, e) in tys.iter().enumerate() {
                let part = emit_value_ord_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A LIST — `Vec<T>` compared element-wise lexicographically, then by length (the derived-`Vec` Ord
        // shape): the first non-Equal zipped element decides, else compare lengths. Built over bound refs.
        Ty::List(elem) => {
            let elem = (**elem).clone();
            let elem_cmp = emit_value_ord_walk_seen(db, &elem, "__le", "__re", seen, helpers)?;
            Ok(format!(
                "{l}.iter().zip({r}.iter()).map(|(__le, __re)| {elem_cmp}).find(|__o| *__o != core::cmp::Ordering::Equal).unwrap_or_else(|| {l}.len().cmp(&{r}.len()))"
            ))
        }
        // A NOMINAL newtype is transparent — walk the inner over the same operands (no projection).
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_ord_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A Qty erases to its inner magnitude — walk the inner (same operands).
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_ord_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A SUM whose payloads carry a Float/List (so it is NOT native-Ord — that path was taken first).
        // Compared through a generated recursive helper `fn __ord_<Ident>(l, r) -> Ordering` that matches
        // `(l, r)`: a same-variant pair compares payloads (lexicographic over the payload walk); a
        // mismatched pair compares the DECLARED discriminant ORDINALS (Cadenza declared order = the enum
        // declaration order, which for a monomorphic user sum matches the emitted enum's variant order). The
        // helper (call-indirection) makes a RECURSIVE sum (`Ast.List (List Ast)`) terminate at runtime —
        // exactly like the eq-walk's `__eq_<Ident>`. Monomorphic only (a generic re-entry declines).
        Ty::Sum { decl, args } => {
            let enum_ty = crate::backend::rust::types::rust_type(&db.name_ctx(), ty)
                .ok_or_else(|| Reject::decline("sum ord: no rust type for the enum"))?;
            let name = db
                .type_decl_by_occ(*decl)
                .map(|t| t.name.clone())
                .ok_or_else(|| Reject::decline("sum ord: no decl name"))?;
            let fn_name = format!("__ord_{}", crate::backend::rust::types::sum_ident(&name));
            let sum_ty = ty.clone();
            if seen.contains(&sum_ty) {
                if !args.is_empty() {
                    // (Internal: rendering this would need a generic helper fn; not built yet.)
                    return Err(Reject::unsupported(
                        "runtime ordering over a recursive generic sum is not supported by the Rust backend",
                    ));
                }
                return Ok(format!("{fn_name}(&{l}, &{r})"));
            }
            if !args.is_empty() {
                return Err(Reject::unsupported(
                    "runtime ordering over a generic sum is not supported by the Rust backend",
                ));
            }
            seen.push(sum_ty);
            let variant_count = match db.type_decl_by_occ(*decl).map(|t| t.variants.len()) {
                Some(n) => n,
                None => {
                    seen.pop();
                    return Err(Reject::decline("sum ord: no variant count"));
                }
            };
            // Same-variant arms compare payloads; the fallthrough compares declared ordinals. `__ord_pos`
            // maps a ref to its declared position (a small helper match, emitted alongside).
            let mut same_arms = Vec::with_capacity(variant_count + 1);
            let mut pos_arms = Vec::with_capacity(variant_count);
            let mut arm_err: Option<Reject> = None;
            for disc in 0..variant_count as u32 {
                let path = match sum_variant_path_of_ty(db, ty, disc) {
                    Ok(p) => p,
                    Err(e) => {
                        arm_err = Some(e);
                        break;
                    }
                };
                match variant_payload_ty(db, ty, disc) {
                    None => {
                        // Nullary — same-variant pair is `Equal`; position arm maps the bare ctor to its disc.
                        same_arms.push(format!("({path}, {path}) => core::cmp::Ordering::Equal,"));
                        pos_arms.push(format!("{path} => {disc}u32,"));
                    }
                    Some(payload_ty) => {
                        let deref =
                            if crate::backend::rust::enums::variant_is_recursive(db, ty, disc) {
                                "**"
                            } else {
                                "*"
                            };
                        let lp = format!("({deref}__lp)");
                        let rp = format!("({deref}__rp)");
                        match emit_value_ord_walk_seen(db, &payload_ty, &lp, &rp, seen, helpers) {
                            Ok(cmp) => {
                                same_arms.push(format!("({path}(__lp), {path}(__rp)) => {cmp},"));
                                pos_arms.push(format!("{path}(_) => {disc}u32,"));
                            }
                            Err(e) => {
                                arm_err = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
            seen.pop();
            if let Some(e) = arm_err {
                return Err(e);
            }
            // A same-discriminant pair is handled by an arm above; a mismatched pair compares declared
            // ordinals via the position helper. The `__pos` closure maps each operand to its declared disc.
            same_arms.push("_ => __pos(l).cmp(&__pos(r)),".to_string());
            if !helpers
                .iter()
                .any(|h| h.contains(&format!("fn {fn_name}(")))
            {
                helpers.push(format!(
                    "#[allow(unused)] fn {fn_name}(l: &{enum_ty}, r: &{enum_ty}) -> core::cmp::Ordering {{ \
                     fn __pos(v: &{enum_ty}) -> u32 {{ match v {{ {} }} }} \
                     match (l, r) {{ {} }} }}",
                    pos_arms.join(" "),
                    same_arms.join(" ")
                ));
            }
            Ok(format!("{fn_name}(&{l}, &{r})"))
        }
        _ => Err(Reject::unsupported(
            "runtime ordering over this compound is not supported by the Rust backend",
        )),
    }
}

/// [`emit_value_eq_walk`] with a `seen` set of sum decls currently being expanded (the recursion guard that
/// routes a self-referential sum through its helper `fn` instead of expanding inline) and a `helpers` sink
/// that collects the generated recursive `fn __eq_<Ident>(l, r) -> bool` definitions. A user SUM is compared
/// through such a helper (call-indirection), mirroring the render crate's `__render_<Ident>`: this is what
/// makes a RECURSIVE sum (via a `Box`ed variant OR through a `List`/tuple element as in `Ast.List (List
/// Ast)`) terminate — inlining a `match` per payload would expand UNBOUNDEDLY at compile time (a codegen
/// stack overflow), but the helper moves the recursion to Rust RUNTIME over the finite value. A self-
/// referential payload position, reached while the sum is on `seen`, emits a CALL to the same helper. The
/// caller hoists `helpers` into the enclosing block so the `fn`s are in scope where the returned `cmp` runs.
pub(super) fn emit_value_eq_walk_seen(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    // Keyed on the FULL instantiated sum type (`Ty`, compared by decl+args), NOT the bare `StructId` decl —
    // mirrors the value-CMP walk. A NESTED distinct instantiation like `(Option (Option Float64))` (which
    // reaches this walk because the Float leaf is NOT native-`Eq`, so the `ty_supports_eq` `==` fast-path
    // above does not fire) re-enters the SAME `Option` decl but is a DIFFERENT type; a decl-only key
    // false-tripped the "recursive generic" decline. Keyed on the full type, only TRUE self-recursion
    // (identical decl+args) re-enters — nested instantiations expand inline (finite depth).
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    // A native-Eq leaf (Int/Bool/Bytes/String/BigInt/… and any all-Eq compound) — a plain `==`. Checked
    // FIRST so an Eq sub-tree compares in one `==` rather than being walked field-by-field (identical
    // result, smaller emit; and it is the ONLY path for a sum/list/map leaf, which the walk does not spell).
    if crate::backend::rust::enums::ty_supports_eq(db, ty) {
        return Ok(format!("({l} == {r})"));
    }
    match ty {
        // A FLOAT leaf — the canonical byte form (mirror `FloatCompare`'s FEq emit). Canonicalize each side
        // to its integer bit pattern with NaN folded to one form, then integer-`==`.
        Ty::Float(ft) => {
            let (canon_nan, bits_ty) = if ft.ground_width() == 32 {
                ("0x7FC0_0000u32", "u32")
            } else {
                ("0x7FF8_0000_0000_0000u64", "u64")
            };
            let canon = |v: &str| {
                format!(
                    "({{ let __f = {v}; if __f.is_nan() {{ {canon_nan} }} else {{ __f.to_bits() as {bits_ty} }} }})"
                )
            };
            Ok(format!("({} == {})", canon(l), canon(r)))
        }
        // A TUPLE — the `&&`-chain of element comparisons, each projected `.i` off both operands.
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let mut parts = Vec::with_capacity(elems.len());
            for (i, e) in elems.iter().enumerate() {
                parts.push(emit_value_eq_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?);
            }
            Ok(join_and(parts))
        }
        // A RECORD — a tuple in rust_type's SORTED-KEY order, so project `.i` over the sorted fields.
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            let mut parts = Vec::with_capacity(tys.len());
            for (i, e) in tys.iter().enumerate() {
                parts.push(emit_value_eq_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?);
            }
            Ok(join_and(parts))
        }
        // A LIST — a `Vec<T>`, compared element-wise: equal LENGTHS and every zipped element equal under the
        // element walk (a float element by canonical byte form). `.len()` first (a length mismatch decides
        // immediately, and short-circuits the zip), then `.iter().zip().all()` with the element comparison
        // built over the bound refs `__le`/`__re`. This is the rust twin of the wasm `value-eq-shaped` list
        // spine walk — element-wise so a concat-built and a push-built `[1.0, 2.0]` compare equal (§"Two
        // lists ... equal ... independent of how each was constructed"), and the float leaf uses the
        // canonical byte form (NOT `Vec`'s derived `PartialEq`, which would give the wrong NaN/-0.0 answer).
        Ty::List(elem) => {
            let elem = (**elem).clone();
            let elem_cmp = emit_value_eq_walk_seen(db, &elem, "__le", "__re", seen, helpers)?;
            Ok(format!(
                "({l}.len() == {r}.len() && {l}.iter().zip({r}.iter()).all(|(__le, __re)| {elem_cmp}))"
            ))
        }
        // A MAP reaches this walk ONLY when its VALUE is not native-`Eq` (a float value, or a compound
        // carrying one) — a `Map` whose value IS `Eq` took the `==` fast-path above. The KEY is always an
        // ord-key (hence `Eq`: a float key is the `__CdzF64` wrapper, canonical), so keys compare by `==`;
        // the VALUE is a RAW slot (`f64` for a float value), walked so a NaN/-0.0 value compares by the
        // canonical byte form (matching wasm's map value-eq — `{5: NaN} == {5: NaN}`), NOT `BTreeMap`'s
        // derived `PartialEq` (which would give the wrong NaN answer). Both `BTreeMap`s iterate in sorted
        // KEY order, so equal maps yield the same `(k, v)` sequence: equal lengths + every zipped pair with
        // equal key and value-walk-equal value. (No `Set` arm here — a `Set` is always native-`Eq`, closes
        // its element via the ord-wrapper, so it never reaches this walk.)
        Ty::Map(_k, v) => {
            let vty = (**v).clone();
            let val_cmp = emit_value_eq_walk_seen(db, &vty, "__lv", "__rv", seen, helpers)?;
            Ok(format!(
                "({l}.len() == {r}.len() && {l}.iter().zip({r}.iter()).all(|((__lk, __lv), (__rk, __rv))| __lk == __rk && ({val_cmp})))"
            ))
        }
        // A NOMINAL newtype is transparent — its Rust value IS the inner, so walk the inner over the same
        // operands (no projection; the newtype adds no Rust wrapper).
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_eq_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A Qty erases to its inner magnitude — walk the inner (same operands).
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_eq_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A SUM whose payloads carry a Float/List (so it is NOT native-Eq — that path was taken first). It is
        // compared through a generated recursive helper `fn __eq_<Ident>(l: &Enum, r: &Enum) -> bool` that
        // `match`es `(l, r)` over the emitted enum's variants: each variant arm binds its payload on BOTH
        // sides and walks the payload (a float leaf by canonical byte form, a list element-wise); a nullary
        // variant → `true`; a mismatched variant pair → the `_ => false` catch-all. This EXACTLY mirrors the
        // render crate's `__render_<Ident>` helper (cdz-rust-render) and the wasm `value-eq-shaped` Shape::Sum
        // walk. Routing through a helper (call-indirection) is what makes a RECURSIVE sum TERMINATE: a
        // self-referential payload (via a `Box`ed variant OR through a `List`/tuple element, `Ast.List (List
        // Ast)`) would expand the emit UNBOUNDEDLY if inlined (a codegen stack overflow), but the helper moves
        // the recursion to Rust RUNTIME over the finite value — a self-reference reached while the decl is on
        // `seen` emits a CALL to the same helper, and a nullary leaf terminates the runtime walk.
        //
        // GENERIC instantiations (args non-empty) still DECLINE on re-entry: a helper for `Box<T0>` would need
        // a spelled generic signature (`fn __eq_Box<T0: ?>(…)`) with the right payload bound — a follow-up.
        // A non-recursive generic sum is native-Eq (took the `==` path); only a recursive generic one reaches
        // here, and it declines cleanly (todo, not a miscompile — wasm computes it).
        Ty::Sum { decl, args } => {
            let enum_ty = crate::backend::rust::types::rust_type(&db.name_ctx(), ty)
                .ok_or_else(|| Reject::decline("sum eq: no rust type for the enum"))?;
            let name = db
                .type_decl_by_occ(*decl)
                .map(|t| t.name.clone())
                .ok_or_else(|| Reject::decline("sum eq: no decl name"))?;
            let fn_name = format!("__eq_{}", crate::backend::rust::types::sum_ident(&name));
            let sum_ty = ty.clone();
            // On re-entry of THIS EXACT instantiated type (a true self-referential cycle — identical
            // decl+args), emit a CALL to its helper (the recursion base). A GENERIC self-recursive sum still
            // can't spell a generic helper signature → decline. But a NESTED DISTINCT instantiation
            // (`Option<Option<T>>` reaching `Option<T>`) is a DIFFERENT type, does NOT re-enter, and takes the
            // inline-match generic path below — the gap this closes (was falsely declined as recursive-generic).
            if seen.contains(&sum_ty) {
                if !args.is_empty() {
                    return Err(Reject::unsupported(
                        "runtime structural equality over a recursive generic sum is not supported by the Rust backend",
                    ));
                }
                return Ok(format!("{fn_name}(&{l}, &{r})"));
            }
            seen.push(sum_ty);
            let variant_count = match db.type_decl_by_occ(*decl).map(|t| t.variants.len()) {
                Some(n) => n,
                None => {
                    seen.pop();
                    return Err(Reject::decline("sum eq: no variant count"));
                }
            };
            let mut arms = Vec::with_capacity(variant_count + 1);
            let mut arm_err: Option<Reject> = None;
            for disc in 0..variant_count as u32 {
                let path = match sum_variant_path_of_ty(db, ty, disc) {
                    Ok(p) => p,
                    Err(e) => {
                        arm_err = Some(e);
                        break;
                    }
                };
                match variant_payload_ty(db, ty, disc) {
                    None => {
                        // Nullary variant — a bare `Enum::V` on both sides is equal (the discriminant matched).
                        arms.push(format!("({path}, {path}) => true,"));
                    }
                    Some(payload_ty) => {
                        // One payload field (a single type OR a tuple type — the walk handles both). A
                        // recursive variant boxes the field, so the bound ref derefs one extra level.
                        let deref =
                            if crate::backend::rust::enums::variant_is_recursive(db, ty, disc) {
                                "**"
                            } else {
                                "*"
                            };
                        // PARENTHESIZE the deref: the payload walk may append a method call (a `List` payload
                        // emits `{l}.len()`/`{l}.iter()`), and `.` binds tighter than prefix `*`, so a bare
                        // `*__lp.len()` parses as `*(__lp.len())` (deref of the `usize`, E0614). `(*__lp)` /
                        // `(**__lp)` binds the deref first. (Not hit before: a `List`-carrying sum payload was
                        // always declined as recursive; the helper now renders it, exercising this path.)
                        let lp = format!("({deref}__lp)");
                        let rp = format!("({deref}__rp)");
                        match emit_value_eq_walk_seen(db, &payload_ty, &lp, &rp, seen, helpers) {
                            Ok(cmp) => arms.push(format!("({path}(__lp), {path}(__rp)) => {cmp},")),
                            Err(e) => {
                                arm_err = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
            seen.pop();
            if let Some(e) = arm_err {
                return Err(e);
            }
            // Mismatched-variant pair → not equal. (Only reached when the two discriminants differ; a matched
            // pair took its arm above.)
            arms.push("_ => false,".to_string());
            // A GENERIC instantiation is NOT routed through a helper (no generic signature to spell) — emit
            // the inline `match` as before (a non-recursive generic sum works; a recursive one already
            // declined above on re-entry). Only a MONOMORPHIC user sum generates + calls a helper `fn`.
            if !args.is_empty() {
                return Ok(format!("(match (&{l}, &{r}) {{ {} }})", arms.join(" ")));
            }
            // Emit the helper `fn` once (a re-entry on the same decl returned a call above, so a given decl's
            // helper is pushed exactly once per top-level walk). `#[allow(unused)]` — a mutually-referenced
            // helper may be defined but only reached via another. The helper takes `&Enum` refs (the caller
            // passes `&value`, and a boxed self-reference deref-then-re-borrows via the call `&{l}`).
            if !helpers
                .iter()
                .any(|h| h.contains(&format!("fn {fn_name}(")))
            {
                helpers.push(format!(
                    "#[allow(unused)] fn {fn_name}(l: &{enum_ty}, r: &{enum_ty}) -> bool {{ match (l, r) {{ {} }} }}",
                    arms.join(" ")
                ));
            }
            Ok(format!("{fn_name}(&{l}, &{r})"))
        }
        // Any other shape should have been excluded by `ty_float_walkable` before we got here.
        _ => Err(Reject::unsupported(
            "runtime structural equality over this compound is not supported by the Rust backend",
        )),
    }
}

/// Join boolean parts with `&&`, yielding `true` for an empty list (an empty tuple/record is always equal to
/// itself) and the sole part unparenthesized for a singleton. A multi-part chain is parenthesized so it
/// composes as one boolean sub-expression inside a larger `&&`.
pub(super) fn join_and(parts: Vec<String>) -> String {
    match parts.len() {
        0 => "true".to_string(),
        1 => parts.into_iter().next().unwrap(),
        _ => format!("({})", parts.join(" && ")),
    }
}

/// Whether `ty` contains (at any depth) a built-in `Option` whose Rust std-`Option` DERIVED ORDER
/// DISAGREES with the Cadenza declared variant order — the soundness trap `emit_value_cmp_walk` exists to
/// fix. Cadenza declares `Some` (disc 0) `< None` (disc 1), but Rust's `std::option::Option` derives
/// `None < Some` — the REVERSE. So a native `l < r` / `l.cmp(&r)` on an `Option`-typed (or Option-containing)
/// value gives the WRONG total order (`compare (Some 3) None` → std `Greater`, Cadenza `Less`). `Result`
/// maps to std `Result` whose `Ok < Err` MATCHES Cadenza's declared `Ok < Err`, so it needs no correction;
/// only `Option` flips. A NON-flip type (no std-Option anywhere) keeps the native compare (byte-identical to
/// before — the overwhelmingly common case). A USER `(type Option …)` emits its own decl-order enum (correct
/// native Ord), so it is NOT a flip — `is_builtin_std_sum` distinguishes it. (breaker/corpus-bugfix #42.)
pub(super) fn ty_uses_flip_order_option(db: &mut Db, ty: &Ty) -> bool {
    ty_uses_flip_order_option_seen(db, ty, &mut Vec::new())
}

pub(super) fn ty_uses_flip_order_option_seen(
    db: &mut Db,
    ty: &Ty,
    seen: &mut Vec<crate::ast::StructId>,
) -> bool {
    match ty.strip_nominal() {
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            elems
                .iter()
                .any(|e| ty_uses_flip_order_option_seen(db, e, seen))
        }
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            tys.iter()
                .any(|t| ty_uses_flip_order_option_seen(db, t, seen))
        }
        Ty::List(elem) => {
            let elem = (**elem).clone();
            ty_uses_flip_order_option_seen(db, &elem, seen)
        }
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_uses_flip_order_option_seen(db, &inner, seen)
        }
        s @ Ty::Sum { decl, .. } => {
            let decl_occ = *decl;
            // The std-mapped `Option` builtin is the ONLY flip. Check via the emit's own recognizer.
            let is_flip_option = db
                .type_decl_by_occ(decl_occ)
                .map(|d| {
                    let d = d.clone();
                    crate::backend::rust::enums::is_builtin_std_sum(db, &d) && d.name == "Option"
                })
                .unwrap_or(false);
            if is_flip_option {
                return true;
            }
            // RECURSION GUARD: a self-referential sum (e.g. `Ast` carrying `List Ast`) would otherwise loop
            // forever through its payloads. Skip a decl already on the descent path — if it were flip-order
            // it'd have returned true at its first (Option) visit; re-entering it adds no new Option.
            if seen.contains(&decl_occ) {
                return false;
            }
            seen.push(decl_occ);
            // Otherwise recurse into the variant payloads — a user sum / Result may CARRY an Option leaf.
            let s = s.clone();
            let vcount = db
                .type_decl_by_occ(decl_occ)
                .map(|d| d.variants.len())
                .unwrap_or(0);
            let found = (0..vcount as u32).any(|disc| {
                variant_payload_ty(db, &s, disc)
                    .map(|p| ty_uses_flip_order_option_seen(db, &p, seen))
                    .unwrap_or(false)
            });
            seen.pop();
            found
        }
        _ => false,
    }
}

/// Emit a Rust `core::cmp::Ordering` expression comparing `l`/`r` (both of type `ty`) in the CADENZA
/// DECLARED total order — the correction for the std-`Option` order flip ([`ty_uses_flip_order_option`]).
/// Used by `Core::ValueCmp` ONLY when `ty` contains a flip-order `Option`; an Option-free type keeps the
/// native `l < r` / `l.cmp(&r)` (byte-identical). The walk is lexicographic (matching core-semantics
/// §Compound Ordering Is Lexicographic + the wasm value-cmp walk), delegating an Option-FREE subtree to the
/// native `.cmp()` (correct there — only Option's derived Ord disagrees) and handling an `Option` position
/// by an EXPLICIT `Some`-before-`None` match so the order is `Some(_) < None` (Cadenza), overriding std's
/// `None < Some`. `helpers` collects generated recursive `fn`s (a self-referential Option-carrying sum),
/// mirroring `emit_value_eq_walk`.
pub(super) fn emit_value_cmp_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    emit_value_cmp_walk_seen(db, ty, l, r, &mut Vec::new(), helpers)
}

pub(super) fn emit_value_cmp_walk_seen(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    // The recursion guard keys on the FULL instantiated sum type (`Ty`, compared by decl + args), NOT the
    // bare `StructId` decl: a NESTED distinct instantiation like `(Option (Option Int64))` re-enters the
    // SAME `Option` decl but is a DIFFERENT type, so a decl-only key would false-trip the "recursive
    // generic" decline. Keyed on the full type, only a TRULY self-referential type (identical decl+args)
    // re-enters — the finite value's own cycle — and nested instantiations expand inline (finite depth).
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    // An Option-FREE subtree compares correctly under the native derived `Ord` — emit `l.cmp(&r)` and stop
    // walking (smaller emit, and it is the ONLY spelling for a Map/Set/other leaf the walk does not descend).
    // The ref-`&` handles a non-Copy compound; a Copy scalar coerces fine. This is what keeps a compare with
    // NO Option byte-identical to the pre-fix native path (the walk only diverges at an actual Option).
    if !ty_uses_flip_order_option(db, ty) {
        return Ok(format!("{l}.cmp(&{r})"));
    }
    match ty.strip_nominal().clone() {
        // A TUPLE — lexicographic: compare field 0, and only on `Equal` fall through to the next (`.then_with`).
        Ty::Tuple(elems) => {
            let mut acc: Option<String> = None;
            for (i, e) in elems.iter().enumerate() {
                let part = emit_value_cmp_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A RECORD — a tuple in sorted-key order; same lexicographic chain over `.i`.
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            let mut acc: Option<String> = None;
            for (i, e) in tys.iter().enumerate() {
                let part = emit_value_cmp_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A LIST — `Vec<T>` compared element-wise lexicographically, then by length (the derived `Vec` Ord
        // shape): zip + find the first non-Equal element compare, else compare lengths. Built over bound refs.
        Ty::List(elem) => {
            let elem_cmp = emit_value_cmp_walk_seen(db, &elem, "__le", "__re", seen, helpers)?;
            Ok(format!(
                "{l}.iter().zip({r}.iter()).map(|(__le, __re)| {elem_cmp}).find(|__o| *__o != core::cmp::Ordering::Equal).unwrap_or_else(|| {l}.len().cmp(&{r}.len()))"
            ))
        }
        Ty::Qty { inner, .. } => emit_value_cmp_walk_seen(db, &inner, l, r, seen, helpers),
        // A SUM. The Option case (the flip) is ordered `Some`-before-`None` (Cadenza) by the declared-ordinal
        // match; any other sum carrying an Option leaf compares by declared discriminant then payload — the
        // correct Cadenza order, computed WITHOUT trusting std's derived Ord.
        Ty::Sum { .. } => emit_sum_cmp_walk(db, &ty.strip_nominal().clone(), l, r, seen, helpers),
        // Unreachable: an Option-free shape (scalar/float/…) took the native `.cmp()` early-return above, and
        // only Tuple/Record/List/Qty/Sum can CONTAIN an Option. Decline defensively rather than miscompile.
        _ => Err(Reject::decline(
            "value-cmp walk reached an unexpected Option-containing shape",
        )),
    }
}

/// The sum arm of [`emit_value_cmp_walk_seen`]: compare two sum values in CADENZA DECLARED variant order
/// (discriminant ascending by declaration, then payload lexicographically) via a generated `match (l, r)`.
/// This is what overrides std `Option`'s `None < Some`: the arms are emitted in DECLARATION order (`Some`
/// disc 0 first, `None` disc 1), and a lower-disc-vs-higher-disc pair yields `Less`/`Greater` by declared
/// position — NOT by std's derived discriminant. Routed through a helper `fn __cmp_<Ident>` for a recursive
/// sum (like `emit_value_eq_walk`'s `__eq_` helper) so it terminates.
pub(super) fn emit_sum_cmp_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    let sum_ty = ty.strip_nominal().clone();
    match &sum_ty {
        Ty::Sum { .. } => {}
        _ => return Err(Reject::decline("value-cmp: not a sum type")),
    };
    let enum_ty = crate::backend::rust::types::rust_type(&db.name_ctx(), ty)
        .ok_or_else(|| Reject::decline("value-cmp: no rust type for the sum"))?;
    // The helper fn name is mangled by the FULL INSTANTIATED type (via `rust_type`), not the bare sum name:
    // a nested `(Option (Option Int64))` needs a distinct `fn __cmp_*` for the outer `Option<Option<i64>>`
    // and the inner `Option<i64>` (different signatures) — a bare `__cmp_Option` would collide (the dedup
    // guard would suppress the 2nd, leaving an ill-typed call). `cmp_helper_name` hashes `enum_ty`, so each
    // instantiation gets a unique, valid ident.
    let fn_name = cmp_helper_name(&enum_ty);
    // RECURSION GUARD (PR#890): a TRULY self-referential sum (`T = (Node (Tuple (Option Int64) T)) | (Leaf)`)
    // re-enters its OWN type and would inline-recurse UNBOUNDED in codegen (a compiler stack overflow); route
    // it through a helper `fn` (the recursion base) instead. The key is the FULL instantiated type, so a
    // NESTED distinct instantiation (`Option<Option<i64>>` containing `Option<i64>`) does NOT trip this — the
    // two are different types → the inner expands inline (finite), no false "recursive generic" decline (the
    // gap this closes). Only an identical decl+args re-entry (a real cycle in the finite value) routes to the
    // helper by call-indirection.
    if seen.contains(&sum_ty) {
        return Ok(format!("{fn_name}(&{l}, &{r})"));
    }
    let decl_occ = match &sum_ty {
        Ty::Sum { decl, .. } => *decl,
        _ => unreachable!("checked Ty::Sum above"),
    };
    seen.push(sum_ty.clone());
    let variant_count = match db.type_decl_by_occ(decl_occ).map(|t| t.variants.len()) {
        Some(n) => n,
        None => {
            seen.pop();
            return Err(Reject::decline("value-cmp: no variant count"));
        }
    };
    // Build the declared-order compare: compare the discriminant-ORDINAL first (declared position), and on an
    // equal-variant pair compare payloads. `__ord` maps a ref to its DECLARED position; the same-variant arms
    // compare payloads; the fallthrough compares ordinals.
    let mut ord_arms = Vec::with_capacity(variant_count);
    let mut same_arms = Vec::with_capacity(variant_count + 1);
    let mut arm_err: Option<Reject> = None;
    for disc in 0..variant_count as u32 {
        let path = match sum_variant_path_of_ty(db, ty, disc) {
            Ok(p) => p,
            Err(e) => {
                arm_err = Some(e);
                break;
            }
        };
        let has_payload = variant_payload_ty(db, ty, disc).is_some();
        let ord_pat = if has_payload {
            format!("{path}(..)")
        } else {
            path.clone()
        };
        ord_arms.push(format!("{ord_pat} => {disc}u32,"));
        match variant_payload_ty(db, ty, disc) {
            None => same_arms.push(format!("({path}, {path}) => core::cmp::Ordering::Equal,")),
            Some(payload_ty) => {
                let deref = if crate::backend::rust::enums::variant_is_recursive(db, ty, disc) {
                    "**"
                } else {
                    "*"
                };
                let lp = format!("({deref}__lp)");
                let rp = format!("({deref}__rp)");
                match emit_value_cmp_walk_seen(db, &payload_ty, &lp, &rp, seen, helpers) {
                    Ok(cmp) => same_arms.push(format!("({path}(__lp), {path}(__rp)) => {cmp},")),
                    Err(e) => {
                        arm_err = Some(e);
                        break;
                    }
                }
            }
        }
    }
    seen.pop();
    if let Some(e) = arm_err {
        return Err(e);
    }
    // Emit the helper `fn __cmp_<Ident>(l, r) -> Ordering` ONCE (call-indirection, so a recursive payload
    // reaches this decl via a CALL, terminating codegen), then return a call. Mirrors `emit_value_eq_walk`'s
    // `__eq_<Ident>` helper. `#[allow]` for the generated fn's lints. Guard against a duplicate emit if the
    // same decl's helper was already pushed (a sibling occurrence in the same walk).
    let helper = format!(
        "#[allow(clippy::all)] fn {fn_name}(__cl: &{enum_ty}, __cr: &{enum_ty}) -> core::cmp::Ordering {{ let __ord = |__v: &{enum_ty}| -> u32 {{ match __v {{ {} }} }}; match (__cl, __cr) {{ {} _ => __ord(__cl).cmp(&__ord(__cr)), }} }}",
        ord_arms.join(" "),
        same_arms.join(" "),
    );
    if !helpers
        .iter()
        .any(|h| h.contains(&format!("fn {fn_name}(")))
    {
        helpers.push(helper);
    }
    Ok(format!("{fn_name}(&{l}, &{r})"))
}

/// The `fn __cmp_*` helper name for a sum's value-cmp walk, mangled by its FULL rendered Rust type
/// (`enum_ty`, e.g. `Option<Option<i64>>`) rather than the bare sum name. Two DISTINCT instantiations of one
/// generic sum (`Option<Option<i64>>` vs `Option<i64>` in a nested compare) need DISTINCT helpers — same
/// bare name + different signatures would collide (the dedup guard suppresses the 2nd → an ill-typed call).
/// Hex-encoding the rendered type gives a unique, valid, collision-free ident (the same injective hex idiom
/// `types::sum_ident` uses for lossy names); the leading `__cmp_` keeps it in the generated-helper namespace
/// (user idents with a leading `__` are escaped by `sanitize_ident`, so no clash with a user fn).
pub(super) fn cmp_helper_name(enum_ty: &str) -> String {
    let mut s = String::with_capacity(enum_ty.len() * 2 + 6);
    s.push_str("__cmp_");
    for b in enum_ty.bytes() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
