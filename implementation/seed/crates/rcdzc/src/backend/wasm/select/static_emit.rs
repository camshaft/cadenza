//! Static-data emission — the immortal build-once constant BYTES/STRING + compound (Tuple/Record/List/
//! Map/Set) hoist path (`DESIGN-static-data.md`), peeled out of select.rs to hold the module-root file
//! under the 512 KiB cap. Pure mechanical split, no logic change; `use super::*` inherits select.rs's
//! imports + sibling-module helpers (box_op/emit_heap_store_tail/emit_key_canonicalize/…). The 6 fns are
//! re-exported into the select root via `pub(crate) use static_emit::*` so every existing call site
//! (select/emit.rs's try_emit_static_*, mod.rs/sum_resource.rs's `select::build_static_compound_init`/
//! `collect_static_compound_ops`) resolves unchanged.
use super::*;

/// §2d STATIC BYTES/STRINGS (`DESIGN-static-data.md`): if `id` is a fully-constant flat-byte-payload value
/// present in the build-once table (`layout.static_bytes`), emit a BARE `global.get` of its module global
/// and return `true`. The value was built ONCE at instantiation (the `CORE_SEC_START` init) and marked
/// IMMORTAL (`mark-immortal`), so a plain read is all a use needs: `op_dup`/`op_drop` are NO-OPs on an
/// immortal node, so the consumer treating the handle as owned and dropping it is harmless (never frees the
/// shared static → no UAF), and `node_rc == IMMORTAL` makes FBIP path-copy so the static is never mutated
/// in place. No dup, no drop, no per-eval `bytes-alloc`+`bytes-set`.
///
/// Covers a constant `Bytes` (a `Core::BytesOf` of constants OR a baked `Core::ConstBytes`, via
/// `constant_bytes_value`) AND a constant `String` (a `Core::ConstStr`, via `constant_string_value`) — a
/// Cadenza `String` value IS the identical flat UTF-8 byte-leaf a `Bytes` is (`str-new`'s rep), built by
/// the same `bytes-alloc`+`bytes-set`, so both hoist through this one path. The table is interned BY
/// CONTENT, so a `String` and a `Bytes` with equal bytes share the ONE immortal global (sound: both are
/// i32 handles to the same leaf rep). Returns `false` (build inline) for a runtime literal or a program
/// with no static table, keeping every non-hoisted program byte-identical.
pub(crate) fn try_emit_static_bytes(
    db: &mut Db,
    id: StructId,
    layout: &Layout,
    out: &mut Emit,
) -> bool {
    if let Some(payload) = crate::lower::constant_bytes_value(db, id)
        .or_else(|| crate::lower::constant_string_value(db, id))
        && let Some(pos) = layout.static_bytes.iter().position(|b| *b == payload)
    {
        out.push(Lir::GlobalGet(pos as u32)); // [handle] — the once-built immortal static, owned-by-value
        return true;
    }
    false
}

/// §2d STATIC COMPOUNDS (`DESIGN-static-data.md` increment 6): if `id` is a markable constant
/// `Tuple`/`Record`/small-`List` in the build-once table, emit a bare `global.get` of its module global and
/// return `true` (the routing is keyed by node id, so it is type-agnostic — a list uses the same table).
/// Compound globals are laid AFTER the static-bytes globals, so compound `pos`'s global index is
/// `static_bytes.len() + pos`. The tree was built ONCE (immortal, per-node marked) by the `start` init, so a
/// use just reads the handle (`op_dup`/`op_drop` no-op on the immortal root; FBIP path-copies). `false`
/// (build the compound inline per-eval, as before) for a non-tabled or runtime compound.
pub(crate) fn try_emit_static_compound(
    db: &mut Db,
    id: StructId,
    layout: &Layout,
    out: &mut Emit,
) -> bool {
    let _ = db;
    if let Some(pos) = layout.static_compounds.iter().position(|&c| c == id) {
        out.push(Lir::GlobalGet((layout.static_bytes.len() + pos) as u32)); // [handle] — immortal compound
        return true;
    }
    false
}

/// §2d increment 6: emit the IMMORTAL build of a markable constant compound `id` into the `start` init,
/// leaving its handle on the stack. Builds every node inline and marks it IMMORTAL per node (`mark-immortal`
/// is shallow, so the WHOLE tree must be marked to be census-excluded + drop-safe): `arr-alloc(n)` then, per
/// element, build its handle + `arr-set`, then `mark-immortal` the root array. Mirrors the runtime
/// `Core::Tuple`/`Core::Record` emit (a record IS a tuple at run time) but recurses for a nested compound
/// and marks each node. Self-contained — references no other global — so ordering across the init is
/// irrelevant. Called on a `Tuple`/`Record` (arr root) OR a small constant `List` (arr + `vec-of-arr`, both
/// nodes marked — see the `ListNew` arm) collected by `collect_static_compounds`.
pub(crate) fn emit_immortal_static(
    db: &mut Db,
    id: StructId,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    match core_of(db, id) {
        Core::Tuple { elems } => {
            let elem_tys = match type_of(db, id).strip_nominal() {
                Ty::Tuple(ts) => Some(ts.clone()),
                _ => None,
            };
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // [arr]
            for (i, &elem) in elems.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit_immortal_elem(db, elem, elem_tys.as_ref().and_then(|ts| ts.get(i)), layout, out)?;
                out.push(Lir::CallImport(OP_ARR_SET)); // [arr]
            }
            out.push(Lir::CallImport("mark-immortal")); // [arr] — the tuple root, immortal
            Ok(())
        }
        Core::Record { fields } => {
            let field_tys = match type_of(db, id).strip_nominal() {
                Ty::Record(m) => Some((*m).clone()),
                _ => None,
            };
            out.push(Lir::ConstI32(fields.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // [arr] (a record IS a tuple at run time)
            for (i, (name, &value)) in fields.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                let declared = field_tys.as_ref().and_then(|m| m.get(name));
                emit_immortal_elem(db, value, declared, layout, out)?;
                out.push(Lir::CallImport(OP_ARR_SET)); // [arr]
            }
            out.push(Lir::CallImport("mark-immortal")); // [arr] — the record root, immortal
            Ok(())
        }
        // A NULLARY variant of a MIXED sum (`(Z)`/`(Nil)`) — a real heap node (`sum-new(disc, IMM_UNIT)`)
        // built ONCE, immortal (`is_markable_constant_sum_nullary`; the rsl1 leak-1 fix). SHALLOW
        // `mark-immortal` suffices: the sum root wraps the inline-unit sentinel `IMM_UNIT` (rc-free, no heap
        // child), so there is nothing deeper to mark — unlike the list/map/set roots that hold heap children.
        Core::SumNew { disc, payloads } if payloads.is_empty() => {
            out.push(Lir::ConstI32(disc as i32)); // [disc]
            out.push(Lir::ConstI32(super::super::runtime_abi::IMM_UNIT as i32)); // [disc, unit]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [sum-handle]
            out.push(Lir::CallImport("mark-immortal")); // [sum-handle] — the nullary sum root, immortal
            Ok(())
        }
        // A PAYLOADED variant of a MIXED sum with ALL-CONSTANT payloads (`(Some 5)`, `(Cons 1 (list …))`) —
        // built ONCE immortal, mirroring the runtime `Core::SumNew` payload marshaling (`select.rs` emit) for
        // constants: 1 payload → the boxed handle IS the sum's payload; n → a tuple `arr` of boxed payloads.
        // Then `mark-immortal-DEEP` (op 96) — unlike the nullary SHALLOW mark, the payload(s) are HEAP CHILDREN
        // (the boxed scalar / built compound / arr), so a deep mark is needed to census-exclude the whole tree
        // (exactly like the const-list/map/set roots). `emit_immortal_elem` builds + shallow-marks each payload
        // (idempotent under the final deep mark). Collected by `is_markable_constant_sum_payloaded`.
        Core::SumNew { disc, payloads } => {
            out.push(Lir::ConstI32(disc as i32)); // [disc]
            match payloads.len() {
                1 => {
                    // The single payload's boxed handle is passed to `sum-new` directly (no wrapping `arr`).
                    emit_immortal_elem(db, payloads[0], None, layout, out)?; // [disc, payload-handle]
                }
                n => {
                    // Multiple payloads: box each into a positional tuple `arr` (the runtime multi-payload shape).
                    out.push(Lir::ConstI32(n as i32)); // [disc, n]
                    out.push(Lir::CallImport(OP_ARR_ALLOC)); // [disc, arr]
                    for (i, &p) in payloads.iter().enumerate() {
                        out.push(Lir::ConstI32(i as i32)); // [disc, arr, i]
                        emit_immortal_elem(db, p, None, layout, out)?; // [disc, arr, i, handle]
                        out.push(Lir::CallImport(OP_ARR_SET)); // [disc, arr]
                    }
                }
            }
            out.push(Lir::CallImport(OP_SUM_NEW)); // [sum-handle]
            out.push(Lir::CallImport("mark-immortal-deep")); // deep — payload(s) are heap children
            Ok(())
        }
        // A constant list of ANY size (non-empty, not all-`Bool`) — built like a tuple (a flat `arr` of boxed
        // elements) then `vec-of-arr`. The build is UNIFORM across sizes: `arr-alloc(n)` + per-element build +
        // `arr-set`, then `vec-of-arr`. What differs is the node topology `vec-of-arr` produces — ≤32 reuses the
        // `arr` as the sole leaf under an 8-byte header; `>32` DRAINS the elements into ≤32-element trie leaves
        // and builds a radix trie (INTERNAL nodes minted inside the op, no compile-time handle). So the root is
        // marked with `mark-immortal-DEEP` (op 96), which transitively marks the whole structure — header + arr
        // leaf (≤32) OR spine + all trie leaves (>32) + every element handle — in ONE call, reaching the trie
        // internals a per-node shallow mark could not. Do NOT shallow-mark the `arr` before `vec-of-arr`: for
        // `>32` the arr shell is drained + dropped (a marked-immortal shell would be orphaned = a leak), and the
        // deep-mark on the result covers the reused-arr leaf for ≤32 anyway. Elements are shallow-marked as built
        // (`emit_immortal_elem`) — redundant with the final deep-mark (idempotent) but harmless. The all-`Bool`
        // PACK path (mints a fresh bit-leaf + drops the arr WITH the marked element boxes → orphaned leak) and the
        // empty-list `vec-empty` singleton are excluded upstream (`is_markable_constant_list`).
        Core::ListNew { elems } => {
            let elem_ty = match type_of(db, id).strip_nominal() {
                Ty::List(t) => Some((**t).clone()),
                _ => None,
            };
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // [arr]
            for (i, &elem) in elems.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit_immortal_elem(db, elem, elem_ty.as_ref(), layout, out)?;
                out.push(Lir::CallImport(OP_ARR_SET)); // [arr]
            }
            out.push(Lir::CallImport(OP_VEC_OF_ARR)); // [arr] → [list] (arr reused (≤32) or drained into a trie (>32))
            out.push(Lir::CallImport("mark-immortal-deep")); // [list] — transitively immortal (header/arr or spine/leaves + elems)
            Ok(())
        }
        // A constant MAP — built EXACTLY like the runtime `Core::MapNew` arm (map-empty + per-entry box key/value
        // by their types, rope-compact / list-key-canonicalize the key for CHAMP slot exactness, map-insert),
        // then ONE `mark-immortal-deep` on the final root. `map-insert` CONSUMES map+key+value (moves them into the
        // CHAMP, no copy), so there is no orphan-leak hazard — the deep-mark on the final root transitively marks
        // the whole CHAMP (HAMT spine + data-entry key/value handles + nested payloads). The keys/values build via
        // a FRESH minimal emit context (like `emit_immortal_elem`): empty slots, base 0, its own high-water +
        // scratch-type map — and since `collect_static_compounds` does NOT descend into a collected map root, no
        // key/value node is itself in `static_compounds`, so `emit` builds each inline (never routes to global.get).
        Core::MapNew {
            entries,
            key_ty,
            val_ty,
        } => {
            let slots: HashMap<StructId, u32> = HashMap::new();
            let mut high = 0u32;
            let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
            out.push(Lir::CallImport(OP_MAP_EMPTY)); // [map]
            for &(k, v) in entries.iter() {
                let key_base = high; // start this entry's scratch above the running high-water (base 0 → high)
                emit(db, k, &slots, key_base, &mut high, &mut scratch_ty, layout, out)?; // [map, key]
                let key_boxed = box_op_for(db, k, &key_ty)?;
                emit_heap_store_tail(db, k, key_boxed, out); // [map, key-handle]
                if key_needs_compaction(db, k) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
                }
                if key_needs_canonicalize(db, k) {
                    emit_key_canonicalize(db, k, &key_ty, &mut high, &mut scratch_ty, out)?; // [map, canon-key]
                }
                let val_base = high;
                emit(db, v, &slots, val_base, &mut high, &mut scratch_ty, layout, out)?; // [map, key, val]
                let val_boxed = box_op_for(db, v, &val_ty)?;
                emit_heap_store_tail(db, v, val_boxed, out); // [map, key, val-handle]
                out.push(Lir::CallImport(OP_MAP_INSERT)); // → [map'] (consumes map, key, val)
            }
            out.push(Lir::CallImport("mark-immortal-deep")); // [map] — transitively immortal (CHAMP spine + k/v)
            Ok(())
        }
        // A constant SET — the set analogue of the Map arm (CHAMP-minus-value-column): `set-empty` + per-element
        // box-by-type + rope-compact / list-element-canonicalize + `set-insert` (CONSUMES set+element, moves in,
        // no copy), then ONE `mark-immortal-deep` on the final root (marks the whole HAMT + element handles).
        Core::SetOf { elems, elem_ty } => {
            let slots: HashMap<StructId, u32> = HashMap::new();
            let mut high = 0u32;
            let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
            out.push(Lir::CallImport(OP_SET_EMPTY)); // [set]
            for &e in elems.iter() {
                let elem_base = high;
                emit(db, e, &slots, elem_base, &mut high, &mut scratch_ty, layout, out)?; // [set, elem]
                let elem_boxed = box_op_for(db, e, &elem_ty)?;
                emit_heap_store_tail(db, e, elem_boxed, out); // [set, elem-handle]
                if key_needs_compaction(db, e) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
                }
                if key_needs_canonicalize(db, e) {
                    emit_key_canonicalize(db, e, &elem_ty, &mut high, &mut scratch_ty, out)?; // [set, canon-elem]
                }
                out.push(Lir::CallImport(OP_SET_INSERT)); // → [set'] (consumes set, elem)
            }
            out.push(Lir::CallImport("mark-immortal-deep")); // [set] — transitively immortal (CHAMP spine + elems)
            Ok(())
        }
        _ => Err(Reject::decline(
            "emit_immortal_static reached a non-markable node (only markable Tuple/Record/List/Map/Set are collected)"
                .to_string(),
        )),
    }
}

/// One element of an immortal static compound (see [`emit_immortal_static`]), leaving its handle on the
/// stack: a nested markable `Tuple`/`Record` recurses (its whole subtree is built + marked immortal); a
/// constant `Bytes`/`String` builds its OWN inline immortal leaf (self-contained — not the shared static-
/// bytes global, so init ordering is irrelevant + a tiny duplication is harmless); a constant scalar emits
/// its value, boxes it by the declared element type, and marks the freshly-boxed node immortal (a `Unit`
/// stores the inline `IMM_UNIT` sentinel — no heap node, no mark).
pub(crate) fn emit_immortal_elem(
    db: &mut Db,
    elem: StructId,
    declared: Option<&Ty>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    match core_of(db, elem) {
        // A nested constant compound (Tuple/Record) OR a nested constant mixed-sum (`(Some 5)`/`(Cons …)`/
        // `(Nil)`) OR a nested constant LIST (`(list (list 1) (list 2))`, a list element of a tuple/record/
        // sum-payload): recurse to `emit_immortal_static`, which builds the child + marks it (the parent's
        // final `mark-immortal[-deep]` re-marks idempotently). The `SumNew`/`ListNew` cases are what make
        // nested-collection immortals work — a sum/list element of a list/tuple/record, or a recursive-sum
        // spine, builds once. Without the `ListNew` arm a nested list falls to the `_` scalar path below,
        // whose `box_op` returns `None` for a list handle → the list is left UNMARKED = a census leak.
        Core::Tuple { .. }
        | Core::Record { .. }
        | Core::SumNew { .. }
        | Core::ListNew { .. }
        | Core::MapNew { .. }
        | Core::SetOf { .. } => emit_immortal_static(db, elem, layout, out),
        _ => {
            if let Some(payload) = crate::lower::constant_bytes_value(db, elem)
                .or_else(|| crate::lower::constant_string_value(db, elem))
            {
                out.push(Lir::ConstI32(payload.len() as i32));
                out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [buf]
                for (bi, &b) in payload.iter().enumerate() {
                    out.push(Lir::ConstI32(bi as i32)); // [buf, i]
                    out.push(Lir::ConstI32(b as i32)); // [buf, i, byte]
                    out.push(Lir::CallImport(OP_BYTES_SET)); // [buf]
                }
                out.push(Lir::CallImport("mark-immortal")); // [leaf] — immortal
                return Ok(());
            }
            // A constant scalar (Int/Bool/Unit): emit the value (no scratch — a constant needs none), box it,
            // and mark the box. A fresh empty emit context is safe because a `Core::ConstInt`/`ConstBool`/
            // `Unit` pushes only an inline constant.
            let slots: HashMap<StructId, u32> = HashMap::new();
            let mut high = 0u32;
            let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
            emit(db, elem, &slots, 0, &mut high, &mut scratch_ty, layout, out)?; // [.., value]
            let boxed = match declared {
                Some(d) => box_op_for(db, elem, d)?,
                None => box_op(db, elem)?,
            };
            emit_heap_store_tail(db, elem, boxed, out); // [.., handle] (box, or the unit sentinel)
            if boxed.is_some() {
                out.push(Lir::CallImport("mark-immortal")); // mark the freshly-boxed scalar node
            }
            Ok(())
        }
    }
}

/// Build the `start`-init `Lir` for all static compounds (`DESIGN-static-data.md` §2d, increment 6): for each
/// entry in `layout.static_compounds`, emit its immortal tree ([`emit_immortal_static`]) and `global.set` it
/// to `static_bytes.len() + k` (compound globals follow the byte globals). Called by the backend (which has
/// `Db` — the tree walk needs `core_of`/`type_of`/box selection) and stored in the `Layout`, so
/// `core_module_impl` (which has no `Db`) can APPEND it to the static-bytes init in the START function.
/// Empty `Vec` when there are no static compounds (no additions → byte-identical).
pub(crate) fn build_static_compound_init(
    db: &mut Db,
    compounds: &[StructId],
    byte_base: usize,
    layout: &Layout,
) -> Result<Vec<Lir>, Reject> {
    let mut out = Emit::new();
    for (k, &root) in compounds.iter().enumerate() {
        emit_immortal_static(db, root, layout, &mut out)?; // [handle]
        out.push(Lir::GlobalSet((byte_base + k) as u32)); // store the once-built immortal handle → []
    }
    Ok(std::mem::take(&mut *out))
}

/// The EXACT runtime-op set the static-compound init (`build_static_compound_init` → `emit_immortal_static`)
/// will emit, derived by a DRY-RUN into a throwaway `Emit` + scanning its `CallImport`s. This makes the
/// module import set PRECISE (only the ops each compound's SHAPE actually builds) instead of the prior
/// unconditional over-approximation (which force-imported the full arr/box/bytes/vec/map/set/canonicalize
/// batch whenever ANY static compound existed — leaving e.g. map/set/vec/bytes imports DEAD in a program
/// whose only constants are sums/tuples). No mirror-divergence: this runs the SAME emit path
/// (`emit_immortal_static`), so the collected op set is exactly what the real init emits. A compound that
/// DECLINES in the dry-run is not built by the real init either (`build_static_compound_init` propagates the
/// same `Reject`, so no module is emitted), so ignoring the dry `Err` never under-collects an op that the
/// real init actually emits. The dry `Emit` is discarded; `emit_immortal_static` only reads/memoizes `db`.
pub(crate) fn collect_static_compound_ops(
    db: &mut Db,
    compounds: &[StructId],
    layout: &Layout,
) -> std::collections::BTreeSet<&'static str> {
    let mut ops = std::collections::BTreeSet::new();
    for &root in compounds {
        let mut probe = Emit::new();
        if emit_immortal_static(db, root, layout, &mut probe).is_ok() {
            for instr in probe.code.iter() {
                if let Lir::CallImport(op) = instr {
                    ops.insert(*op);
                }
            }
        }
    }
    ops
}
