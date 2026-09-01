use super::*;

/// The recursive worker of [`collect_used_ops`] — descends every sub-position (both `if` branches, every
/// arm body — an op used only under a branch is still imported, since the branch may run). A box/get op
/// that would decline (a non-scalar element) is simply not added here; the decline surfaces at `emit`.
///
/// SHARING-AWARE (2026-08-16, v-core-opt-signed-off CLASS-A slice): a per-body `visited` set skips a
/// shared `StructId` already walked. The op accumulator is a `BTreeSet` — PRESENCE-only, idempotent under
/// revisits — so skipping a re-descent changes NO output while collapsing the exponential DAG re-walk (a
/// VALID self-host emit body reached 291M core-visits from re-walking shared nodes; cmb1 is unbounded).
/// This thin wrapper creates the `visited` set FRESH per top-level call so it never leaks across defs
/// (the reset discipline of `count_localref_reads` / the layout visited-sets). NOT applied to the class-B
/// dup/retain/consuming-payload walks: those feed per-occurrence dup/drop placement (multiplicity), where a
/// visited-set UNDER-counts → use-after-free (the reverted memo-sweep) — they stay Perceus-blocked.
pub(super) fn collect_used_ops_into(
    db: &mut Db,
    id: StructId,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    let mut visited: HashSet<StructId> = HashSet::new();
    collect_used_ops_into_seen(db, id, out, &mut visited);
}

/// The runtime ops the guest's record-host-arg field marshal calls for ONE field, recursing into a nested
/// record — mirrors [`emit_record_arg_marshal`] so `collect_used_ops` imports exactly what the emit calls
/// (a scalar field's get-op, a `Bytes` field's `bytes-len`/`bytes-get`, a nested record's `arr-get` + its
/// fields' ops). `arr-get` is inserted by the caller once for the whole record.
pub(super) fn collect_record_field_ops(
    db: &mut Db,
    fty: &Ty,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    match get_op_ty(db, fty) {
        Ok(Some(op)) => {
            out.insert(op);
        }
        _ if matches!(fty, Ty::Bytes) => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
        }
        _ if crate::backend::wasm::host::result_bytes_enum(db, fty).is_some() => {
            // A `result<list<u8>, enum>` field's branch-lower marshal: `sum-disc`/`sum-payload` (both arms)
            // + the Ok arm's `bytes-len`/`bytes-get` rope copy.
            out.insert(OP_SUM_DISC);
            out.insert(OP_SUM_PAYLOAD);
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
        }
        // A `list<T>` field (non-`Bytes`): the field marshal `arr-get`s the List handle then runs the list
        // marshal (`vec-len`/`vec-get` + the element's own ops) — declare exactly what it calls.
        _ if matches!(fty.strip_nominal(), Ty::List(_)) => {
            out.insert(OP_ARR_GET);
            out.insert(OP_VEC_LEN);
            out.insert(OP_VEC_GET);
            if let Ty::List(elem) = fty.strip_nominal() {
                let elem = (**elem).clone();
                collect_list_elem_ops(db, &elem, out);
            }
        }
        // A `tuple<…>` field (flattened inline): the marshal `arr-get`s the tuple handle, then per element a
        // SCALAR's unbox op OR a `Bytes` element's `bytes-len`/`bytes-get` rope copy.
        _ if matches!(fty.strip_nominal(), Ty::Tuple(_)) => {
            out.insert(OP_ARR_GET);
            if let Ty::Tuple(elems) = fty.strip_nominal() {
                let elems: Vec<Ty> = elems.iter().cloned().collect();
                for ety in &elems {
                    if matches!(ety.strip_nominal(), Ty::Bytes | Ty::String) {
                        out.insert(OP_BYTES_LEN);
                        out.insert(OP_BYTES_GET);
                    } else if let Ok(Some(read)) = get_op_ty(db, ety) {
                        out.insert(read);
                    }
                }
            }
        }
        // An `option<scalar|bytes>` field: the marshal `arr-get`s the Option, reads its `sum-disc`, and on the
        // Some arm `sum-payload` + the payload's ops (a scalar's unbox op, or a `Bytes` payload's
        // `bytes-len`/`bytes-get` rope copy) — declare exactly what it calls.
        _ if crate::backend::wasm::host::option_payload_ty(db, fty).is_some_and(|p| {
            valtype_of(&p).is_some() || matches!(p.strip_nominal(), Ty::Bytes | Ty::String)
        }) =>
        {
            out.insert(OP_ARR_GET);
            out.insert(OP_SUM_DISC);
            out.insert(OP_SUM_PAYLOAD);
            if let Some(payload) = crate::backend::wasm::host::option_payload_ty(db, fty) {
                if matches!(payload.strip_nominal(), Ty::Bytes | Ty::String) {
                    out.insert(OP_BYTES_LEN);
                    out.insert(OP_BYTES_GET);
                } else if let Ok(Some(read)) = get_op_ty(db, &payload) {
                    out.insert(read);
                }
            }
        }
        // A general `variant<scalar>` field: the marshal `arr-get`s the variant, reads `sum-disc`, and on a
        // payload case `sum-payload` + the payload scalar's unbox op.
        _ if crate::backend::wasm::host::variant_scalar_payload_cases(db, fty).is_some() => {
            out.insert(OP_ARR_GET);
            out.insert(OP_SUM_DISC);
            out.insert(OP_SUM_PAYLOAD);
            let first_payload_disc =
                crate::backend::wasm::host::variant_scalar_payload_cases(db, fty)
                    .and_then(|cs| cs.iter().position(|(_, p)| p.is_some()));
            if let Some(d) = first_payload_disc
                && let Some(pty) = variant_payload_ty_at(db, fty, d as u32)
                && let Ok(Some(read)) = get_op_ty(db, &pty)
            {
                out.insert(read);
            }
        }
        _ => {
            if let Ty::Record(sub) = fty {
                out.insert(OP_ARR_GET);
                for sfty in sub.values() {
                    collect_record_field_ops(db, sfty, out);
                }
            }
        }
    }
}

/// Collect the runtime ops `emit_list_arg_marshal` calls to lower a `list<T>` ELEMENT of type `elem` (mirrors
/// the marshal's element arms so the import section declares exactly what the body calls): a `Bytes`/`String`
/// element's rope copy (`bytes-len`/`bytes-get`), a NESTED list element's walk (`vec-len`/`vec-get`) recursed
/// on its inner element, or a SCALAR element's unbox get-op. The caller inserts the OUTER list's
/// `vec-len`/`vec-get` once. Kept in lockstep with `host::list_elem_marshalable` (the representability gate).
pub(super) fn collect_list_elem_ops(
    db: &mut Db,
    elem: &Ty,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    if matches!(elem.strip_nominal(), Ty::Bytes | Ty::String) {
        out.insert(OP_BYTES_LEN);
        out.insert(OP_BYTES_GET);
    } else if let Ty::List(inner) = elem.strip_nominal() {
        out.insert(OP_VEC_LEN);
        out.insert(OP_VEC_GET);
        let inner = (**inner).clone();
        collect_list_elem_ops(db, &inner, out);
    } else if let Ty::Record(fields) = elem.strip_nominal() {
        // A RECORD element (`emit_record_to_mem`): `arr-get` each field (borrow) + the field's own ops (a
        // scalar's unbox get-op, a `Bytes` field's `bytes-len`/`bytes-get` rope copy) — via the same
        // `collect_record_field_ops` the flatten marshal uses.
        out.insert(OP_ARR_GET);
        let fields = fields.clone();
        for fty in fields.values() {
            collect_record_field_ops(db, fty, out);
        }
    } else if let Ty::Tuple(elems) = elem.strip_nominal() {
        // A TUPLE element (`emit_tuple_to_mem`): `arr-get` each element (borrow) + its own ops — the same
        // `collect_record_field_ops` per element (a tuple is a positional product).
        out.insert(OP_ARR_GET);
        let elems: Vec<Ty> = elems.iter().cloned().collect();
        for ety in &elems {
            collect_record_field_ops(db, ety, out);
        }
    } else if let Some(payload) =
        crate::backend::wasm::host::option_payload_ty(db, elem).filter(|p| valtype_of(p).is_some())
    {
        // An OPTION<scalar> element (`emit_option_to_mem`): reads `sum-disc`, and on Some `sum-payload` + the
        // payload scalar's unbox op.
        out.insert(OP_SUM_DISC);
        out.insert(OP_SUM_PAYLOAD);
        if let Ok(Some(read)) = get_op_ty(db, &payload) {
            out.insert(read);
        }
    } else if let Some(cases) = crate::backend::wasm::host::variant_scalar_payload_cases(db, elem) {
        // A VARIANT<scalar> element (`emit_variant_to_mem`): reads `sum-disc`, and on a payload case
        // `sum-payload` + the payload scalar's unbox op (the N-case generalization of the option arm).
        out.insert(OP_SUM_DISC);
        out.insert(OP_SUM_PAYLOAD);
        if let Some(pd) = cases.iter().position(|(_, p)| p.is_some())
            && let Some(pty) = variant_payload_ty_at(db, elem, pd as u32)
            && let Ok(Some(read)) = get_op_ty(db, &pty)
        {
            out.insert(read);
        }
    } else if let Ok(Some(read)) = get_op_ty(db, elem) {
        out.insert(read);
    }
}

pub(super) fn collect_used_ops_into_seen(
    db: &mut Db,
    id: StructId,
    out: &mut std::collections::BTreeSet<&'static str>,
    visited: &mut HashSet<StructId>,
) {
    // Already walked this shared node → its ops are already in `out` (a set); skip the re-descent.
    if !visited.insert(id) {
        return;
    }
    match core_of(db, id) {
        Core::Tuple { elems } => {
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            for elem in elems.iter() {
                // A scalar element boxes (`box-int`/`box-bool`); a nested compound is already a handle
                // (`box_op` → `Ok(None)`), stored as-is. Recurse either way so a nested compound's own
                // construction ops (`arr-alloc`/`arr-set`/its element boxes) are collected.
                if let Ok(Some(op)) = box_op(db, *elem) {
                    out.insert(op);
                }
                // A rope-capable String/Bytes element is `bytes-compact`ed on construction (the emit
                // arm's nested-rope canonicalization), so import it.
                if elem_needs_rope_compaction(db, *elem) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into_seen(db, *elem, out, visited);
            }
        }
        Core::Proj { operand, .. } => {
            out.insert(OP_ARR_GET);
            let scalar_elem = get_op(db, id);
            if let Ok(Some(op)) = scalar_elem {
                out.insert(op);
            }
            // RECLAMATION (U13/U14): a projection off an OWNED-temporary aggregate reclaims it after the
            // borrowing read — mirror the emit's reclaim condition so the ops are imported. A SCALAR element
            // `drop`s the parent; a NESTED-COMPOUND element `dup`s the returned child then `drop`s the parent.
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
                if matches!(scalar_elem, Ok(None)) {
                    out.insert(OP_DUP);
                }
            }
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // A list construction is a BULK build: a flat `arr` (`arr-alloc` + a boxed `arr-set` per element,
        // like a tuple) then one `vec-of-arr`. So it imports the arr ops + `vec-of-arr`, not the old
        // `vec-empty`/`vec-push` chain.
        Core::ListNew { elems } => {
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            out.insert(OP_VEC_OF_ARR);
            for elem in elems.iter() {
                if let Ok(Some(op)) = box_op(db, *elem) {
                    out.insert(op);
                }
                if elem_needs_rope_compaction(db, *elem) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into_seen(db, *elem, out, visited);
            }
        }
        // `List.len` uses `vec-len` and evaluates its operand.
        Core::ListLen { operand } => {
            out.insert(OP_VEC_LEN);
            // RECLAMATION: a `vec-len` over an OWNED-temporary list drops it after the borrowing read
            // (mirror the emit's reclaim condition so `drop` is imported). A borrowed param/local is not
            // dropped (its owner reclaims).
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `Bytes.of` uses `bytes-alloc` + a `bytes-set` per element (each element is a raw byte — an
        // i32 in `0..=255`, NOT boxed to a handle, unlike a list element). Evaluate each element.
        Core::BytesOf { elems } => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            for elem in elems.iter() {
                collect_used_ops_into_seen(db, *elem, out, visited);
            }
        }
        // A baked byte-constant materializes with the SAME `bytes-alloc`+`bytes-set` shape as `BytesOf`,
        // but has no child nodes to descend into (the bytes are known constants).
        Core::ConstBytes(_) => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
        }
        // A runtime `(bin …)` build allocs the byte buffer + writes each segment byte with `bytes-set`.
        Core::BinBuild { segs } => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            for s in &segs {
                collect_used_ops_into_seen(db, s.value, out, visited);
            }
        }
        // A runtime bit-field run allocs the buffer + writes each packed byte with `bytes-set`.
        Core::BinBitsBuild { fields } => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            for f in &fields {
                collect_used_ops_into_seen(db, f.value, out, visited);
            }
        }
        // A `BinIntRead` reads its segment bytes with `bytes-get`. A §4a `off_plus` (a scalar `BinIntRead`)
        // brings its own ops in via the recurse.
        Core::BinIntRead {
            bytes, off_plus, ..
        } => {
            out.insert(OP_BYTES_GET);
            collect_used_ops_into_seen(db, bytes, out, visited);
            if let Some(op) = off_plus {
                collect_used_ops_into_seen(db, op, out, visited);
            }
        }
        // A `BinRestRead` slices the tail: `dup` the shared scrutinee, then `bytes-slice(bytes, off,
        // bytes-len - off)` on the copy. A §4a `off_plus` brings its own ops in via the recurse.
        Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            out.insert(OP_DUP);
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_SLICE);
            collect_used_ops_into_seen(db, bytes, out, visited);
            if let Some(op) = off_plus {
                collect_used_ops_into_seen(db, op, out, visited);
            }
        }
        // A `BinSizedRead` slices exactly `len` bytes at a static offset: `dup` the shared scrutinee, then
        // `bytes-slice(bytes, off, len)` on the copy. `len` is a runtime `BinIntRead` (its own `bytes-get`
        // + operand come in via the recurse), so no `bytes-len` here (unlike the rest read). A §4a `off_plus`
        // brings its own ops in via the recurse too.
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            out.insert(OP_DUP);
            out.insert(OP_BYTES_SLICE);
            collect_used_ops_into_seen(db, bytes, out, visited);
            collect_used_ops_into_seen(db, len, out, visited);
            if let Some(op) = off_plus {
                collect_used_ops_into_seen(db, op, out, visited);
            }
        }
        // `Bytes.len` uses `bytes-len` and evaluates its operand.
        Core::BytesLen { operand } => {
            out.insert(OP_BYTES_LEN);
            // RECLAMATION: a `bytes-len` over an OWNED-temporary bytes drops it after the borrow (mirror the
            // emit); a borrowed param/local is not dropped (its owner reclaims).
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `String.scalar-len` walks the UTF-8 byte leaf counting lead bytes — `bytes-len` (the loop bound)
        // + `bytes-get` (the per-byte read), the same borrowing reads `Core::StrAt`'s scalar-scan uses. Same
        // OWNED-temporary reclamation as `BytesLen`.
        Core::StrScalarLen { operand } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `List.push` uses `vec-push` (the pushed element boxed by its type); `List.concat` uses `vec-concat`.
        Core::ListPush { list, elem } => {
            out.insert(OP_VEC_PUSH);
            if let Ok(Some(op)) = box_op(db, elem) {
                out.insert(op);
            }
            collect_used_ops_into_seen(db, list, out, visited);
            collect_used_ops_into_seen(db, elem, out, visited);
        }
        // `List.prepend` uses `vec-prepend` (the prepended element boxed by its type, like a push).
        Core::ListPrepend { list, elem } => {
            out.insert(OP_VEC_PREPEND);
            if let Ok(Some(op)) = box_op(db, elem) {
                out.insert(op);
            }
            collect_used_ops_into_seen(db, list, out, visited);
            collect_used_ops_into_seen(db, elem, out, visited);
        }
        Core::ListConcat { lhs, rhs } => {
            out.insert(OP_VEC_CONCAT);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // `Map.merge` uses the `map-merge` op; recurse into both map operands.
        Core::MapMerge { lhs, rhs } => {
            out.insert(OP_MAP_MERGE);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // `List.update` uses `vec-update` (the replacement element boxed by its type, like a push).
        Core::ListUpdate { list, index, elem } => {
            out.insert(OP_VEC_UPDATE);
            if let Ok(Some(op)) = box_op(db, elem) {
                out.insert(op);
            }
            collect_used_ops_into_seen(db, list, out, visited);
            collect_used_ops_into_seen(db, index, out, visited);
            collect_used_ops_into_seen(db, elem, out, visited);
        }
        // A RUNTIME `List.at` reads the length (`vec-len`) for the bounds test and, in bounds, the
        // element (`vec-get`, which BORROWS → `dup` before the `Some` consumes it), then builds
        // `Some`/`None` (`sum-new`, with `arr-alloc(0)` for `None`'s unit payload). The element stays
        // BOXED (the handle `vec-get` returns feeds `sum-new` directly; a downstream match unboxes it),
        // so no `box-*`/`get-*` here — mirrors the `emit` arm's op choices exactly.
        Core::ListAt { list, index, .. } => {
            // Some(elem) via `vec-get` (dup'd) + `sum-new`; None from the inline-unit constant
            // (`IMM_UNIT`), NOT an allocation — so no `arr-alloc` (see the emit arm; PR #404 class).
            out.insert(OP_VEC_LEN);
            out.insert(OP_VEC_GET);
            out.insert(OP_DUP);
            out.insert(OP_SUM_NEW);
            // RECLAMATION: an OWNED-temporary list operand is dropped after the borrowing len/get (the emit
            // reclaims it). This collect pass has no `slots` to test `reusable_handle_slot`, so import `drop`
            // whenever the operand's ownership is Owned — a superset of the emit's condition (a declared-but-
            // unused import is harmless; the emit only actually emits the drop for a non-reused owned list).
            if matches!(heap_operand_ownership(db, list), Ok(HandleOwnership::Owned)) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, list, out, visited);
            collect_used_ops_into_seen(db, index, out, visited);
        }
        // A map construction is `map-empty` then a `map-insert` per entry (each key/value boxed by its
        // type). Mirrors the emit arm's op choices.
        Core::MapNew {
            entries,
            key_ty,
            val_ty,
        } => {
            out.insert(OP_MAP_EMPTY);
            if !entries.is_empty() {
                out.insert(OP_MAP_INSERT);
            }
            for (k, v) in entries.iter() {
                // NODE-AWARE box op (mirror the emit's `box_op_for`, not `box_op_ty`) — a Float key/value into
                // an empty (`Var`-typed) map must import the `box-float` the emit calls, not the `box-int`
                // `box_op_ty` defaults an unresolved `Var` to (the empty-collection float-element gap).
                if let Ok(Some(op)) = box_op_for(db, *k, &key_ty) {
                    out.insert(op);
                }
                if let Ok(Some(op)) = box_op_for(db, *v, &val_ty) {
                    out.insert(op);
                }
                if key_needs_compaction(db, *k) {
                    out.insert(OP_BYTES_COMPACT);
                }
                if key_needs_canonicalize(db, *k) {
                    out.insert(OP_VALUE_CANONICALIZE);
                    out.insert(OP_BYTES_ALLOC); // descriptor bake
                    out.insert(OP_BYTES_SET);
                }
                collect_used_ops_into_seen(db, *k, out, visited);
                collect_used_ops_into_seen(db, *v, out, visited);
            }
        }
        // `Map.insert` = `map-insert`, boxing the key and value by their types (NODE-AWARE `box_op_for` so a
        // Float key/value into an empty `Var`-typed map imports the `box-float` the emit calls — see `MapNew`).
        Core::MapInsert {
            map,
            key,
            val,
            key_ty,
            val_ty,
        } => {
            out.insert(OP_MAP_INSERT);
            if let Ok(Some(op)) = box_op_for(db, key, &key_ty) {
                out.insert(op);
            }
            if let Ok(Some(op)) = box_op_for(db, val, &val_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, key) {
                out.insert(OP_BYTES_COMPACT);
            }
            if key_needs_canonicalize(db, key) {
                out.insert(OP_VALUE_CANONICALIZE);
                out.insert(OP_BYTES_ALLOC); // descriptor bake
                out.insert(OP_BYTES_SET);
            }
            collect_used_ops_into_seen(db, map, out, visited);
            collect_used_ops_into_seen(db, key, out, visited);
            collect_used_ops_into_seen(db, val, out, visited);
        }
        // A RUNTIME `Map.lookup`: box the key, `map-lookup` (→ the stored value handle, or NULL when
        // absent), then build `Some(value)` / `None` (`sum-new`, `arr-alloc(0)` for None's unit). The
        // stored value is a BOXED handle (like a list element), used DIRECTLY as the `Some` payload —
        // `dup`'d so the map keeps its own reference (mirrors `ListAt`) — no unbox. The boxed key is an
        // owned temporary the emit `drop`s after the borrow-lookup.
        Core::MapLookup {
            map, key, key_ty, ..
        } => {
            // Some(value) via `map-lookup` (dup'd) + `sum-new`; None from the inline-unit constant
            // (`IMM_UNIT`), NOT an allocation — so no `arr-alloc` (see the emit arm; PR #404 class).
            out.insert(OP_MAP_LOOKUP);
            out.insert(OP_DUP);
            out.insert(OP_DROP);
            out.insert(OP_SUM_NEW);
            // NODE-AWARE box op for the looked-up KEY (mirror the emit) — a Float key into an empty `Var`-typed
            // map imports the `box-float` the emit calls, not `box_op_ty`'s `box-int` default.
            if let Ok(Some(op)) = box_op_for(db, key, &key_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, key) {
                out.insert(OP_BYTES_COMPACT);
            }
            if key_needs_canonicalize(db, key) {
                out.insert(OP_VALUE_CANONICALIZE);
                out.insert(OP_BYTES_ALLOC); // descriptor bake
                out.insert(OP_BYTES_SET);
            }
            collect_used_ops_into_seen(db, map, out, visited);
            collect_used_ops_into_seen(db, key, out, visited);
        }
        // `Map.remove` = `map-remove`, boxing the key by its type (NODE-AWARE `box_op_for` — see `MapInsert`).
        Core::MapRemove { map, key, key_ty } => {
            out.insert(OP_MAP_REMOVE);
            out.insert(OP_DROP);
            if let Ok(Some(op)) = box_op_for(db, key, &key_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, key) {
                out.insert(OP_BYTES_COMPACT);
            }
            if key_needs_canonicalize(db, key) {
                out.insert(OP_VALUE_CANONICALIZE);
                out.insert(OP_BYTES_ALLOC); // descriptor bake
                out.insert(OP_BYTES_SET);
            }
            collect_used_ops_into_seen(db, map, out, visited);
            collect_used_ops_into_seen(db, key, out, visited);
        }
        // `Map.size` = `map-size` (→ u32, extended to i64) — reads the map operand.
        Core::MapSize { map } => {
            out.insert(OP_MAP_SIZE);
            // RECLAMATION: a `map-size` over an OWNED-temporary map drops it after the borrow (mirror emit).
            if matches!(heap_operand_ownership(db, map), Ok(HandleOwnership::Owned)) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, map, out, visited);
        }
        // A set construction is `set-empty` then a `set-insert` per element (each boxed by its type).
        Core::SetOf { elems, elem_ty } => {
            out.insert(OP_SET_EMPTY);
            if !elems.is_empty() {
                out.insert(OP_SET_INSERT);
            }
            for &e in elems.iter() {
                // NODE-AWARE box op (mirror the emit's `box_op_for`, NOT `box_op_ty`): when `elem_ty` is an
                // unresolved `Var`/`Any` (an empty-base set fixed no element type), `box_op_ty` DEFAULTS to
                // `box-int`, but the emit's `box_op_for` falls back to the ELEMENT NODE's real type — a Float
                // element emits `box-float`. Collecting `box-int` while the emit calls `box-float` leaves
                // `box-float` un-imported → `call u32::MAX` → INVALID WASM (the empty-set float-element gap,
                // the historical empty-set String box-int bug's twin). Use the node so the two agree.
                if let Ok(Some(op)) = box_op_for(db, e, &elem_ty) {
                    out.insert(op);
                }
                if key_needs_compaction(db, e) {
                    out.insert(OP_BYTES_COMPACT);
                }
                if key_needs_canonicalize(db, e) {
                    out.insert(OP_VALUE_CANONICALIZE);
                    out.insert(OP_BYTES_ALLOC); // descriptor bake
                    out.insert(OP_BYTES_SET);
                }
                collect_used_ops_into_seen(db, e, out, visited);
            }
        }
        // `Set.contains` = `set-contains` (→ bool), boxing the element (an owned temporary the emit drops).
        Core::SetContains { set, elem, elem_ty } => {
            out.insert(OP_SET_CONTAINS);
            out.insert(OP_DROP);
            if let Ok(Some(op)) = box_op_for(db, elem, &elem_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, elem) {
                out.insert(OP_BYTES_COMPACT);
            }
            if key_needs_canonicalize(db, elem) {
                out.insert(OP_VALUE_CANONICALIZE);
                out.insert(OP_BYTES_ALLOC); // descriptor bake
                out.insert(OP_BYTES_SET);
            }
            collect_used_ops_into_seen(db, set, out, visited);
            collect_used_ops_into_seen(db, elem, out, visited);
        }
        // `Set.insert`/`Set.remove` = `set-insert`/`set-remove`, boxing the element by its type. NODE-AWARE
        // `box_op_for` (not `box_op_ty`) so a Float element into an empty (`Var`-typed) set imports the
        // `box-float` the emit calls, not the `box-int` `box_op_ty` defaults a `Var` to — see `SetOf`.
        Core::SetInsert { set, elem, elem_ty } => {
            out.insert(OP_SET_INSERT);
            if let Ok(Some(op)) = box_op_for(db, elem, &elem_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, elem) {
                out.insert(OP_BYTES_COMPACT);
            }
            if key_needs_canonicalize(db, elem) {
                out.insert(OP_VALUE_CANONICALIZE);
                out.insert(OP_BYTES_ALLOC); // descriptor bake
                out.insert(OP_BYTES_SET);
            }
            collect_used_ops_into_seen(db, set, out, visited);
            collect_used_ops_into_seen(db, elem, out, visited);
        }
        // `set-remove` BORROWS the element, so the emit drops an OWNED-TEMPORARY element after the borrow
        // (the ownership gate) — hence `drop`.
        Core::SetRemove { set, elem, elem_ty } => {
            out.insert(OP_SET_REMOVE);
            out.insert(OP_DROP);
            if let Ok(Some(op)) = box_op_for(db, elem, &elem_ty) {
                out.insert(op);
            }
            if key_needs_compaction(db, elem) {
                out.insert(OP_BYTES_COMPACT);
            }
            if key_needs_canonicalize(db, elem) {
                out.insert(OP_VALUE_CANONICALIZE);
                out.insert(OP_BYTES_ALLOC); // descriptor bake
                out.insert(OP_BYTES_SET);
            }
            collect_used_ops_into_seen(db, set, out, visited);
            collect_used_ops_into_seen(db, elem, out, visited);
        }
        // `Set.to-list` = `set-to-list` + the inline descriptor `Bytes` build (`bytes-alloc`/`bytes-set`).
        Core::SetToList { set, .. } => {
            out.insert(OP_SET_TO_LIST);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP); // the borrowed-only descriptor Bytes is dropped after the op
            collect_used_ops_into_seen(db, set, out, visited);
        }
        // `Map.to-list` = `map-to-list` + the inline descriptor `Bytes` build (`bytes-alloc`/`bytes-set`).
        Core::MapToList { map, .. } => {
            out.insert(OP_MAP_TO_LIST);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP); // the borrowed-only descriptor Bytes is dropped after the op
            collect_used_ops_into_seen(db, map, out, visited);
        }
        // `Set.len` = `set-size` (→ u32, extended to i64) — reads the set operand.
        Core::SetLen { set } => {
            out.insert(OP_SET_SIZE);
            // RECLAMATION: a `set-size` over an OWNED-temporary set drops it after the borrow (mirror emit).
            if matches!(heap_operand_ownership(db, set), Ok(HandleOwnership::Owned)) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, set, out, visited);
        }
        // A set-algebra op = the matching runtime op (consumes both operand sets).
        Core::SetAlgebra { op, lhs, rhs } => {
            out.insert(match op {
                crate::core::SetAlgebraOp::Union => OP_SET_UNION,
                crate::core::SetAlgebraOp::Intersection => OP_SET_INTERSECTION,
                crate::core::SetAlgebraOp::Difference => OP_SET_DIFFERENCE,
            });
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // A RUNTIME `Bytes.at`: `bytes-len` (bounds test) + `bytes-get` (the raw byte VALUE, in bounds),
        // then `box-int` the byte into the `Some` payload (`sum-new`), or `arr-alloc(0)` for `None`'s
        // unit payload. No `dup` — `bytes-get` returns a value, not a borrowed handle. Mirrors `emit`.
        Core::BytesAt { bytes, index, .. } => {
            // Some(byte) via `bytes-get` (box-int) + `sum-new`; None from the inline-unit constant
            // (`IMM_UNIT`), NOT an allocation — so no `arr-alloc` (see the emit arm; PR #404 class).
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            out.insert(OP_BOX_INT);
            out.insert(OP_SUM_NEW);
            // RECLAMATION: an OWNED-temporary bytes operand is dropped after the borrowing len/get (see the
            // emit + the `ListAt` collect arm — import `drop` for an Owned operand; harmless if the emit's
            // reused-slot condition suppresses the actual drop).
            if matches!(
                heap_operand_ownership(db, bytes),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, bytes, out, visited);
            collect_used_ops_into_seen(db, index, out, visited);
        }
        // `String.at` on a runtime string walks the UTF-8 buffer (`bytes-len`/`bytes-get`), slices the
        // scalar span (`bytes-slice`, which CONSUMES the string handle → the borrowed scan `dup`s first,
        // and the None branch `drop`s the un-consumed handle), and builds `Some`/`None` (`sum-new`,
        // `arr-alloc` for the unit payload).
        // `String.scalar-at` on a runtime string calls `bytes-scalar-at` (borrows the string — its owner
        // reclaims, like `StrAt`, so no `drop` is imported here), boxes the returned codepoint into a Char
        // scalar cell (`box-int`, #5252 rep), and builds `Some`/`None` (`sum-new`; the unit payload is inline).
        Core::StrScalarAt { operand, index, .. } => {
            out.insert(OP_BYTES_SCALAR_AT);
            out.insert(OP_BOX_INT);
            out.insert(OP_SUM_NEW);
            // An OWNED-temporary string operand is dropped after the borrow-read (see the emit); a borrowed
            // one is not (its owner reclaims) — so `drop` is imported only when the operand is owned.
            if matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, operand, out, visited);
            collect_used_ops_into_seen(db, index, out, visited);
        }
        Core::StrAt { string, index, .. } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            out.insert(OP_BYTES_SLICE);
            // The Some-branch DUPs the string (`dup`), the slice CONSUMES that dup'd copy, then COMPACTS
            // the fresh slice to an independent flat leaf (see the emit) so a `String.at` result's
            // content-equality / key-hashing compares by content, not rope offset. The original string is
            // NOT dropped here — its owner (an enclosing let/param) reclaims it (see the emit comment), so
            // no `drop` is imported (unlike Map.lookup/Set.contains, whose boxed KEY is an owned temporary
            // they must drop). None is the inline-unit constant (`IMM_UNIT`), NOT an allocation — no
            // `arr-alloc` either.
            out.insert(OP_BYTES_COMPACT);
            out.insert(OP_DUP);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into_seen(db, string, out, visited);
            collect_used_ops_into_seen(db, index, out, visited);
        }
        // `String.slice` walks the UTF-8 buffer to the start/end scalar byte positions (`bytes-len`/`bytes-
        // get`), slices that span (`bytes-slice`, which CONSUMES the string handle → the Some branch `dup`s
        // first), COMPACTS the fresh slice to an independent flat leaf (content-equality/key-hashing), and
        // builds `Some`/`None` (`sum-new`; `None` is the inline `IMM_UNIT`, no `arr-alloc`). Same op set as
        // `String.at`. The source string is NOT dropped here (its owner reclaims it), so no `drop` import.
        Core::StrSlice {
            string, start, end, ..
        } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            out.insert(OP_BYTES_SLICE);
            out.insert(OP_BYTES_COMPACT);
            out.insert(OP_DUP);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into_seen(db, string, out, visited);
            collect_used_ops_into_seen(db, start, out, visited);
            collect_used_ops_into_seen(db, end, out, visited);
        }
        // `Bytes.concat` = `bytes-concat`; `Bytes.compact` = `bytes-compact`; `Bytes.slice` bounds-checks
        // via `bytes-len` then builds `Some(bytes-slice)` (a Bytes HANDLE, no box) / `None` (`arr-alloc(0)`).
        Core::BytesConcat { lhs, rhs } => {
            out.insert(OP_BYTES_CONCAT);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // `bigint-of-i64` mints a leaf from an i64 scalar — no handle operand, no drop. An UNSIGNED source
        // instead materializes a sign-magnitude byte leaf (`bytes-alloc`/`bytes-set` → `bigint-of-bytes`),
        // so declare those imports to match the emit's signedness branch (see the `Core::BigIntOfI64` emit).
        Core::BigIntOfI64 { value } => {
            if int_ty_of(db, value).ground_signed() {
                out.insert(OP_BIGINT_OF_I64);
            } else {
                out.insert(OP_BYTES_ALLOC);
                out.insert(OP_BYTES_SET);
                out.insert(OP_BIGINT_OF_BYTES);
            }
            collect_used_ops_into_seen(db, value, out, visited);
        }
        // The borrowing BigInt ops also import `drop` (to reclaim an OWNED-temporary handle operand after
        // the borrowing call — see the `emit_bigint_borrow_*` helpers), plus `bigint-of-i64` when an
        // operand is a CONSTANT BigInt materialized inline (`const_bigint_materializes`).
        Core::BigIntToI64 { operand } => {
            out.insert(OP_BIGINT_TO_I64_CHECKED);
            out.insert(OP_DROP);
            insert_const_bigint_materialize_ops(db, operand, out);
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `Char.to-int` uses NO runtime op — it is a pure wasm `i64.extend_i32_u` of the i32 code-point
        // slot. Just descend into the operand.
        Core::CharToInt { operand } => collect_used_ops_into_seen(db, operand, out, visited),
        // `Char.from-int n` (runtime): the emit boxes the code point (box-int) into a `Some` and builds
        // `Some`/`None` via sum-new (None's unit payload is the inline-unit constant, no arr-alloc). Collect
        // those ops + recurse the operand.
        Core::IntToCharChecked { operand, .. } => {
            collect_used_ops_into_seen(db, operand, out, visited);
            out.insert(OP_BOX_INT);
            out.insert(OP_SUM_NEW);
        }
        Core::BigIntBinOp { op, lhs, rhs } => {
            out.insert(match op {
                crate::core::BigIntOp::Add => OP_BIGINT_ADD,
                crate::core::BigIntOp::Sub => OP_BIGINT_SUB,
                crate::core::BigIntOp::Mul => OP_BIGINT_MUL,
                crate::core::BigIntOp::Div => OP_BIGINT_DIV,
                crate::core::BigIntOp::Rem => OP_BIGINT_REM,
            });
            out.insert(OP_DROP);
            insert_const_bigint_materialize_ops(db, lhs, out);
            insert_const_bigint_materialize_ops(db, rhs, out);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // A BigInt comparison imports `bigint-cmp` (the three-way primitive) AND `drop` (to reclaim an
        // owned-temporary operand after the borrowing compare — the `emit_bigint_borrow_binary` helper),
        // plus the materialization ops for an inline-materialized constant operand.
        Core::BigIntCmp { lhs, rhs, .. } => {
            out.insert(OP_BIGINT_CMP);
            out.insert(OP_DROP);
            insert_const_bigint_materialize_ops(db, lhs, out);
            insert_const_bigint_materialize_ops(db, rhs, out);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // `Rational.of n d` on runtime ints — widen each to a BigInt (`bigint-of-i64`) then `rational-of`.
        Core::RationalOfInts { num, den } => {
            out.insert(OP_BIGINT_OF_I64);
            out.insert(OP_RATIONAL_OF);
            collect_used_ops_into_seen(db, num, out, visited);
            collect_used_ops_into_seen(db, den, out, visited);
        }
        // `Rational.of-int n` — widen `n` + the constant `1` to BigInt, then `rational-of`.
        Core::RationalOfIntWiden { value } => {
            out.insert(OP_BIGINT_OF_I64);
            out.insert(OP_RATIONAL_OF);
            collect_used_ops_into_seen(db, value, out, visited);
        }
        // `Rational.numerator`/`denominator` — `rational-num`/`rational-den` BORROW the operand (import
        // `drop` to reclaim an owned-temporary Rational after the borrowing read), returning a BigInt.
        // Split per-variant so the op is chosen by the arm that already knows it — no `core_of` re-lookup.
        Core::RationalNum { operand } => {
            out.insert(OP_RATIONAL_NUM);
            out.insert(OP_DROP);
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        Core::RationalDen { operand } => {
            out.insert(OP_RATIONAL_DEN);
            out.insert(OP_DROP);
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // The borrowing Rational arithmetic ops import their op + `drop` (reclaim an owned-temporary
        // operand after the borrowing call — the `emit_rational_borrow_binary` helper).
        Core::RationalBinOp { op, lhs, rhs } => {
            out.insert(match op {
                crate::core::RationalOp::Add => OP_RATIONAL_ADD,
                crate::core::RationalOp::Sub => OP_RATIONAL_SUB,
                crate::core::RationalOp::Mul => OP_RATIONAL_MUL,
                crate::core::RationalOp::Div => OP_RATIONAL_DIV,
            });
            out.insert(OP_DROP);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        Core::RationalCmp { lhs, rhs, .. } => {
            out.insert(OP_RATIONAL_CMP);
            out.insert(OP_DROP);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_SLICE);
            out.insert(OP_DROP); // the None branch drops the un-consumed bytes reference
            out.insert(OP_SUM_NEW);
            // None is the inline-unit constant (`IMM_UNIT`), NOT an allocation — no `arr-alloc` (see the
            // emit arm; PR #404 class).
            collect_used_ops_into_seen(db, bytes, out, visited);
            collect_used_ops_into_seen(db, start, out, visited);
            collect_used_ops_into_seen(db, len, out, visited);
        }
        Core::BytesCompact { operand } => {
            out.insert(OP_BYTES_COMPACT);
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `Ast.print` (runtime) calls the `ast-print` heap op (op 92) over its Ast operand. The emit also
        // bakes the disc descriptor into a fresh `Bytes` buffer (`bytes-alloc` + per-byte `bytes-set`), so
        // those ops must be imported too (mirrors `ValueEncode`'s desc bake).
        Core::AstPrint { operand, .. } => {
            out.insert(OP_AST_PRINT);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP); // reclaim the fresh discs buffer + an owned Ast operand (emit_ast_op_with_discs)
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `Ast.encode` (runtime) calls the `ast-encode` heap op (op 93) over its Ast operand. Like
        // `AstPrint`, it also bakes the disc descriptor into a fresh `Bytes` buffer (`bytes-alloc` +
        // per-byte `bytes-set`), so those ops must be imported too.
        Core::AstEncode { operand, .. } => {
            out.insert(OP_AST_ENCODE);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP); // reclaim the fresh discs buffer + an owned Ast operand (emit_ast_op_with_discs)
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `Ast.decode` (runtime): like `AstEncode` it bakes the disc descriptor (`bytes-alloc`+`bytes-set`)
        // and calls the op with a borrow-drop of the operand + discs buffer (`ast-decode`+`drop`); THEN it
        // wraps the returned handle-or-0 as `(Ok …)`/`(Err unit)` via `sum-new` (the unit payload is the
        // inline `IMM_UNIT` constant, no alloc — mirrors `StrFromBytes`).
        Core::AstDecode { operand, .. } => {
            out.insert(OP_AST_DECODE);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `Blake3.of` calls the `hash-blake3` heap op (op 91) over its Bytes operand.
        Core::Blake3Of { operand } => {
            out.insert(OP_HASH_BLAKE3);
            out.insert(OP_DROP); // reclaim an owned Bytes operand after the borrowing hash (Blake3Of emit)
            collect_used_ops_into_seen(db, operand, out, visited);
        }
        // `String.from-bytes` on a runtime Bytes: `str-from-bytes` (→ the String handle, or NULL when the
        // buffer is invalid UTF-8), then build `Some(handle)` / `None` via `sum-new`. The returned handle is
        // already OWNED (str-from-bytes consumes the buffer and transfers it out), so it is used DIRECTLY as
        // the `Some` payload — no `dup`. The None branch's unit payload is the inline `IMM_UNIT` CONSTANT
        // (matching the emit, exactly as `Map.lookup`/`Bytes.at` build their None), NOT an `arr-alloc(0)`
        // call — so `arr-alloc` must NOT be imported here (the emit never calls it; importing it declares a
        // DEAD runtime op for a program using only str-from-bytes + sum-new). The None branch has no handle
        // to drop (the runtime consumed the buffer on failure).
        Core::StrFromBytes { bytes, .. } => {
            // `str-from-bytes` decodes the buffer to a String handle or NULL; the emit then builds
            // `Some(handle)` / `None` via `sum-new`. The `None` payload is the INLINE-unit constant
            // (`IMM_UNIT`), not an allocated cell — so `sum-new` is the only heap op, no `arr-alloc`.
            // (An earlier version over-declared `OP_ARR_ALLOC` "for None's unit"; None uses no alloc, so
            // importing it forced an unnecessary runtime import — PR #404 Copilot review.)
            out.insert(OP_STR_FROM_BYTES);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into_seen(db, bytes, out, visited);
        }
        // `String.to-bytes` on a runtime String: `bytes-compact` flattens the string's byte-rope to a
        // canonical flat leaf (a String IS a UTF-8 Bytes leaf, so no conversion) and transfers it out as the
        // Bytes result — the total encoding needs no `sum-new`/validation, just the one flatten op.
        Core::StrToBytes { string } => {
            out.insert(OP_BYTES_COMPACT);
            collect_used_ops_into_seen(db, string, out, visited);
        }
        // `str-nfc-normalize` on a runtime String: one runtime op that canonicalizes to NFC. The collect
        // MUST mirror the emit arm's single `CallImport(OP_STR_NFC_NORMALIZE)` exactly (an import-set that
        // omits it, or adds an extra, shifts every import index → invalid module), then recurse into `string`.
        Core::NfcNormalize { string } => {
            out.insert(OP_STR_NFC_NORMALIZE);
            collect_used_ops_into_seen(db, string, out, visited);
        }
        Core::If { cond, then_, else_ } => {
            collect_used_ops_into_seen(db, cond, out, visited);
            collect_used_ops_into_seen(db, then_, out, visited);
            collect_used_ops_into_seen(db, else_, out, visited);
        }
        Core::Match { scrutinee, arms } => {
            collect_used_ops_into_seen(db, scrutinee, out, visited);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_used_ops_into_seen(db, g, out, visited);
                }
                collect_used_ops_into_seen(db, arm.body, out, visited);
            }
        }
        Core::Let { bindings, body } => {
            for (binder, value) in bindings.iter() {
                // A HEAP-typed binding is `drop`'d after the body (Perceus) — so the program imports
                // `drop`. (A scalar binding owns no heap cell → no drop, matching `emit`.) The `dup` a
                // consumed-then-reused binding needs is imported ONCE at the `collect_used_ops` entry
                // (over the whole body, covering params too), not per-binding here.
                // `_for_retain`: a still-`Var` binder that solves to heap needs its `drop` DECLARED (a
                // declared-but-unused import is harmless if it turns out scalar) — keeps the import set a
                // superset of what the retain-candidate broadening can emit.
                if is_heap_type_for_retain(&type_of(db, *binder)) {
                    out.insert(OP_DROP);
                }
                collect_used_ops_into_seen(db, *value, out, visited);
            }
            collect_used_ops_into_seen(db, body, out, visited);
        }
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => {
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // Runtime String/Symbol ordering walks both leaves with `bytes-len`/`bytes-get` and drops an owned-
        // temporary operand after the borrowing walk (see the `Core::StrCmp` emit). No `bytes-compact` — the
        // byte walk reads logical bytes through a rope transparently, so no pre-canonicalization is needed.
        Core::StrCmp { lhs, rhs, .. } => {
            out.insert(OP_BYTES_LEN);
            out.insert(OP_BYTES_GET);
            out.insert(OP_DROP);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // Runtime structural equality imports `value-eq` (the compare) AND `drop` (to reclaim an owned
        // temporary operand after the borrowing compare — see the `Core::ValueEq` emit). A STRING/BYTES
        // operand is canonicalized with `bytes-compact` before the compare (a rope vs its flat twin — see
        // the emit), so import `bytes-compact` when either operand is a String or Bytes.
        Core::ValueEq { lhs, rhs } => {
            out.insert(OP_VALUE_EQ);
            out.insert(OP_DROP);
            if operand_is_string_or_bytes(db, lhs) || operand_is_string_or_bytes(db, rhs) {
                out.insert(OP_BYTES_COMPACT);
            }
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // Runtime compound ORDERING imports `value-cmp` (the compare) + `drop` (reclaim the descriptor Bytes
        // AND an owned-temporary operand after the borrowing compare) + `bytes-alloc`/`bytes-set` (the emit
        // BAKES the shape descriptor inline as a Bytes constant, exactly like `Set.to-list`). Mirrors ValueEq's
        // borrow contract plus the descriptor-baking op set.
        Core::ValueCmp { lhs, rhs, .. } => {
            out.insert(OP_VALUE_CMP);
            out.insert(OP_DROP);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // Runtime descriptor-guided structural EQUALITY imports `value-eq-shaped` + `drop` (reclaim the
        // descriptor Bytes AND an owned-temporary operand after the borrowing compare) + `bytes-alloc`/
        // `bytes-set` (the emit BAKES the shape descriptor inline as a Bytes constant, exactly like ValueCmp).
        Core::ValueEqShaped { lhs, rhs, .. } => {
            out.insert(OP_VALUE_EQ_SHAPED);
            out.insert(OP_DROP);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            collect_used_ops_into_seen(db, lhs, out, visited);
            collect_used_ops_into_seen(db, rhs, out, visited);
        }
        // `Value.encode` (R2) imports `value-encode` (the render) + `bytes-alloc`/`bytes-set` (the emit BAKES
        // the shape descriptor inline as a Bytes constant, exactly like `ValueCmp`) + `drop` (reclaim the
        // borrowed-only descriptor Bytes — and an owned-temporary value operand — after the borrowing call).
        Core::ValueEncode { value, .. } => {
            out.insert(OP_VALUE_ENCODE);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP);
            // A scalar-erased operand (a single-field single-ctor newtype over a scalar) is BOXED to a leaf
            // handle first, so its box op (`box-int`/`box-bool`/`box-float`/`box-float32`) must be imported
            // too. `box_op_ty` returns `None` for an already-handle operand (no extra import needed).
            let vty = type_of(db, value);
            if let Ok(Some(op)) = box_op_ty(db, &vty) {
                out.insert(op);
            }
            collect_used_ops_into_seen(db, value, out, visited);
        }
        // `Value.decode` (R2) imports `value-decode` (the parse) + the descriptor-baking op set + `drop`
        // (reclaim the borrowed descriptor + owned-temporary bytes operand) + `sum-new` (wrap the success
        // handle into `Some` / the NULL signal into `None` — the `Option a` result). Mirrors `MapLookup`'s
        // present/absent → Some/None op set plus the descriptor bake.
        Core::ValueDecode { bytes, .. } => {
            out.insert(OP_VALUE_DECODE);
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_DROP);
            out.insert(OP_SUM_NEW);
            collect_used_ops_into_seen(db, bytes, out, visited);
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            collect_used_ops_into_seen(db, operand, out, visited)
        }
        Core::Call { args, .. } => {
            // A CONSTANT-BigInt argument to a BigInt param materializes via `bigint-of-i64` in the
            // `Core::ConstInt` collect arm (matching its emit) — no per-call special-case needed here.
            for &arg in args.iter() {
                collect_used_ops_into_seen(db, arg, out, visited);
            }
        }
        // A HOST CALL vs a PEER CALL — mirror the `emit` arm's arg handling EXACTLY, and the two boundaries
        // treat a String argument DIFFERENTLY:
        //  - A HOST call marshals a `Ty::String` arg as `(ptr, len)` into the data segment (the emit arm
        //    consumes the `Core::ConstStr` there) — it is NOT built as a runtime byte-leaf, so it uses NO
        //    runtime op; descending into it via the generic walk would wrongly import `bytes-alloc`/
        //    `bytes-set` (the ConstStr arm), making the runtime-import set non-empty and tripping the
        //    "host + runtime imports don't yet compose" decline for what is a host-ONLY program.
        //  - A PEER call (a peer-BOUND effect, `db.effect_bindings`) crosses a String/Bytes arg as a runtime
        //    HANDLE — the emit arm calls `emit(arg)`, which for a `Core::ConstStr` builds the rope on the
        //    value heap (`bytes-alloc`/`bytes-set`). So a peer String/Bytes arg DOES use runtime ops and MUST
        //    recurse, or the import section would omit an op the body calls (→ an invalid consumer component).
        // A `Unit` arg carries no value on either boundary. So: skip a String/Unit HOST arg; recurse into a
        // peer arg (and any scalar/compound host arg) as before.
        Core::HostCall {
            args,
            effect,
            result,
            ..
        } => {
            let peer_bound = db.effect_bindings.contains_key(&*effect);
            // A SPILLED compound host RESULT is lifted by `emit_result_lift` at the call site — collect the
            // value-heap ops that lift emits (mirrors `declare_result_lift_ops`, the same recursion) so they
            // are imported. Without this the lift's `CallImport` (e.g. run.run's Err-arm `box-int`) resolves
            // to u32::MAX. (Previously the ops were only collected coincidentally, when the guest happened to
            // use the same op elsewhere — a `list<list<u8>>`/`option` result's arr/sum ops usually overlap the
            // guest's own; run.run's `box-int` does not.) A peer-bound compound crosses as a handle (no lift).
            if !peer_bound && crate::backend::wasm::host::result_is_liftable(db, &result) {
                super::super::declare_result_lift_ops(db, &result, out);
            }
            for &arg in args.iter() {
                match crate::infer::type_of(db, arg) {
                    Ty::Unit => {}
                    // A CONSTANT string/bytes host arg crosses via the data segment (no runtime op). A
                    // RUNTIME (non-const) string arg is marshaled by the `_mem` copy loop, which calls
                    // `bytes-len` + `bytes-get` (+ `i32.store8`, a core instr needing no import) — declare
                    // them here to MATCH the emit (else their `CallImport` resolves to u32::MAX → an invalid
                    // module) and descend into the arg to collect the ops that BUILD the runtime rope.
                    Ty::String | Ty::Bytes if !peer_bound => {
                        if !matches!(core_of(db, arg), Core::ConstStr(_)) {
                            out.insert(OP_BYTES_LEN);
                            out.insert(OP_BYTES_GET);
                            collect_used_ops_into_seen(db, arg, out, visited);
                        }
                    }
                    // A RECORD host arg (shape d) is marshaled field-by-field by the guest: `arr-get` (borrow
                    // each field) + a SCALAR field's get-op, OR a BYTES field's rope→mem copy (`bytes-len` +
                    // `bytes-get`). Declare them to MATCH the emit (else the `CallImport` resolves to u32::MAX
                    // → an invalid module), then descend to collect the ops that BUILD the record value.
                    Ty::Record(fields) if !peer_bound => {
                        out.insert(OP_ARR_GET);
                        for fty in fields.values() {
                            collect_record_field_ops(db, fty, out);
                        }
                        collect_used_ops_into_seen(db, arg, out, visited);
                    }
                    // A `list<T>` arg's marshal (`emit_list_bytes_arg_marshal`) walks the list (`vec-len`/
                    // `vec-get`) + copies each `list<u8>` element's rope (`bytes-len`/`bytes-get`) into `mem`.
                    // Declare them (else the marshal's `CallImport` resolves to u32::MAX), then descend to
                    // collect the ops that BUILD the list value.
                    Ty::List(elem) if !peer_bound => {
                        // The marshal (`emit_list_arg_marshal`) walks the list (`vec-len`/`vec-get`), then per
                        // element lowers it (`collect_list_elem_ops` mirrors the marshal's element arms: a
                        // `Bytes` rope copy, a NESTED-list recursion, or a SCALAR unbox). Declare exactly what
                        // the marshal calls (else the `CallImport` resolves to u32::MAX → an invalid module).
                        out.insert(OP_VEC_LEN);
                        out.insert(OP_VEC_GET);
                        collect_list_elem_ops(db, &elem, out);
                        collect_used_ops_into_seen(db, arg, out, visited);
                    }
                    // A bare scalar-payload VARIANT arg (top-level) is decomposed by `emit_variant_reg_flatten`:
                    // `sum-disc`, and on a payload case `sum-payload` + the payload scalar's unbox op. Declare
                    // them (else the marshal's `CallImport` resolves to u32::MAX → an invalid module), then
                    // descend to collect the ops that BUILD the variant value. Mirrors the variant-field /
                    // variant-list-element collection arms.
                    at if !peer_bound
                        && crate::backend::wasm::host::variant_scalar_payload_cases(db, &at)
                            .is_some() =>
                    {
                        out.insert(OP_SUM_DISC);
                        out.insert(OP_SUM_PAYLOAD);
                        if let Some(cases) =
                            crate::backend::wasm::host::variant_scalar_payload_cases(db, &at)
                            && let Some(pd) = cases.iter().position(|(_, p)| p.is_some())
                            && let Some(pty) = variant_payload_ty_at(db, &at, pd as u32)
                            && let Ok(Some(read)) = get_op_ty(db, &pty)
                        {
                            out.insert(read);
                        }
                        collect_used_ops_into_seen(db, arg, out, visited);
                    }
                    _ => collect_used_ops_into_seen(db, arg, out, visited),
                }
            }
        }
        Core::Seq { stmts, tail } => {
            for &s in stmts.iter() {
                // (A) CASE2 heap-arg: a marked HEAP-typed strict-force stmt is rc-reclaimed at emit with
                // `OP_DROP` (a fresh owned producer result) — import `drop` so that call resolves (emit/
                // import agreement, mirroring the Seq emit's condition exactly).
                if db.strict_force_eval.contains(&s)
                    && crate::core_analysis::is_heap_type(&crate::infer::type_of(db, s))
                {
                    out.insert(OP_DROP);
                }
                collect_used_ops_into_seen(db, s, out, visited);
            }
            collect_used_ops_into_seen(db, tail, out, visited);
        }
        // A boundary block / break — descend into the body / break value to reach any op inside.
        Core::Block { body, .. } => collect_used_ops_into_seen(db, body, out, visited),
        Core::Break { value } => collect_used_ops_into_seen(db, value, out, visited),
        // The abort VALUE is evaluated before the non-local branch, so its ops belong to the import/op set;
        // recurse into it. `handle_id` is a reference to the target handle node, not an emitted subexpression.
        Core::HandleAbort { value, .. } => collect_used_ops_into_seen(db, value, out, visited),
        Core::Record { fields } => {
            // A runtime record builds on the heap exactly as a tuple — `arr-alloc` + per-field
            // `box-*`/`arr-set` (the same ops `emit`'s `Core::Record` arm lays down), so the used-set
            // must include them or the import section would omit an op the body calls.
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            // Box each field by the DECLARED field type (`box_op_for`), NOT the field-value NODE's type
            // (`box_op`) — the SAME choice `emit`'s `Core::Record` arm makes. They MUST agree: a `(: x
            // Float32)` field with a bare `1.5` value has node type `Float64` → `box_op` would declare
            // `box-float`, but emit boxes by the declared `Float32` → `box-float32`, so the used-set would
            // OMIT `box-float32` and the body's `call box-float32` resolves to an out-of-bounds func index
            // (an INVALID module). Read the declared field types off the record's own solved type by name.
            let field_tys = match crate::infer::type_of(db, id).strip_nominal() {
                crate::ty::Ty::Record(m) => Some((*m).clone()),
                _ => None,
            };
            for (name, value) in fields.iter() {
                let boxed = match field_tys.as_ref().and_then(|m| m.get(name)) {
                    Some(declared) => box_op_for(db, *value, declared),
                    None => box_op(db, *value),
                };
                if let Ok(Some(op)) = boxed {
                    out.insert(op);
                }
                if elem_needs_rope_compaction(db, *value) {
                    out.insert(OP_BYTES_COMPACT);
                }
                collect_used_ops_into_seen(db, *value, out, visited);
            }
        }
        // A sum construction always calls `sum-new`; the payload build mirrors `emit`'s `Core::SumNew`:
        //  - nullary → the inline-unit CONSTANT (`IMM_UNIT`), no runtime op (see `emit`);
        //  - single → `box-*` the one payload (a compound payload is already a handle, no box);
        //  - multi → a tuple handle (`arr-alloc` + per-payload `box-*`/`arr-set`).
        Core::SumNew { payloads, .. } => {
            // An ENUM-DISC sum (all variants nullary) is built as a bare `i32.const disc` — NO `sum-new`,
            // no payload op (mirrors `emit`'s `node_is_enum_disc` fast path). Over-reporting `sum-new`
            // here declares a dead runtime import.
            if node_is_enum_disc(db, id) {
                return;
            }
            out.insert(OP_SUM_NEW);
            match payloads.len() {
                0 => {
                    // The unit payload is the inline-unit constant — no `arr-alloc` import.
                }
                1 => {
                    if let Ok(Some(op)) = box_op(db, payloads[0]) {
                        out.insert(op);
                    }
                    if elem_needs_rope_compaction(db, payloads[0]) {
                        out.insert(OP_BYTES_COMPACT);
                    }
                    collect_used_ops_into_seen(db, payloads[0], out, visited);
                }
                _ => {
                    out.insert(OP_ARR_ALLOC);
                    out.insert(OP_ARR_SET);
                    for p in payloads.iter() {
                        if let Ok(Some(op)) = box_op(db, *p) {
                            out.insert(op);
                        }
                        if elem_needs_rope_compaction(db, *p) {
                            out.insert(OP_BYTES_COMPACT);
                        }
                        collect_used_ops_into_seen(db, *p, out, visited);
                    }
                }
            }
        }
        // A sum match calls `sum-disc` to dispatch at each switch; a switch on a deeper sub-value (a
        // non-empty `path`) first WALKS there (`sum-payload`/`arr-get` per step) before the disc. The
        // scrutinee + the root continuation are emitted (any op reachable in the tree must be imported) —
        // `collect_cont_ops` recurses switches/guards, inserting each switch's disc + walk ops.
        Core::MatchSum { scrutinee, root } => {
            // Owned-shell reclaim (see the emit): a non-reusable OWNED boxed-sum scrutinee whose variants all
            // carry SCALAR (or no) payloads has its shell dropped after the match. This collect pass has no
            // slots/reusability info, so import `drop` whenever the operand is an owned boxed sum — a SUPERSET
            // of the emit's condition (a declared-but-unused import is harmless; the emit only emits the drop
            // for a freshly-stashed owned i32-handle shell whose payloads are reclaim-safe: all-scalar OR
            // compound-but-borrow-only). Broadened from the all-scalar-only test to cover the compound-borrow-
            // only shell reclaim too (else the emit's drop would lack its import → invalid module). NOT for an
            // enum-disc (bare i32, no shell).
            let scrut_ty = type_of(db, scrutinee);
            // Also import `drop` for a PARAM/LocalRef scrutinee: the NON-TAIL SPINE reclaim (v-mem-safety-
            // signed-off) drops a proven-owned-dead-after param's shell slot, but heap_operand_ownership(Param)
            // == Borrowed so the Owned test below misses it → without this the emitted `drop` would have no
            // import (function index u32::MAX → invalid module). A SUPERSET of the emit's param-reclaim gate (a
            // non-reclaimed param scrutinee → a declared-but-unused import = harmless, per the note above).
            let param_scrut = matches!(
                core_of(db, scrutinee),
                Core::Param { .. } | Core::LocalRef { .. }
            );
            if is_heap_type(&scrut_ty)
                && !ty_is_enum_disc(db, &scrut_ty)
                && (matches!(
                    heap_operand_ownership(db, scrutinee),
                    Ok(HandleOwnership::Owned)
                ) || param_scrut)
            {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, scrutinee, out, visited);
            collect_cont_ops(db, scrutinee, &root, out);
        }
        // A list match reads `vec-len` to dispatch by length; arm bodies' element/rest binders bring in
        // `vec-get`/`vec-split` via their own `SumPayload` occurrences. A guarded arm's GUARD is also
        // emitted (its ops must be collected too).
        Core::MatchList { scrutinee, arms } => {
            out.insert(OP_VEC_LEN);
            // `drop` may be emitted to reclaim an owned-temporary list shell after the arms (the
            // `list_shell_reclaim_slot` path); import it unconditionally (a declared-but-unused import is
            // benign, and the reclaim gate is emit-time TailPos-dependent, not knowable here).
            out.insert(OP_DROP);
            collect_used_ops_into_seen(db, scrutinee, out, visited);
            for arm in &arms {
                if let Some(g) = arm.guard {
                    collect_used_ops_into_seen(db, g, out, visited);
                }
                collect_used_ops_into_seen(db, arm.body, out, visited);
            }
        }
        // A sum-payload read walks its `path` (`sum-payload`/`arr-get` per step) then unboxes the leaf
        // by THIS node's solved type (`get-*`).
        Core::SumPayload { scrutinee, path } => {
            for step in path.iter() {
                match step {
                    crate::core::PathStep::Payload => {
                        out.insert(OP_SUM_PAYLOAD);
                    }
                    // An `Elem` may read a tuple `arr` OR a list `vec`; insert both (emit picks by type).
                    crate::core::PathStep::Elem(_) => {
                        out.insert(OP_ARR_GET);
                        out.insert(OP_VEC_GET);
                    }
                    // A list REST binder slices the tail with `vec-drop` (single-result tail), preceded by
                    // a `dup` to RETAIN the shared arm handle across the consuming slice (so a sibling
                    // element binder in the same arm still reads a live handle — see the `RestFrom` emit).
                    crate::core::PathStep::RestFrom(_) => {
                        out.insert(OP_VEC_DROP);
                        out.insert(OP_DUP);
                    }
                    // A runtime tuple-rest read declines at emit (slice 1), so it emits no ops here.
                    crate::core::PathStep::TupleRestFrom(_) => {}
                };
            }
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
            collect_used_ops_into_seen(db, scrutinee, out, visited);
        }
        // `expect` probes the discriminant (`sum-disc`) and, on the present arm, reads the payload
        // (`sum-payload`) then unboxes by the result type (`get-*`); the absent arm traps (no op). It also
        // emits the scrutinee once.
        Core::SumExpect { scrutinee, .. } => {
            out.insert(OP_SUM_DISC);
            out.insert(OP_SUM_PAYLOAD);
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
            // Shell reclaim (see the emit): an OWNED-temporary sum scrutinee's shell is dropped after the
            // payload read. This collect pass has no `slots` to test `reusable_handle_slot`, so import `drop`
            // whenever the operand's ownership is Owned — a superset of the emit's condition (a declared-but-
            // unused import is harmless; the emit only actually emits the drop for a freshly-stashed owned
            // shell with a scalar/dup'd-compound payload).
            if matches!(
                heap_operand_ownership(db, scrutinee),
                Ok(HandleOwnership::Owned)
            ) {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, scrutinee, out, visited);
        }
        // A closure VALUE is a heap CELL — `arr-alloc(1 + captures)` then `arr-set` of `box-int(code)`
        // (slot 0) and each boxed capture. So it uses `arr-alloc`/`arr-set`/`box-int` always, plus the
        // per-capture box op. A closure APPLICATION reads the code slot (`arr-get`+`get-int`) then
        // `call_indirect` (a core instruction, not a runtime import), plus its operands.
        Core::Closure { captures, .. } => {
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            out.insert(OP_BOX_INT); // slot 0 = box-int(code)
            for &c in captures.iter() {
                if let Ok(Some(op)) = box_op(db, c) {
                    out.insert(op);
                }
                collect_used_ops_into_seen(db, c, out, visited);
            }
        }
        Core::CallClosure { closure, args } => {
            out.insert(OP_ARR_GET); // read the code slot from the cell
            out.insert(OP_GET_INT); // unbox it to the table index
            // SITE-A owned-temp env-cell reclaim (part b): mirror the emit's drop condition so `drop` is
            // imported whenever an Owned closure operand with a non-function result is reclaimed after the
            // borrowing call (else `local.get cell_slot; drop` references an undeclared import → invalid
            // module). Same gate as the emit: operand Owned + result not a function type.
            if matches!(
                heap_operand_ownership(db, closure),
                Ok(HandleOwnership::Owned)
            ) && !matches!(type_of(db, id), crate::ty::Ty::Fn(_, _))
            {
                out.insert(OP_DROP);
            }
            collect_used_ops_into_seen(db, closure, out, visited);
            for &arg in args.iter() {
                collect_used_ops_into_seen(db, arg, out, visited);
            }
        }
        // A CAPTURED-variable read: `arr-get(env, 1+index)` then unbox by the captured value's type.
        Core::Captured { .. } => {
            out.insert(OP_ARR_GET);
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
        }
        // A constant STRING used as an in-body runtime value builds a flat UTF-8 byte leaf via
        // `bytes-alloc` + a `bytes-set` per byte (byte-identical to `str-new`'s rep — see the `emit`
        // arm). So it imports those two ops. (A constant string that only FOLDS — its equality, or an
        // escape's baked bytes — never reaches the emit path, so this is a superset that is harmless if
        // the string folds away; `collect_used_ops` mirrors `emit`'s op choices for the values that DO
        // reach emission.)
        Core::ConstStr(_) => {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
        }
        // A CONSTANT typed `BigInt` used as an in-body runtime value MATERIALIZES at `emit` (a map key/value,
        // set element, call arg, op operand — it must be an i32 handle, not a raw i64): `bigint-of-i64` for
        // an i64-fitting value, or `bytes-alloc`/`bytes-set`/`bigint-of-bytes` for a beyond-i64 one — declare
        // whichever it needs here to match `emit_const_bigint_leaf`. (A whole-export constant BigInt takes
        // the baked-bytes path and never reaches `emit`, but an unused import would be harmless if it did.)
        Core::ConstInt(_) if is_bigint_valued(db, id) => {
            insert_const_bigint_materialize_ops(db, id, out);
        }
        // A constant Rational used as an in-body runtime value MATERIALIZES via `bigint-of-i64` (×2 the
        // components) + `rational-of` at `emit` — declare those imports to match. (A component beyond i64
        // declines at emit; a whole-export constant Rational takes the baked path and doesn't reach here.)
        Core::ConstRational(n, d) if n.to_i64().is_some() && d.to_i64().is_some() => {
            out.insert(OP_BIGINT_OF_I64);
            out.insert(OP_RATIONAL_OF);
        }
        // Leaves and references emit no runtime op. `trap` emits `unreachable` (a core instruction, not a
        // runtime import), so it adds nothing.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
        | Core::Param { .. }
        | Core::LocalRef { .. }
        | Core::Poison(_) => {}
    }
}

/// Collect the runtime ops a sum-match CONTINUATION uses — a leaf's body, or a nested switch (its own
/// `sum-disc` + path walk ops + its arms, recursed). Mirrors the `MatchSum` arm walk in `collect_used_ops`
/// so an op used only deep in the tree is still imported.
pub(super) fn collect_cont_ops(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    // The entered-variant payload types, threaded exactly as the EMIT threads `Emit::sum_path_types`, so the
    // `sub_is_enum` disc-op choice here agrees with `push_discriminant`'s (which now resolves a `Payload`
    // step to the ACTUAL entered variant, not variant 0). Starts empty at the root.
    let mut recorded: HashMap<(StructId, Vec<crate::core::PathStep>), Ty> = HashMap::new();
    collect_cont_ops_rec(db, scrutinee, cont, &mut recorded, out);
}

pub(super) fn collect_cont_ops_rec(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    recorded: &mut HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    match cont {
        // OP-ONLY recursion into an arm body: use `collect_used_ops_into` (the op-set walk), NOT the full
        // `collect_used_ops`. The full one ALSO runs the three dup-site collectors (collect_retain_candidate_
        // binders / collect_shell_reclaim_child_dups / collect_row_op_field_dups), each a FULL-subtree walk —
        // but those already ran ONCE over the WHOLE body at the enclosing `collect_used_ops` call (their
        // `core_child_ids` recursion descends into every MatchSum arm, so the arm bodies are already covered),
        // and the `dup` import decision was made there. Re-running the full `collect_used_ops` per arm re-ran
        // the trio on each arm's subtree — O(match-nesting-depth × subtree) = the 2.58-BILLION-`core_of`-call
        // spec-body re-walk that made the arg-NApp demand spine's specialize pass run 300s+ (v-cml profile,
        // 2026-08-10: the trio tied at ~134M+ climbing). Op collection alone (this walk) recurses arms once;
        // the dup trio does not repeat. The imported op set is identical (the trio only fills the dup-site set
        // + the single `OP_DUP` import, both already decided at the top-level body call).
        crate::core::SumCont::Leaf(body) => collect_used_ops_into(db, *body, out),
        // A guarded arm uses the ops of its guard cond, its body, AND the fall-through continuation.
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_used_ops_into(db, *cond, out);
            collect_used_ops_into(db, *body, out);
            collect_cont_ops_rec(db, scrutinee, els, recorded, out);
        }
        // A literal test walks its `path` (sum-payload/arr-get|vec-get) then reads the leaf scalar to compare
        // it; an Int probe reads `get-int`, a Bool probe `get-bool`. Then both continuations' ops.
        crate::core::SumCont::LitTest {
            path,
            probe,
            then_,
            els,
        } => {
            for step in path.iter() {
                match step {
                    crate::core::PathStep::Payload => out.insert(OP_SUM_PAYLOAD),
                    // An `Elem` may read a tuple `arr` OR a list `vec` — insert both; emit picks by type.
                    crate::core::PathStep::Elem(_) => {
                        out.insert(OP_ARR_GET);
                        out.insert(OP_VEC_GET)
                    }
                    crate::core::PathStep::RestFrom(_) => false, // never on a sum-disc path
                    crate::core::PathStep::TupleRestFrom(_) => false, // never on a sum-disc path
                };
            }
            // The payload leaf type after the path walk — needed to tell an Int probe over a BOXED
            // BIGINT leaf (compares via `bigint-cmp` + a materialized leaf, imports below) from an
            // ordinary fixnum/narrow Int probe (`get-int`). MUST agree with `emit_littest_probe`'s `cur`
            // so the import set matches the emitted ops exactly (an extra/missing import shifts every
            // `CallImport` index → invalid module). Resolve it ENTERED-VARIANT-AWARE, mirroring emit's
            // `payload_step_ty_of(.., sum_path_types)`: a `Payload` over a MULTI-variant sum entered at a
            // NON-zero variant must take THAT variant's payload, not variant 0. `ty_at_path_recorded`
            // consults the same `recorded` map the Switch arm threads (scoped save/restore per entered
            // disc), so `int_probe_is_bigint` here agrees with emit's BigInt-branch for EVERY variant — a
            // hardcoded variant-0 walk was exact only for the single-variant `(type W (Mk BigInt))` shape
            // the original fix's test covered, and omitted the 6 bigint imports for a BigInt payload on a
            // non-variant-0 arm.
            let lit_root = type_of(db, scrutinee);
            let lit_cur = ty_at_path_recorded(db, scrutinee, &lit_root, path, recorded);
            let int_probe_is_bigint = matches!(lit_cur.strip_nominal(), Ty::BigInt);
            match probe {
                // A BIGINT-payload Int probe compares via `bigint-cmp` over a materialized literal leaf
                // (`bigint-of-i64` for a fits-i64 literal, else the sign-magnitude byte leaf), then drops
                // the owned literal — mirror `emit_littest_probe`'s BigInt branch EXACTLY. An ordinary
                // (fixnum/narrow) Int probe reads `get-int`.
                crate::core::Probe::Int(_) if int_probe_is_bigint => {
                    out.insert(OP_BIGINT_CMP);
                    out.insert(OP_DROP);
                    out.insert(OP_BIGINT_OF_I64);
                    out.insert(OP_BYTES_ALLOC);
                    out.insert(OP_BYTES_SET);
                    out.insert(OP_BIGINT_OF_BYTES)
                }
                crate::core::Probe::Int(_) => out.insert(OP_GET_INT),
                crate::core::Probe::Bool(_) => out.insert(OP_GET_BOOL),
                // A string-literal probe over a RUNTIME payload emits a `value-eq` content compare against a
                // freshly-built literal byte-leaf (`bytes-alloc`+`bytes-set`), after `bytes-compact`ing the
                // leaf handle to canonical flat form and dropping the owned literal — so import all four (the
                // emit arm's ops). A CONSTANT string sub-value still folds in `build_tree` and never reaches
                // here; this covers the runtime case (`(Ast.Name "+")` over a runtime Ast).
                crate::core::Probe::Str(_) => {
                    out.insert(OP_BYTES_COMPACT);
                    out.insert(OP_BYTES_ALLOC);
                    out.insert(OP_BYTES_SET);
                    out.insert(OP_VALUE_EQ);
                    out.insert(OP_DROP)
                }
                // A char-literal probe over a RUNTIME char reaches emit now that `is_scalar` includes
                // `Ty::Char` (Char-rep 2/N): a `Char` is a boxed i32 code-point (`box_op_ty(Char) =
                // box-int`), so `emit_littest_probe`'s Char arm reads the boxed leaf with `get-int` (→ i64)
                // then `i64.eq`s the literal code point — the SAME unbox the `Int` probe uses. Declare it,
                // else the emitted `CallImport(OP_GET_INT)` resolves to an out-of-bounds function index
                // (invalid module — a runtime char match at a sub-path miscompiled to `call u32::MAX`). A
                // CONSTANT char sub-value still folds in `build_tree` and never reaches here.
                crate::core::Probe::Char(_) => out.insert(OP_GET_INT),
                // A byte-string-literal probe over a RUNTIME payload emits the SAME `value-eq` content
                // compare a `Str` probe does — a Bytes is a flat byte leaf built by `bytes-alloc`+`bytes-set`
                // and compared by `value-eq` after `bytes-compact`ing the payload handle (rope→flat). So it
                // needs the same four ops. (A CONSTANT bytes sub-value still folds in `build_tree`.)
                crate::core::Probe::Bytes(_) => {
                    out.insert(OP_BYTES_COMPACT);
                    out.insert(OP_BYTES_ALLOC);
                    out.insert(OP_BYTES_SET);
                    out.insert(OP_VALUE_EQ);
                    out.insert(OP_DROP)
                }
                // A `ListLen` probe over a runtime list payload reads `vec-len` of the sub-list handle to
                // gate the arm (a constant list folds instead, never reaching here).
                crate::core::Probe::ListLen { .. } => out.insert(OP_VEC_LEN),
                // A `MapHasKeys` probe only ever FOLDS (a constant map sub-value); a runtime map declines at
                // `build_lit_test` before a decision tree emits, so it never reaches a runtime LitTest — no
                // op to collect.
                crate::core::Probe::MapHasKeys { .. } => false,
                crate::core::Probe::Wild => false,
            };
            collect_cont_ops_rec(db, scrutinee, then_, recorded, out);
            collect_cont_ops_rec(db, scrutinee, els, recorded, out);
        }
        crate::core::SumCont::Switch { path, arms } => {
            // Mirror `push_discriminant` EXACTLY: the switched sub-value's discriminant is read via
            // `sum-disc` for a BOXED sum, but an ENUM-DISC value needs none at the top level (it IS the
            // raw i32) and `get-int` (+ `i32.wrap_i64`, a core op) at a NESTED position. Over-reporting
            // `sum-disc` here — as the old unconditional insert did — declares a DEAD runtime import for a
            // program whose only match is on an all-nullary enum (now a bare i32), forcing a needless
            // `heap` linkage. Resolve the sub-value's type at `path` USING the recorded entered-variant
            // types (so a non-variant-0 payload agrees with the emit) and branch as the emit does.
            let root = type_of(db, scrutinee);
            let sub = ty_at_path_recorded(db, scrutinee, &root, path, recorded);
            let sub_is_enum = ty_is_enum_disc(db, &sub);
            for step in path.iter() {
                match step {
                    crate::core::PathStep::Payload => out.insert(OP_SUM_PAYLOAD),
                    // An `Elem` may read a tuple `arr` OR a list `vec` — insert both; emit picks by type.
                    crate::core::PathStep::Elem(_) => {
                        out.insert(OP_ARR_GET);
                        out.insert(OP_VEC_GET)
                    }
                    crate::core::PathStep::RestFrom(_) => false, // never on a sum-disc path
                    crate::core::PathStep::TupleRestFrom(_) => false, // never on a sum-disc path
                };
            }
            if sub_is_enum {
                // A NESTED enum-disc was boxed as an int → `get-int` recovers it (the wrap is a core op).
                if !path.is_empty() {
                    out.insert(OP_GET_INT);
                }
            } else {
                out.insert(OP_SUM_DISC);
            }
            for arm in arms {
                // Thread each arm's entered-variant payload type (scoped save/restore), mirroring the emit.
                let restore = arm
                    .disc
                    .and_then(|d| record_entered_payload_ty_into(db, scrutinee, path, d, recorded));
                collect_cont_ops_rec(db, scrutinee, &arm.cont, recorded, out);
                restore_entered_payload_ty_into(scrutinee, path, restore, recorded);
            }
        }
    }
}
