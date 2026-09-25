//! Bare-entry-param FIELD WIT derivation + rebuild (peeled from `mod.rs`, a pure code move — the file-size
//! mandate split). `natural_wit_bare`/`sum_field_wit` derive a record/tuple entry param's structural WIT
//! (recovering an option/result FIELD's WIT that Db-less `ty_natural_wit` refuses); `structuralize_wit`
//! rewrites nominal `record<…>` to the structural `tuple<…>` the bare assembler can declare;
//! `param_field_rebuild`/`record_fields_rebuild` build the per-field value-heap-cell rebuild. Called only
//! from `mod.rs`'s `try_bare_entry_param_component` (hence `pub(super)`).

use super::*;

/// Recursively rewrite a WIT type so no nominal `record<…>` remains: each `record` becomes the STRUCTURAL
/// `tuple<…>` of its field types (in the record's declaration order, which for a bare entry param is
/// `ty_natural_wit`'s sorted order — the same order the value-heap cell stores the fields), recursing through
/// every compound former so a record nested at any depth is flattened. This is BARE-PATH ONLY: the bare
/// assembler (`assemble_bare_typed_with_runtime`) cannot declare a nominal `record<…>` defined type, so a
/// nominal record (top-level OR nested inside a crossing tuple/record) emits an INVALID component (CDZ0910);
/// the structural tuple carries the byte-identical wire and needs no declaration. The TYPED interface path
/// keeps nominal records (declarable there) and MUST NOT call this.
pub(super) fn structuralize_wit(wt: &crate::wit_world::WitType) -> crate::wit_world::WitType {
    use crate::wit_world::WitType;
    match wt {
        WitType::Record(fields) => {
            WitType::Tuple(fields.iter().map(|(_n, t)| structuralize_wit(t)).collect())
        }
        WitType::Tuple(ts) => WitType::Tuple(ts.iter().map(structuralize_wit).collect()),
        WitType::List(inner) => WitType::List(Box::new(structuralize_wit(inner))),
        WitType::Option(inner) => WitType::Option(Box::new(structuralize_wit(inner))),
        WitType::Result { ok, err } => WitType::Result {
            ok: ok.as_ref().map(|t| Box::new(structuralize_wit(t))),
            err: err.as_ref().map(|t| Box::new(structuralize_wit(t))),
        },
        WitType::Variant(cases) => WitType::Variant(
            cases
                .iter()
                .map(|(n, p)| (n.clone(), p.as_ref().map(structuralize_wit)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The STRUCTURAL WIT a two-variant / liftable-variant sum FIELD crosses as on the bare entry path — the
/// field-WIT counterpart to the top-level sum-param arms' WIT synthesis. [`crate::wit_world::ty_natural_wit`]
/// is Db-less and returns `None` for a `Ty::Sum` (a top-level sum crosses via its own synthesized-WIT arm, not
/// that deriver), so a record/tuple entry param with an option/result FIELD used to bail there and decline
/// (CDZ0904); this recovers the field's WIT so the product crosses. It MUST agree with
/// [`param_field_rebuild`]'s `Ty::Sum` flattening: an all-scalar `result<ok,err>` (two single-scalar payloads)
/// crosses as the STRUCTURAL `result<ok,err>` of its two payload naturals — NOT the `variant`
/// [`crate::backend::wasm::host::spilled_result_wit_type`] would mint (the erp2 disagreement that emitted an
/// INVALID component) — and every other admitted sum (`option<…>`, `result<list<u8>,enum>`, a general liftable
/// variant) takes `spilled_result_wit_type`'s structural former. Returns `None` for a sum shape the rebuild
/// does not admit; the caller then declines the whole param.
pub(super) fn sum_field_wit(db: &mut Db, gty: &crate::ty::Ty) -> Option<crate::wit_world::WitType> {
    use crate::wit_world::WitType;
    // A TWO-PAYLOAD Result-shaped sum `(Ok a)(Err b)` FIELD crosses as a STRUCTURAL `result<ok,err>` — its two
    // payload naturals — matching `fixed_shape_sum_param_arg`'s `[1,1]` rebuild (a leading disc then the
    // position-wise JOIN of the arms' leaves, Ok=boundary-disc 0 / Err=1). This MUST agree with that rebuild
    // for EVERY liftable arm — a scalar, a byte-leaf `String`/`Bytes`, or a fixed-shape compound — NOT the
    // `variant<…>` `spilled_result_wit_type` would mint: a `variant` WIT disagrees with the result-shaped
    // flattening the rebuild emits, so an admitted `result<s64, string>` field emitted an INVALID component
    // (the erp2-class disagreement). Derived here from the decl (exactly two variants, each one payload) so a
    // byte-leaf arm takes this path too, not the scalar-only `ArgSlot::Result` shortcut. Falls through to
    // `spilled_result_wit_type` when a payload has no natural WIT (e.g. `result<list<u8>, enum>`, whose enum
    // err arm `spilled` mints as an `enum`), and for the Option / general-variant shapes.
    if let crate::ty::Ty::Sum { decl, args, .. } = gty.strip_nominal() {
        let counts: Vec<usize> = {
            let dr = db.type_decl_by_occ(*decl)?;
            dr.variants.iter().map(|v| v.payloads.len()).collect()
        };
        if counts == [1, 1]
            && args.len() == 2
            && let (Some(ok), Some(err)) = (
                crate::wit_world::ty_natural_wit(&args[0]),
                crate::wit_world::ty_natural_wit(&args[1]),
            )
        {
            return Some(structuralize_wit(&WitType::Result {
                ok: Some(Box::new(ok)),
                err: Some(Box::new(err)),
            }));
        }
    }
    // Every other admitted sum (option<…>, result<list<u8>,enum>, a general liftable variant) crosses as the
    // structural former `spilled_result_wit_type` mints Db-awarely.
    crate::backend::wasm::host::spilled_result_wit_type(db, gty)
}

/// [`crate::wit_world::ty_natural_wit`] extended for the BARE entry-param path: BYTE-IDENTICAL to it for every
/// type it already handles, but a `Ty::Sum` FIELD nested in a record/tuple param derives its structural WIT
/// ([`sum_field_wit`]) instead of bailing. `ty_natural_wit` returns `None` for a `Ty::Sum`, so a record/tuple
/// whose field is an option/result used to decline (CDZ0904); this recovers the field WIT so the product
/// crosses. It does NOT change `ty_natural_wit`'s global `Sum = None` contract — a TOP-LEVEL sum reaching the
/// caller's `ty_natural_wit` site still declines at the product-type match below (a bare `Ty::Sum` param is not
/// a record/tuple/scalar/mem-leaf), so this only ever activates sum handling for a record/tuple FIELD. The
/// Record arm keeps `ty_natural_wit`'s raw `name.to_string()` field name + sorted (`BTreeMap`) order so the
/// bare-record arm's positional zip and the nested-record name-match resolve identically.
pub(super) fn natural_wit_bare(
    db: &mut Db,
    gty: &crate::ty::Ty,
) -> Option<crate::wit_world::WitType> {
    use crate::ty::Ty;
    use crate::wit_world::WitType;
    match gty.strip_nominal() {
        Ty::Record(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            for (name, fty) in fields.iter() {
                out.push((name.name.to_string(), natural_wit_bare(db, fty)?));
            }
            Some(WitType::Record(out))
        }
        Ty::Tuple(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                out.push(natural_wit_bare(db, e)?);
            }
            Some(WitType::Tuple(out))
        }
        Ty::Sum { .. } => sum_field_wit(db, gty),
        // Recurse through `natural_wit_bare` (not `ty_natural_wit`) for a list ELEMENT so an option/result
        // element derives its structural WIT — `list<option<s64>>` crosses as `list<option<s64>>` rather than
        // declining (`ty_natural_wit`'s `Sum = None`). BYTE-IDENTICAL for every non-sum element (a scalar/
        // record/tuple element's `natural_wit_bare` equals its `ty_natural_wit`), so this is purely additive.
        Ty::List(elem) => Some(WitType::List(Box::new(natural_wit_bare(db, elem)?))),
        other => crate::wit_world::ty_natural_wit(other),
    }
}

/// Build the [`serialize::FieldRebuild`] for ONE record field (recursively), appending its flattened core
/// valtypes to `param_vts` in field order. A scalar boxes one flattened leaf; a `list<u8>`/`Bytes` leaf
/// crosses as `(ptr, len)` and copies out of memory (`BytesLeaf`, two i32); a FLAT `list<scalar>` field
/// likewise crosses as `(ptr, len)` and lifts into a value-heap vec (`ListLeaf`, rpp3); a NESTED record builds
/// a `Nested` rebuild over its own fields (the message's `sender` shape). Declines any other compound field
/// (a nested `list<list<…>>` field, a `Map`/`Set` field — later slices).
pub(super) fn param_field_rebuild(
    db: &mut Db,
    gty: &crate::ty::Ty,
    wty: &crate::wit_world::WitType,
    param_vts: &mut Vec<u8>,
) -> Option<crate::backend::wasm::serialize::FieldRebuild> {
    use crate::backend::wasm::lir::{ValType, valtype_of};
    use crate::backend::wasm::serialize::FieldRebuild;
    use crate::ty::Ty;
    use crate::wit_world::WitType;
    match gty.strip_nominal() {
        // A `String` or `Bytes` field both cross as `(ptr: i32, len: i32)` and lift via the SAME byte-leaf
        // copy-in (`FieldRebuild::BytesLeaf`): a Cadenza `String` value IS a flat UTF-8 byte-leaf, built
        // exactly by the `bytes-alloc`/`bytes-set` loop, so the copied buffer is already a canonical String
        // handle — no `str-from-bytes` decode (a WIT `string` field is guaranteed valid UTF-8). Only the
        // field's WIT type differs (`string` vs `list<u8>`), synthesized from the guest field type by
        // `ty_natural_wit` at the routing site. This mirrors the top-level `MemLeafKind::Str | Bytes` lift.
        Ty::String | Ty::Bytes => {
            param_vts.push(ValType::I32.byte());
            param_vts.push(ValType::I32.byte());
            Some(FieldRebuild::BytesLeaf)
        }
        Ty::Record(map) => {
            let WitType::Record(wfs) = wty else {
                return None;
            };
            let (fields, slots) = record_fields_rebuild(db, map, wfs, param_vts)?;
            Some(FieldRebuild::Nested(fields, slots))
        }
        // A variant/result PARAM field (the response's `answer: result<…>`): the canon lift hands the
        // flattened `(disc, payload…)`; the wrapper reads them and rebuilds the guest sum cell. First reuse the
        // closure-arg `fixed_shape_option_scalar_arg` — a two-variant Option/Result whose payload arms are
        // scalar or a fixed-shape tuple/record; then `fixed_shape_sum_param_arg` for the `list<u8>`/enum payload
        // arms (`result<list<u8>, error>` — the reducer response's `answer`).
        Ty::Sum { .. } => {
            // The canon lift flattens a variant to `(disc: i32, payload-join…)`. `fixed_shape_option_scalar_arg`
            // returns only the PAYLOAD valtypes (the closure convention passes the disc separately), so prepend
            // the disc's i32 — matching `flattened_param_count` (1 disc + the payload leaves) + `emit_sum_field`
            // (disc at the cursor, payload at cursor+1).
            if let Some((_slot, vts, rebuild)) = fixed_shape_option_scalar_arg(db, gty) {
                param_vts.push(ValType::I32.byte());
                param_vts.extend(vts.iter().map(|vt| vt.byte()));
                return Some(FieldRebuild::Sum(Box::new(rebuild)));
            }
            // `list<u8>` (Bytes) / all-nullary enum payload arms — appends the disc + join vts itself.
            fixed_shape_sum_param_arg(db, gty, param_vts)
        }
        // A `list<scalar>` field (rpp3's `xs: list<s64>`): crosses as `(ptr, len)` and lifts into a value-heap
        // vec (`FieldRebuild::ListLeaf`), mirroring the top-level `MemLeafKind::List` param lift. Only a FLAT
        // list is admitted — `list_scalar_elem` returns the scalar element's read/box descriptor + its
        // `nest_lists`, which must be 0; a nested `list<list<…>>` field is a later slice. A SUM element
        // (`list<option<scalar>>`, rpp23) falls to `list_sum_elem` when `list_scalar_elem` declines it — the
        // per-element `emit_list_level` sum branch reads the disc+payload straight from the element address and
        // builds the guest sum cell with NO fresh locals (like the compound branch, unlike the byte-leaf), so it
        // lifts through the same throwaway-`next_local` field cell-lift the top-level `list<option<scalar>>`
        // param (lpo1) uses.
        Ty::List(elem) => {
            let le = list_scalar_elem(elem).or_else(|| list_sum_elem(db, elem))?;
            if le.nest_lists != 0 {
                return None;
            }
            // A byte-leaf element list (`list<string>`/`list<bytes>`) is admitted only as a TOP-LEVEL param
            // (its per-element byte copy-in needs fresh scratch locals the top-level lift reserves); as a
            // nested FIELD the cell-rebuild lift threads a throwaway `next_local`, so decline here — a later
            // slice.
            if le.byte_leaf.is_some() {
                return None;
            }
            // A compound-element list (`list<tuple>`/`list<record>`) as a nested FIELD lifts through the SAME
            // `emit_list_leaf_lift` → `emit_list_level` compound branch as a top-level `list<tuple>` param: the
            // per-element cell build allocates no fresh locals (it threads the wrapper's `(buf, ctr)` scratch
            // pair the `ListLeaf` copy-in already reserves), so no extra scratch is needed here.
            param_vts.push(ValType::I32.byte());
            param_vts.push(ValType::I32.byte());
            Some(FieldRebuild::ListLeaf(le))
        }
        // A nested `Tuple` field crosses as a STRUCTURAL `tuple<…>` (no defined-type declaration) and rebuilds a
        // positional sub-cell, exactly like the top-level tuple entry-param arm but as a `FieldRebuild::Nested`.
        // Each element recurses through `param_field_rebuild` (positional → identity slots). A `Record` element
        // recurses into the `Ty::Record` arm (a structure-driven name-lex cell rebuild); the top-level arm's
        // `structuralize_wit` rewrites the emitted WIT so the nested `record<…>` crosses as the STRUCTURAL
        // `tuple<…>` the bare assembler can declare (was CDZ0910; #9717), at any tuple nesting depth. The
        // rebuild reads the record fields in the same name-lex order `structuralize_wit` emits, so the wire is
        // byte-identical.
        Ty::Tuple(gtys) => {
            let WitType::Tuple(wtys) = wty else {
                return None;
            };
            if gtys.len() != wtys.len() {
                return None;
            }
            let mut sub = Vec::with_capacity(gtys.len());
            for (gt, wt) in gtys.iter().zip(wtys.iter()) {
                sub.push(param_field_rebuild(db, gt, wt, param_vts)?);
            }
            let slots: Vec<u32> = (0..gtys.len() as u32).collect();
            Some(FieldRebuild::Nested(sub, slots))
        }
        _ => {
            let fr = scalar_field_rebuild(gty)?;
            param_vts.push(valtype_of(gty)?.byte());
            Some(fr)
        }
    }
}

/// Build a record's per-field rebuilds + name-lex SLOTS, PERMUTED BY NAME — the entry for both a top-level
/// record param and a nested-record field, at any WIT field order. Iterates the WIT fields IN WIT ORDER (so
/// `param_vts` + the wrapper's flattened-leaf cursor are the actual canon-lift param order), matching each
/// WIT field to its guest field BY NAME (kebab-normalized, the same rule as variant cases + exports).
/// Returns the rebuild (WIT order) + `slots` (each WIT field's name-lex cell position) — the wrapper
/// `arr-set`s each field at its slot. A WIT field with no name match declines. So a declaration-ordered
/// record like `message{contract, sender, payload, token}` or its nested `sender{reducer, host}` (name-lex
/// `host, reducer`) both cross correctly.
pub(super) fn record_fields_rebuild(
    db: &mut Db,
    map: &std::collections::BTreeMap<crate::resolved::Symbol, crate::ty::Ty>,
    wfs: &[(String, crate::wit_world::WitType)],
    param_vts: &mut Vec<u8>,
) -> Option<(Vec<crate::backend::wasm::serialize::FieldRebuild>, Vec<u32>)> {
    use crate::backend::common::export_name::kebab_extern_name;
    if wfs.len() != map.len() {
        return None;
    }
    let guest_kebab: Vec<String> = map
        .keys()
        .map(|s| kebab_extern_name(s.name.as_ref()))
        .collect();
    let gtys: Vec<crate::ty::Ty> = map.values().cloned().collect();
    let mut rebuild = Vec::new();
    let mut slots = Vec::new();
    for (wname, fw) in wfs {
        let slot = guest_kebab.iter().position(|g| g == wname)?; // NAME-MATCH
        let fg = gtys[slot].clone();
        rebuild.push(param_field_rebuild(db, &fg, fw, param_vts)?); // appends WIT-order vts
        slots.push(slot as u32);
    }
    Some((rebuild, slots))
}
