//! `lower::runtime_ops` — lowerings for the runtime COLLECTION / STRING / RECORD / TUPLE / BYTES / ARITH
//! operations, split out of `lower.rs`: Map (`to-list`/`insert`/`field`/`lookup`/`remove`), Set
//! (`insert`/`remove`/`algebra`), String (`at`/`scalar-at`/`slice`/`to-bytes`/`from-bytes`), Record
//! (`project`/`merge`/`insert`/`pop`), Tuple (`cat`/`split-at`/`pop`), `Sum.expect`, `char-from-int`,
//! `value-decode`, checked/wrapping arithmetic, and Bytes (`of`/`at`/`concat`/`slice`). Behaviour-
//! preserving move: all items are module-private (now `pub(super)`), reached across the tree via a plain
//! `use runtime_ops::*` re-import in `lower` (and the siblings' own `use super::*`).

use super::*;

/// Lower `(Map.to-list map)` → `Core::MapToList`. Like `Set.to-list`, no const-fold for a NON-EMPTY map
/// (canonical KEY order is the runtime sorted walk). The key/value types come from the operand map's
/// solved `Ty::Map` and bake the map-shape descriptor the runtime orders by.
///
/// The ONE compile-time fold (the MAP twin of `lower_set_to_list`'s empty-set fold): a provably-EMPTY
/// constant map (`Core::MapNew` with no entries) folds to the empty `Core::ListNew` — its canonical
/// enumeration is `[]` regardless of key/value type, so no ordering descriptor is needed. SOUNDNESS-load-
/// bearing: a bare `Map.empty` leaves its key/value types UNDETERMINED (free `Ty::Var`s — no entry ever
/// constrained them), and a var has no orderable shape descriptor. Without this fold `Map.to-list` on
/// such a map declined at the BACKEND ("Map.to-list key/value shape has no orderable descriptor") though
/// the type-checker accepted the program — a check/compile divergence. Folding it here (the key/value
/// type is irrelevant to an empty enumeration) keeps the emit total.
pub(super) fn lower_map_to_list(db: &mut Db, map: StructId) -> Core {
    // Bind the operand's core ONCE — the Poison check and the empty-`MapNew` check reuse the one result.
    let map_core = core_of(db, map);
    if let Core::Poison(r) = &map_core {
        return Core::Poison(r.clone());
    }
    // A compile-time-visible EMPTY constant map enumerates to the empty list — no descriptor, no key/value
    // type needed.
    if let Core::MapNew { entries, .. } = &map_core
        && entries.is_empty()
    {
        trace!(target: "rcdzc::fold", node = map.0, "Map.to-list folds an empty constant map to the empty list");
        return Core::ListNew {
            elems: std::rc::Rc::from([]),
        };
    }
    let Some((key_ty, val_ty)) = map_kv_types(db, map) else {
        let mismatch = !matches!(
            crate::infer::type_of(db, map),
            crate::ty::Ty::Map(_, _) | crate::ty::Ty::Var(_) | crate::ty::Ty::Any
        );
        return ill_typed_operand_decline(mismatch, "Map.to-list operand is not a solved map type");
    };
    // A NON-EMPTY CONSTANT map folds to a baked list of `(key value)` TUPLES in canonical KEY order — the SAME
    // order the runtime `map-to-list` op produces (spec-pinned canonical value order == `const_key_order`, a
    // v-runtime contract; the Map twin of the Set.to-list fold #3765). Sort the entries by KEY via
    // `const_key_order`; an entry whose key the canonical order cannot rank as a constant (float / bytes /
    // nested-collection / a runtime key — `const_key_order` returns `None`, EXACTLY the classes the runtime op
    // declines too) keeps the runtime op. The map already holds each key at most once (the `Map.insert` fold
    // replaced-by-key), so this only reorders. Each entry materializes as a `Core::Tuple { [key, value] }`
    // typed `(Tuple key_ty val_ty)` — the runtime op's `(List (Tuple K V))` element shape.
    if let Core::MapNew { entries, .. } = &map_core {
        let mut sorted: Vec<(StructId, StructId)> = entries.to_vec();
        // Every KEY must be individually canonically-orderable — a `sort_by` over a 0/1-entry map never calls
        // the comparator, so probe each key against itself (else a lone non-orderable key would materialize,
        // diverging from the runtime op which declines it).
        let mut orderable = sorted
            .iter()
            .all(|&(k, _)| const_key_order(db, k, k).is_some());
        sorted.sort_by(|a, b| {
            const_key_order(db, a.0, b.0).unwrap_or_else(|| {
                orderable = false;
                std::cmp::Ordering::Equal
            })
        });
        if orderable {
            let tuple_ty =
                crate::ty::Ty::Tuple(std::rc::Rc::from([key_ty.clone(), val_ty.clone()]));
            let elems: Vec<StructId> = sorted
                .into_iter()
                .map(|(k, v)| {
                    synth_core(
                        db,
                        Core::Tuple {
                            elems: std::rc::Rc::from([k, v]),
                        },
                        tuple_ty.clone(),
                    )
                })
                .collect();
            trace!(target: "rcdzc::fold", node = map.0, n = elems.len(), "Map.to-list folds a constant map to a key-sorted list of (k v) tuples");
            return Core::ListNew {
                elems: elems.into(),
            };
        }
    }
    // A KEY type with NO total order cannot be enumerated in canonical key order — a float leaf (only the
    // IEEE partial order) or a set/map leaf (no blessed order). Decline in the shared front-end so both
    // backends + `cdz check` inherit ONE coded CDZ0203 verdict (ALL-LEAF — the reconcile family). Only the
    // KEY needs an order; a float/un-orderable VALUE rides along untouched (19-sets "…float values … ride
    // along"), so this checks `key_ty`, not `val_ty`. A BARE float key still enumerates (canonical bytes).
    if !orderable_leaf_or_compound(db, &key_ty, /*float_ok=*/ true, &mut Vec::new()) {
        return Core::Poison(to_list_unorderable_reject("key"));
    }
    Core::MapToList {
        map,
        key_ty,
        val_ty,
    }
}

/// Lower `(Set.insert set elem)` / `(Set.remove set elem)`. FOLD onto a constant set (`Core::SetOf`) when
/// the element is constant: insert appends (no-op if already present, by value); remove drops the matching
/// element (no-op if absent). Else emit the runtime `Core::SetInsert`/`Core::SetRemove`. The element type
/// comes from the RESULT node's solved `Ty::Set`. A poison propagates.
pub(super) fn lower_set_insert_remove(
    db: &mut Db,
    prim: crate::resolved::Prim,
    set: StructId,
    elem: StructId,
) -> Core {
    use crate::resolved::Prim;
    // Bind the set operand's core ONCE — `core_of` is a non-trivial (memoized) lowering pass, so the Poison
    // check and the constant-`SetOf` fold below reuse the one result rather than re-cloning it (the sibling
    // of the Copilot PR #415 fix already applied to `lower_set_to_list`).
    let set_core = core_of(db, set);
    if let Core::Poison(r) = &set_core {
        return Core::Poison(r.clone());
    }
    if let Core::Poison(r) = core_of(db, elem) {
        return Core::Poison(r);
    }
    let is_insert = prim == Prim::SetInsert;
    // FOLD only when the ENTIRE set is constant (see `lower_set_contains`): a `SetOf` carrying a RUNTIME
    // element cannot be folded against, because `const_compound_eq(runtime, const)` is `None`. Folding
    // then MISCOMPILES both ops: a `remove` would RETAIN a runtime element that equals the query (it is
    // not `Some(true)`-equal to the const), so `(Set.len (Set.remove (Set.of (list (rep …))) <twin>))`
    // stayed 1 instead of 0; an `insert` of an element equal to a runtime element would wrongly add a
    // duplicate (the const probe misses the runtime twin), inflating the cardinality. A set with any
    // non-constant element must run the real champ op (`Core::SetInsert`/`SetRemove`).
    if let Core::SetOf { elems, elem_ty } = &set_core
        && is_const_value(db, set)
        && is_const_value(db, elem)
    {
        let mut out: Vec<StructId> = elems.to_vec();
        if is_insert {
            if !set_has_const_elem(db, &out, elem) {
                out.push(elem); // add-if-absent (a present element is a no-op value)
            }
        } else {
            out.retain(|&e| const_compound_eq(db, e, elem) != Some(true)); // drop the matching element
        }
        trace!(target: "rcdzc::fold", elems = out.len(), insert = is_insert, "Set.insert/remove folds onto a constant set");
        return Core::SetOf {
            elems: out.into(),
            elem_ty: elem_ty.clone(),
        };
    }
    let Some(elem_ty) = set_elem_type(db, set) else {
        let mismatch = !matches!(
            crate::infer::type_of(db, set),
            crate::ty::Ty::Set(_) | crate::ty::Ty::Var(_) | crate::ty::Ty::Any
        );
        return ill_typed_operand_decline(
            mismatch,
            "Set.insert/remove operand is not a solved set type",
        );
    };
    if is_insert {
        Core::SetInsert { set, elem, elem_ty }
    } else {
        Core::SetRemove { set, elem, elem_ty }
    }
}

/// Lower `(Set.union a b)` / `intersection` / `difference`. FOLD two constant sets (`Core::SetOf`) to a
/// canonical constant result set (by-value element algebra, `const_compound_eq`); else emit the runtime
/// `Core::SetAlgebra`. A poison propagates.
pub(super) fn lower_set_algebra(
    db: &mut Db,
    prim: crate::resolved::Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    use crate::core::SetAlgebraOp;
    use crate::resolved::Prim;
    // Bind each operand's core ONCE — the Poison check and the two-constant-`SetOf` fold below reuse the one
    // result rather than re-cloning it (the sibling of the Copilot PR #415 fix on `lower_set_to_list`).
    let lhs_core = core_of(db, lhs);
    if let Core::Poison(r) = &lhs_core {
        return Core::Poison(r.clone());
    }
    let rhs_core = core_of(db, rhs);
    if let Core::Poison(r) = &rhs_core {
        return Core::Poison(r.clone());
    }
    let op = match prim {
        Prim::SetUnion => SetAlgebraOp::Union,
        Prim::SetIntersection => SetAlgebraOp::Intersection,
        _ => SetAlgebraOp::Difference,
    };
    // FOLD two SetOf operands ONLY when every element of BOTH is a compile-time constant. A runtime
    // element (a parameter, a call result) makes `set_has_const_elem` report it ABSENT (its
    // `const_compound_eq` is `None`) — so union would keep a spurious duplicate, and
    // intersection/difference would drop or keep the wrong element — and `lower_set_of` leaves a
    // runtime-element `SetOf` un-dedup'd, so the folded result's cardinality is wrong too. When either
    // side carries a non-constant element the fold is declined and the runtime `Core::SetAlgebra` (below)
    // operates on two canonical CHAMP handles correctly (the same protection the equality fold and the
    // `MapNew` folds apply to their runtime elements/keys).
    if let (Core::SetOf { elems: a, elem_ty }, Core::SetOf { elems: b, .. }) =
        (&lhs_core, &rhs_core)
        && a.iter()
            .chain(b.iter())
            .all(|&e| const_compound_eq(db, e, e) == Some(true))
    {
        let out: Vec<StructId> = match op {
            // union: a's elements, then b's elements not already present.
            SetAlgebraOp::Union => {
                let mut out = a.to_vec();
                for &e in b.iter() {
                    if !set_has_const_elem(db, &out, e) {
                        out.push(e);
                    }
                }
                out
            }
            // intersection: a's elements that are also in b.
            SetAlgebraOp::Intersection => a
                .iter()
                .copied()
                .filter(|&e| set_has_const_elem(db, b, e))
                .collect(),
            // difference: a's elements NOT in b.
            SetAlgebraOp::Difference => a
                .iter()
                .copied()
                .filter(|&e| !set_has_const_elem(db, b, e))
                .collect(),
        };
        trace!(target: "rcdzc::fold", ?op, elems = out.len(), "set-algebra folds two constant sets");
        return Core::SetOf {
            elems: out.into(),
            elem_ty: elem_ty.clone(),
        };
    }
    Core::SetAlgebra { op, lhs, rhs }
}

/// Lower `(Map.insert map key val)` — add-or-replace, returning the new map. For M1 this emits the
/// runtime `Core::MapInsert` on a runtime map operand (a constant-map fold is a later increment). The
/// key/value types come from the map operand's `Ty::Map` (they choose the box ops). A poison propagates.
pub(super) fn lower_map_insert(db: &mut Db, id: StructId, args: &[StructId]) -> Core {
    let (map, key, val) = (args[0], args[1], args[2]);
    for &a in &[map, key, val] {
        if let Core::Poison(r) = core_of(db, a) {
            return Core::Poison(r);
        }
    }
    // The key/value types come from the INSERT NODE's own solved type `Map k v` (the RESULT map),
    // which unification has fully determined — NOT from the map OPERAND, whose isolated type may still
    // be `Map ?0 ?1` for a bare `Map.empty` (its key/value are solved only via this insert's arguments).
    let Some((key_ty, val_ty)) = map_kv_types(db, id) else {
        return Core::Poison(Reject::decline(
            "Map.insert result is not a solved map type",
        ));
    };
    // FOLD onto a compile-time-visible constant map when the KEY is a constant (its value need not be —
    // the value occurrence carries over regardless, exactly as `List.push` folds onto a constant list).
    // Add-or-REPLACE by key VALUE: an existing entry whose key is `const_compound_eq` to the new key has
    // its value replaced IN PLACE (preserving position — the each-key-at-most-once rule); otherwise the
    // entry is appended. The result is a constant `Core::MapNew` that bakes at escape / renders sorted /
    // compares by `value-eq`, so a chain `(Map.insert (Map.insert Map.empty 2 20) 1 10)` folds to one
    // canonical two-entry map. A runtime map operand or a runtime key stays a `Core::MapInsert` (the
    // persistent CHAMP op). Keys compared by VALUE (`const_compound_eq`), so two names bound to the same
    // value collapse here just as they do at run time.
    if let (Core::MapNew { entries, .. }, true) = (core_of(db, map), is_const_value(db, key)) {
        let mut merged = entries.to_vec();
        let mut replaced = false;
        for e in merged.iter_mut() {
            if const_compound_eq(db, e.0, key) == Some(true) {
                *e = (e.0, val); // replace the value at this key (keep the key occurrence + position)
                replaced = true;
                break;
            }
        }
        if !replaced {
            merged.push((key, val));
        }
        trace!(target: "rcdzc::fold", node = id.0, entries = merged.len(), "Map.insert folds onto a constant map");
        return Core::MapNew {
            entries: merged.into(),
            key_ty,
            val_ty,
        };
    }
    Core::MapInsert {
        map,
        key,
        val,
        key_ty,
        val_ty,
    }
}

/// Lower a MAP PATTERN binder reference — read from the scrutinee by key. `key = Some(k)` is a VALUE
/// binder at key `k` → the entry's value core; `key = None` is the REST binder → the map with the `named`
/// keys removed. A CONSTANT `Core::MapNew` scrutinee folds (the corpus shape: an inline `Map.insert`
/// chain); a RUNTIME scrutinee reads at run time via `lower_map_field_runtime` (a value binder emits
/// `Map.lookup`, a rest binder a `Map.remove` chain). The arm was already SELECTED by `lower_match_map`
/// (which ran the same key-presence probe), so a value binder's key IS present here; a defensive miss
/// declines rather than miscompiling.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_map_field(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    key: Option<StructId>,
    named: &[StructId],
    value_steps: &[crate::core::PathStep],
    value_heads: &[StructId],
) -> Core {
    // Reach the matched MAP core: the scrutinee DIRECTLY (empty path — a direct map match), or a NESTED map
    // at `path` inside a constant tuple/list scrutinee (`fold_sum_path` folds the `Elem` steps to the
    // nested `Core::MapNew`).
    let map_core = if path.is_empty() {
        core_of(db, scrutinee)
    } else {
        match fold_sum_path(db, scrutinee, path) {
            Some(c) => c,
            None => {
                // A nested map inside a RUNTIME/non-constant compound — the runtime nested-map read is not
                // yet wired (the direct runtime path below reads the whole `scrutinee`, not a sub-path).
                return Core::Poison(Reject::unsupported(
                    "matching a nested map pattern over a runtime/non-constant scrutinee is not supported",
                ));
            }
        }
    };
    // A `MapNew` whose keys/values are NOT all compile-time constants (`(map ((add 2 3) 42))` — a runtime
    // key) is a RUNTIME map for matching purposes: `const_compound_eq` cannot decide its key presence, so the
    // fold below would report a present runtime key ABSENT and mis-select an arm (the miscompile
    // `desugar_runtime_map_match` guards by routing such a scrutinee to the runtime presence-chain). At the
    // DIRECT-match (empty path) the runtime VALUE/REST read applies to the `MapNew` value verbatim (it builds
    // a real CHAMP map at run time, so `Map.lookup`/`Map.remove` over it work). So a non-constant `MapNew` at
    // an empty path reads via `lower_map_field_runtime`, not the const fold — treat it like any runtime map.
    if matches!(&map_core, Core::MapNew { .. }) && path.is_empty() && !is_const_value(db, scrutinee)
    {
        return lower_map_field_runtime(db, id, scrutinee, key, named, value_steps, value_heads);
    }
    let Core::MapNew { entries, .. } = map_core else {
        // A RUNTIME map scrutinee (not a compile-time-constant `MapNew`). Only the DIRECT map match (empty
        // path) has the runtime read wired: the arm was SELECTED by the runtime presence-test `if`-chain
        // `desugar_runtime_map_match` built, so control is here ONLY when every named key IS present — a
        // VALUE binder reads its key (`Map.lookup` then unwrap the `Some`, safe: the key is present) and the
        // REST binder reads the map minus the named keys (a `Map.remove` chain), both synthesized as SOURCE
        // forms + re-lowered via `core_of`. A NESTED runtime map (non-empty path) is a further increment.
        if path.is_empty() {
            return lower_map_field_runtime(
                db,
                id,
                scrutinee,
                key,
                named,
                value_steps,
                value_heads,
            );
        }
        return Core::Poison(Reject::unsupported(
            "matching a nested map pattern over a runtime map scrutinee is not supported (constant map only)",
        ));
    };
    match key {
        // A VALUE binder — the value at key `k` (keys compared by value, `const_compound_eq`). When the
        // value binder is NESTED inside a value sub-pattern (`(map ("a" (tuple x y)))`), `value_steps` walks
        // INTO that value to the binder — folded over the constant value via `fold_sum_path` (`(tuple 3 4)`
        // at `Elem(0)` folds to `3`), exactly as a nested tuple/payload binder folds over its scrutinee.
        Some(k) => {
            let mut all_misses_decidable = true;
            for (ek, ev) in entries.iter() {
                match const_compound_eq(db, *ek, k) {
                    Some(true) => {
                        if value_steps.is_empty() {
                            return core_of(db, *ev);
                        }
                        let ev = *ev;
                        return match fold_sum_path(db, ev, value_steps) {
                            Some(c) => c,
                            None => Core::Poison(Reject::unsupported(
                                "matching a nested value sub-pattern over a runtime value in a constant map is not supported",
                            )),
                        };
                    }
                    Some(false) => {} // decidably NOT this key — keep scanning
                    None => all_misses_decidable = false, // undecidable (a runtime key) — can't prove absence
                }
            }
            if all_misses_decidable {
                // The key is PROVABLY absent from this constant-KEYED map (every entry key compared decidably
                // unequal). A value binder for a definitively-absent key is only ever lowered in a DEAD arm:
                // every caller gates key presence first — the direct map matcher skips an arm whose key is
                // absent (so it never calls here), and a nested `(list (map (k v)…) …)` element is desugared
                // with a key-PRESENCE guard (`desugar_refutable_map_list_elements`). When that element's map
                // has a RUNTIME value the presence guard is a runtime `Map.lookup` test, so the guarded arm's
                // body is kept in the `MatchList` and its binder Core is lowered EAGERLY even though the guard
                // gates it false at run time. Emit a divergent `Core::Trap` (well-typed, never executed under
                // the guard) rather than a hard Poison that fails compilation of a VALID program — the nested
                // const-keyed sibling of the top-level runtime fall-through (#5450), and the map twin of
                // `lower_map_field_runtime`'s guaranteed-present `None → trap` dead branch.
                return Core::Trap;
            }
            // A non-constant (runtime) KEY left presence undecidable — the runtime map matcher should have
            // routed this; a defensive decline rather than risk a mis-selection.
            Core::Poison(Reject::decline(
                "a map pattern value binder's key is absent from the constant map (arm mis-selected)",
            ))
        }
        // The REST binder — the map with every `named` key removed. Its key/value types come from this
        // binder node's own solved `Ty::Map` (the scrutinee's map type). Build a fresh constant `MapNew`.
        None => {
            let (key_ty, val_ty) = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Map(k, v) => (*k, *v),
                _ => (crate::ty::Ty::Any, crate::ty::Ty::Any),
            };
            let rest: Vec<(StructId, StructId)> = entries
                .iter()
                .filter(|(ek, _)| {
                    !named
                        .iter()
                        .any(|&nk| const_compound_eq(db, *ek, nk) == Some(true))
                })
                .copied()
                .collect();
            Core::MapNew {
                entries: rest.into(),
                key_ty,
                val_ty,
            }
        }
    }
}

/// The RUNTIME arm of `lower_map_field`: read a map-pattern binder off a RUNTIME map scrutinee. The arm was
/// selected by the presence-test `if`-chain `desugar_runtime_map_match` built, so every named key IS
/// present when control reaches the body. A VALUE binder at key `k` reads `(match (Map.lookup scrutinee k)
/// ((Some x) x) ((None) (trap …)))` — the `Some` is guaranteed by the presence test, the `None → trap` is
/// dead but keeps the match exhaustive; the REST binder reads `(Map.remove (Map.remove scrutinee k1) …)`
/// (the map minus every named key). Both are synthesized as SOURCE forms + lowered via `core_of` after
/// `resolve_subtree` — a source-written `Map.lookup`/`Map.remove` over a runtime map already compiles, and
/// re-lowering grounds the synthesized nodes' types (the Inc-11/12/14 discipline that unblocked Inc-9's
/// "synthesized generic app not grounded at emit").
#[allow(clippy::too_many_arguments)]
/// Synthesize a SOURCE expression reading `base` (a binder name) down the value sub-path `steps` — the
/// runtime companion of `fold_sum_path` for a map value sub-pattern binder over a RUNTIME value. An
/// `Elem(i)` step is a tuple projection `(. acc i)`; a `Payload` step (head from the `heads` queue, in
/// order) extracts a variant's sole payload via a nested `(match acc ((<head> __p) __p) (_ (trap …)))` — the
/// match is safe (control reached here because the arm's keys are present AND, for an IRREFUTABLE value
/// sub-pattern, the ctor is single-variant so the `_ → trap` is dead but keeps the match well-formed). An
/// empty `steps` reads `base` directly. The result re-resolves + lowers via the caller's
/// `resolve_subtree`/`core_of`.
pub(super) fn synth_value_path_read(
    db: &mut Db,
    base: &str,
    steps: &[crate::core::PathStep],
    heads: &[StructId],
) -> StructId {
    let mut acc = db.push_name(base);
    let mut heads_it = heads.iter();
    for step in steps {
        match step {
            crate::core::PathStep::Elem(i) => {
                // `(. acc <i>)` — a tuple projection at an integer key.
                let dot = db.push_name(".");
                let idx = db.push_atom(crate::ast::Leaf::Int {
                    value: crate::ast::IntValue::from_u128(*i as u128),
                    radix: crate::ast::Radix::Dec,
                });
                acc = db.push_list(vec![dot, acc, idx]);
            }
            crate::core::PathStep::Payload => {
                // `(match acc ((<head> __p) __p) (_ (trap …)))` — extract the variant's sole payload. The
                // head occurrence is copied (a fresh `(. Sum V)` / bare-name pattern head) so it re-resolves
                // as a ctor PATTERN, not the original expression-context node.
                let head = match heads_it.next() {
                    Some(&h) => clone_ctor_head(db, h),
                    None => return acc, // malformed (fewer heads than Payload steps) — read what we have
                };
                let p_binder = db.push_name("__pp");
                let ctor_pat = db.push_list(vec![head, p_binder]);
                let p_ref = db.push_name("__pp");
                let arm = db.push_list(vec![ctor_pat, p_ref]);
                let wild = db.push_name("_");
                let trap_head = db.push_name("trap");
                let trap_msg = db.push_str("unreachable: value sub-pattern ctor already matched");
                let trap = db.push_list(vec![trap_head, trap_msg]);
                let else_arm = db.push_list(vec![wild, trap]);
                let match_head = db.push_name("match");
                acc = db.push_list(vec![match_head, acc, arm, else_arm]);
            }
            crate::core::PathStep::RestFrom(_) => {
                // A list rest inside a map value is not produced here (value sub-patterns descend tuple/ctor;
                // a nested list value declines upstream) — leave `acc` unchanged defensively.
            }
            crate::core::PathStep::TupleRestFrom(_) => {
                // A tuple rest inside a map value is not produced here (same as the list `RestFrom` above) —
                // leave `acc` unchanged defensively.
            }
        }
    }
    acc
}

/// A FRESH copy of a constructor-pattern HEAD occurrence `h` — a `(. Sum V)` member form or a bare variant
/// name — for reuse as a ctor pattern head in a synthesized value-path read. A `.`-member form is rebuilt
/// from fresh copies of its segments; a bare name is re-pushed. Falls back to reusing `h` for any other
/// shape (never happens for a real ctor head).
/// A copy of a whole constructor PATTERN `(C.V p…)` / `(. Sum V)` / bare-name with its HEAD rebuilt fresh
/// (via [`clone_ctor_head`]) while REUSING its payload sub-patterns. Used when re-parenting a ctor element
/// pattern into a synthesized inner match arm: the original head was resolved in list-element position
/// (inert), so it must re-resolve as a variant reference from scratch, but the payload sub-patterns
/// (`x` in `(Some x)`) re-resolve cleanly on their own once the subtree is forgotten. A bare member
/// `(. Sum V)` used whole, or a bare name, has no payloads — just clone the head.
pub(super) fn clone_ctor_pattern_head(db: &mut Db, ctor_pat: StructId) -> StructId {
    match db.ast.get(ctor_pat) {
        crate::ast::Struct::List(children) => {
            let children = children.clone();
            match children.first().copied() {
                // A bare member `(. Sum V)` used whole — the whole thing IS the head; clone it.
                Some(first) if db.ast.as_name(first) == Some(".") => clone_ctor_head(db, ctor_pat),
                // An applied ctor `(head p…)` — fresh head, reused payload sub-patterns.
                Some(head) => {
                    let fresh_head = clone_ctor_head(db, head);
                    let mut new_children = vec![fresh_head];
                    new_children.extend(children[1..].iter().copied());
                    db.push_list(new_children)
                }
                None => ctor_pat,
            }
        }
        // A bare-name nullary ctor — clone it fresh.
        crate::ast::Struct::Atom(_) => clone_ctor_head(db, ctor_pat),
    }
}

pub(super) fn clone_ctor_head(db: &mut Db, h: StructId) -> StructId {
    if let Some(seg) = db.ast.as_form(h, ".").map(<[_]>::to_vec) {
        let dot = db.push_name(".");
        let mut children = vec![dot];
        for s in seg {
            // Each segment is a bare name (`Sum`, `V`) — copy it.
            match db.ast.as_name(s) {
                Some(nm) => {
                    let n = nm.to_string();
                    children.push(db.push_name(&n));
                }
                None => children.push(s),
            }
        }
        return db.push_list(children);
    }
    match db.ast.as_name(h) {
        Some(nm) => {
            let n = nm.to_string();
            db.push_name(&n)
        }
        None => h,
    }
}

pub(super) fn lower_map_field_runtime(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    key: Option<StructId>,
    named: &[StructId],
    value_steps: &[crate::core::PathStep],
    value_heads: &[StructId],
) -> Core {
    // Helper: `((. Map <op>) args…)`.
    fn map_op(db: &mut Db, op: &str, args: &[StructId]) -> StructId {
        let dot = db.push_name(".");
        let map_mod = db.push_name("Map");
        let op_key = db.push_name(op);
        let member = db.push_list(vec![dot, map_mod, op_key]);
        let mut call = vec![member];
        call.extend_from_slice(args);
        db.push_list(call)
    }
    match key {
        // VALUE binder at key `k`: `(match (Map.lookup scrutinee k) ((Some __mv) <read>) ((None) (trap …)))`.
        // The Some-arm binds the value to `__mv`; `<read>` is `__mv` for a bare value binder, or `__mv`
        // walked down `value_steps` for a NESTED binder (`(map ("a" (tuple x y)))` reads `(. __mv 0)`;
        // `(map ("a" (Box.Mk n)))` reads the payload via a nested `(match __mv ((Box.Mk __p) __p) …)`) —
        // synthesized as SOURCE by `synth_value_path_read`. `Some` is guaranteed by the presence test, the
        // `None → trap` is dead but keeps the match exhaustive.
        Some(k) => {
            let k_copy = clone_key_expr(db, k);
            let lookup = map_op(db, "lookup", &[scrutinee, k_copy]);
            let some_head = db.push_name("Some");
            let x_binder = db.push_name("__mv");
            let some_pat = db.push_list(vec![some_head, x_binder]);
            // The Some-arm body: read `__mv` down the value sub-path (empty steps = bare `__mv`).
            let read = synth_value_path_read(db, "__mv", value_steps, value_heads);
            let some_arm = db.push_list(vec![some_pat, read]);
            // `((None) (trap …))` — dead (presence proven) but keeps the match exhaustive.
            let none_head = db.push_name("None");
            let none_pat = db.push_list(vec![none_head]);
            let trap_head = db.push_name("trap");
            let trap_msg = db.push_str("unreachable: map key absent after presence test");
            let trap = db.push_list(vec![trap_head, trap_msg]);
            let none_arm = db.push_list(vec![none_pat, trap]);
            let match_head = db.push_name("match");
            let rewritten = db.push_list(vec![match_head, lookup, some_arm, none_arm]);
            crate::resolve::resolve_subtree(db, rewritten);
            // Carry the binder's KNOWN type onto the synthesized read (the nested binder's own solved type
            // — `id`'s type), so the emit path has a grounded result type. Only a ground type sticks.
            let vty = crate::infer::type_of(db, id);
            if !matches!(vty, crate::ty::Ty::Any) && !vty.has_free_var() {
                db.types.fill(rewritten, vty);
            }
            core_of(db, rewritten)
        }
        // REST binder: `(Map.remove (Map.remove scrutinee k1) k2 …)` — the map minus every named key.
        None => {
            let mut acc = scrutinee;
            for &nk in named {
                let k_copy = clone_key_expr(db, nk);
                acc = map_op(db, "remove", &[acc, k_copy]);
            }
            crate::resolve::resolve_subtree(db, acc);
            let mty = crate::infer::type_of(db, id);
            if !matches!(mty, crate::ty::Ty::Any) && !mty.has_free_var() {
                db.types.fill(acc, mty);
            }
            core_of(db, acc)
        }
    }
}

pub(crate) fn is_const_value(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::ConstChar(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstFloat(_)
        | Core::Unit => true,
        Core::Tuple { elems } | Core::ListNew { elems } => {
            elems.iter().all(|&e| is_const_value(db, e))
        }
        Core::SumNew { payloads, .. } => payloads.iter().all(|&p| is_const_value(db, p)),
        Core::Record { fields } => fields.values().all(|&v| is_const_value(db, v)),
        Core::MapNew { entries, .. } => entries
            .iter()
            .all(|&(k, v)| is_const_value(db, k) && is_const_value(db, v)),
        Core::SetOf { elems, .. } => elems.iter().all(|&e| is_const_value(db, e)),
        // A `BytesOf` built from constant integer elements is a compile-time-constant bytes value (a
        // `b"…"` literal / `Bytes.of` of constants), exactly like `ConstBytes` — a constant that a const
        // list-fold may carry (e.g. a reflected/quoted module source containing a `b"…"` literal, or a
        // transform that builds tagged bytes with `Bytes.concat(b"\x01", …)`). Without this, a const list
        // whose elements include such a literal was wrongly deemed non-const, so the recursive const-fold
        // unroll declined (`Ast.encode of a runtime AST value`).
        Core::BytesOf { elems } => elems.iter().all(|&e| is_const_value(db, e)),
        _ => false,
    }
}

/// Lower `(Map.lookup map key)` — the fallible keyed read → `(Option v)`. Emits the runtime
/// `Core::MapLookup` (a NULL-or-handle test building `Some`/`None`). The result Option's discriminants
/// are read off the node's result type; the key/value types off the map operand. A poison propagates.
pub(super) fn lower_map_lookup(db: &mut Db, id: StructId, map: StructId, key: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, map) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, key) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Map.lookup result is not the built-in Option sum",
        ));
    };
    let Some((key_ty, val_ty)) = map_kv_types(db, map) else {
        let mismatch = !matches!(
            crate::infer::type_of(db, map),
            crate::ty::Ty::Map(_, _) | crate::ty::Ty::Var(_) | crate::ty::Ty::Any
        );
        return ill_typed_operand_decline(mismatch, "Map.lookup operand is not a solved map type");
    };
    Core::MapLookup {
        map,
        key,
        key_ty,
        val_ty,
        disc_some,
        disc_none,
    }
}

/// Lower `(Map.remove map key)` — drop a key's association, returning the new map. Emits the runtime
/// `Core::MapRemove`. The key type comes from the map operand's `Ty::Map` (for the box op). A poison
/// propagates.
pub(super) fn lower_map_remove(db: &mut Db, map: StructId, key: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, map) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, key) {
        return Core::Poison(r);
    }
    let Some((key_ty, _)) = map_kv_types(db, map) else {
        let mismatch = !matches!(
            crate::infer::type_of(db, map),
            crate::ty::Ty::Map(_, _) | crate::ty::Ty::Var(_) | crate::ty::Ty::Any
        );
        return ill_typed_operand_decline(mismatch, "Map.remove operand is not a solved map type");
    };
    Core::MapRemove { map, key, key_ty }
}

/// Lower `(String.at string index)` — the fallible SCALAR-indexed read. FOLD when both operands are
/// constant: index the string by UNICODE SCALAR position (`chars().nth`, NOT byte offset —
/// collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values), yielding `(Some
/// "<char>")` in range (the ONE-scalar string at that position, a fresh `Core::ConstStr` synthesized
/// into the arena) and `None` out (negative, or `>=` the scalar length). Builds a `Core::SumNew` at the
/// result Option's Some/None discriminants, so it rides the ordinary sum fold/escape/match — no string
/// heap. A runtime string declines (the byte-rope indexed read is a later increment). A poison
/// operand propagates.
/// Lower `(Char.from-int n)` — the FALLIBLE integer→char conversion `Int64 → (Option Char)`. FOLD a
/// constant integer: a value that IS a Unicode scalar (in `0..=0x10FFFF`, not a surrogate `0xD800..=
/// 0xDFFF`) → `(Some #\c)` (a fresh `Leaf::Char` payload, the shape `String.at` uses for its scalar);
/// a surrogate / out-of-range integer → `(None unit)`. Never traps (`collections-and-text.md` §A Char
/// Converts To And From An Integer Totally). A runtime operand declines (no runtime char rep yet); a
/// poison propagates. `char::from_u32` performs the exact scalar-validity test.
pub(super) fn lower_char_from_int(db: &mut Db, id: StructId, n: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, n) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Char.from-int result is not the built-in Option sum",
        ));
    };
    match core_of(db, n) {
        Core::ConstInt(v) => {
            // A scalar iff the value fits u32 AND `char::from_u32` accepts it (excludes surrogates and
            // > U+10FFFF). A negative or > u32 value is trivially not a scalar → None.
            let scalar = v
                .to_i64()
                .and_then(|i| u32::try_from(i).ok())
                .and_then(char::from_u32);
            match scalar {
                Some(c) => {
                    trace!(target: "rcdzc::fold", node = id.0, "Char.from-int folds to Some (a valid scalar)");
                    let payload = db.push_atom(crate::ast::Leaf::Char(c));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload].into(),
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "Char.from-int folds to None (surrogate / out-of-range)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    }
                }
            }
        }
        // A RUNTIME integer (a param/local/`if`-join): emit the checked conversion node. The backend
        // range-checks n against the Unicode-scalar domain and wraps the i32 code point into `Some`/`None`
        // (the runtime companion of the constant fold above; Char-rep 4/N made a runtime `Char` a boxable
        // `Some` payload, which is what this yields on success).
        _ => Core::IntToCharChecked {
            operand: n,
            disc_some,
            disc_none,
        },
    }
}

/// Lower `Value.decode b` (R2) — the in-fold binary-AST value-form DECODE `∀a. Bytes → (Option a)`. The
/// target type `a` is the node's solved type peeled from `(Option a)` (typing declines an unsolved `a` at
/// the decode node, so a decode node reaching lowering has a concrete `a`). Emits `Core::ValueDecode {
/// bytes, desc, disc_some, disc_none }` — the runtime `value-decode(b, desc)` op — with `desc` the framed
/// `(: value Type)` descriptor for `a` (the SAME descriptor `Value.encode` writes, so a round-trip agrees).
/// The backend wraps the op's success handle (or its failure signal) into the `(Option a)` sum. Declines if
/// the node's type is not the built-in `(Option a)`, or `a` has no value-form descriptor.
pub(super) fn lower_value_decode(db: &mut Db, id: StructId, b: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, b) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Value.decode result is not the built-in Option sum",
        ));
    };
    // Peel `a` from the node's solved `(Option a)` type — the target type the bytes decode into.
    let node_ty = crate::infer::type_of(db, id);
    let crate::ty::Ty::Sum { args, .. } = &node_ty else {
        return Core::Poison(Reject::decline(
            "Value.decode result type is not a resolved (Option a) — the target type is unsolved",
        ));
    };
    let Some(target_ty) = args.first().cloned() else {
        return Core::Poison(Reject::decline(
            "Value.decode target type (Option's element) is unresolved",
        ));
    };
    match sum_shape_descriptor(db, &target_ty) {
        Some(desc) => Core::ValueDecode {
            bytes: b,
            desc: std::rc::Rc::from(desc.as_slice()),
            disc_some,
            disc_none,
        },
        // No descriptor. TWO distinct causes, distinguished for an ACTIONABLE diagnostic: an UNDETERMINED
        // target (`a` is still a free `Ty::Var` — an unannotated decode, or one whose target is only
        // implied by downstream match-arm patterns that don't thread their type back to the decode node)
        // has no descriptor because there is no type YET, and the fix is to ANNOTATE (`(: (Value.decode
        // bs) (Option T))` or a typed let-binder). A CONCRETE target with no value-form descriptor (e.g. a
        // function type) is genuinely unsupported. The old message ("no binary-AST value-form descriptor")
        // fired for BOTH and misled the free-var case into looking like an unsupported type. `has_free_var`
        // splits them so the common "you forgot the annotation" case gets the honest fix.
        None if target_ty.has_free_var() => Core::Poison(Reject::decline(
            "Value.decode target type is unsolved — annotate the decode with its expected type, e.g. \
             (: (Value.decode bs) (Option T)) or a typed let-binder (let (((: p (Option T)) …)) …)",
        )),
        None => Core::Poison(Reject::decline(
            "Value.decode into a type with no binary-AST value-form descriptor",
        )),
    }
}

pub(super) fn lower_str_at(db: &mut Db, id: StructId, string: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, string) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.at result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, index)) {
        (Core::ConstStr(s), Core::ConstInt(i)) => {
            // Index by scalar value; a negative index or one at/beyond the scalar length is out of range.
            let scalar = i.to_i64().and_then(|n| {
                if n >= 0 {
                    s.chars().nth(n as usize)
                } else {
                    None
                }
            });
            match scalar {
                Some(c) => {
                    // The one-scalar string at that position — a fresh `Leaf::Str` node whose `core_of`
                    // is `Core::ConstStr`, used as the `Some` payload (the same shape `List.at` uses,
                    // but the element is synthesized here since a string has no element sub-nodes).
                    trace!(target: "rcdzc::fold", node = id.0, "String.at folds to Some (in-bounds constant scalar index)");
                    let payload = db.push_atom(crate::ast::Leaf::Str(c.to_string().into()));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload].into(),
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.at folds to None (out-of-range constant index)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    }
                }
            }
        }
        // A RUNTIME string (or runtime index) — walk the UTF-8 byte buffer to the i-th scalar's byte span
        // and slice it (`Core::StrAt`). A String is a flat UTF-8 byte leaf, so the backend scans scalar
        // starts (a byte is a scalar START iff `(b & 0xC0) != 0x80`), skips `index` scalars, and slices the
        // scalar's byte span into the `Some` payload — matching the const `chars().nth`. Guarded on the
        // string operand being a definite `Ty::String` (the index is any integer).
        _ if matches!(crate::infer::type_of(db, string), crate::ty::Ty::String) => Core::StrAt {
            string,
            index,
            disc_some,
            disc_none,
        },
        // A non-String operand is a TYPE error (`infer` reports the authoritative CDZ0203); defer to it
        // via the neutral, coded-reject-deferring decline rather than an uncoded "needs a String operand".
        _ => runtime_string_op_decline(
            db,
            string,
            "String.at needs a String operand (its runtime read walks the UTF-8 buffer)",
        ),
    }
}

/// Lower `(String.scalar-at string index)` — the fallible read of the CHAR (single Unicode scalar) at a
/// scalar position `String → Int64 → (Option Char)`. The char-typed companion of `String.at`: identical
/// index logic (by Unicode SCALAR position, `chars().nth`, not byte), but the `Some` payload is a
/// `Leaf::Char` (the scalar itself), so the result is `(Option Char)` — folds to `(Some #\c)` in range /
/// `(None unit)` out (negative or at/beyond the scalar length). A runtime string declines; a poison
/// operand propagates.
pub(super) fn lower_str_scalar_at(
    db: &mut Db,
    id: StructId,
    string: StructId,
    index: StructId,
) -> Core {
    if let Core::Poison(r) = core_of(db, string) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.scalar-at result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, index)) {
        (Core::ConstStr(s), Core::ConstInt(i)) => {
            let scalar = i.to_i64().and_then(|n| {
                if n >= 0 {
                    s.chars().nth(n as usize)
                } else {
                    None
                }
            });
            match scalar {
                Some(c) => {
                    // The scalar at that position — a fresh `Leaf::Char` node (`core_of` = `Core::ConstChar`),
                    // the `Some` payload. Distinct from `String.at`, whose payload is a one-scalar `Leaf::Str`.
                    trace!(target: "rcdzc::fold", node = id.0, "String.scalar-at folds to Some (in-bounds constant scalar index)");
                    let payload = db.push_atom(crate::ast::Leaf::Char(c));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload].into(),
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.scalar-at folds to None (out-of-range constant index)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    }
                }
            }
        }
        // A runtime string/index: emit `Core::StrScalarAt` — the backend calls the runtime
        // `bytes-scalar-at(buf, scalar_index) -> u32` op (#5516), boxes the returned codepoint into a `Char`
        // (#5252 rep), and maps `u32::MAX -> None`, building the `(Option Char)`. The resulting runtime `Char`
        // is an i32 code-point (NO distinct runtime rep — char is int at runtime, like bool) and renders as a
        // `#\c` char literal via its `ShapeNode::Char` render tag (the bool-analog: bool is an i32 rendered
        // `true`/`false` via `ShapeNode::Bool`). The constant string+index case folds to a
        // `Some(Leaf::Char)`/`None` above and never reaches here.
        _ => Core::StrScalarAt {
            operand: string,
            index,
            disc_some,
            disc_none,
        },
    }
}

/// Lower `(String.slice string start end)` — the fallible SCALAR sub-range read, half-open `[start,
/// end)`. FOLD when all three operands are constant: cut the string by UNICODE SCALAR position (`chars`,
/// NOT byte offset — collections-and-text.md #A String Is A Sequence Of Unicode Scalar Values). The
/// range is well-defined only when `0 <= start <= end <= scalar-len`: then `(Some "<substr>")` (a fresh
/// `Core::ConstStr` of the selected scalars — `start == end` yields the empty string, present not None);
/// any bound outside that (reversed `end < start`, over-long `end > len`, or negative) yields `(None
/// unit)`. Builds a `Core::SumNew` at the result Option's discriminants, riding the ordinary sum
/// fold/escape/match — no string heap. A runtime string declines; a poison operand propagates.
pub(super) fn lower_str_slice(
    db: &mut Db,
    id: StructId,
    string: StructId,
    start: StructId,
    end: StructId,
) -> Core {
    for operand in [string, start, end] {
        if let Core::Poison(r) = core_of(db, operand) {
            return Core::Poison(r);
        }
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.slice result is not the built-in Option sum",
        ));
    };
    match (core_of(db, string), core_of(db, start), core_of(db, end)) {
        (Core::ConstStr(s), Core::ConstInt(a), Core::ConstInt(b)) => {
            let scalars: Vec<char> = s.chars().collect();
            let len = scalars.len() as i64;
            // The range is valid iff `0 <= start <= end <= scalar-len` (signed — a negative bound is out
            // of range, NOT wrapped to a large unsigned offset). `start == end` is an in-range empty slice.
            match (a.to_i64(), b.to_i64()) {
                (Some(a), Some(b)) if a >= 0 && a <= b && b <= len => {
                    let sub: String = scalars[a as usize..b as usize].iter().collect();
                    trace!(target: "rcdzc::fold", node = id.0, "String.slice folds to Some (in-range constant bounds)");
                    let payload = db.push_atom(crate::ast::Leaf::Str(sub.into()));
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload].into(),
                    }
                }
                _ => {
                    trace!(target: "rcdzc::fold", node = id.0, "String.slice folds to None (out-of-range constant bounds)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    }
                }
            }
        }
        // A RUNTIME string (or a runtime bound over any string) — walk the UTF-8 byte buffer to the
        // `start`-th and `end`-th scalar starts and slice that byte span (`Core::StrSlice`). Same flat-leaf
        // buffer walk `String.at` uses (a byte is a scalar START iff `(b & 0xC0) != 0x80`); the backend
        // slices `[p0, p1)` and COMPACTS it so the result compares by content. Guarded on the string operand
        // being a definite `Ty::String` (the bounds are any integers).
        _ if matches!(crate::infer::type_of(db, string), crate::ty::Ty::String) => Core::StrSlice {
            string,
            start,
            end,
            disc_some,
            disc_none,
        },
        // A non-String operand is a TYPE error (`infer` reports the authoritative CDZ0203); defer to it
        // via the neutral, coded-reject-deferring decline rather than an uncoded "needs a String operand".
        _ => runtime_string_op_decline(
            db,
            string,
            "String.slice needs a String operand (its runtime read walks the UTF-8 buffer)",
        ),
    }
}

/// Lower `(String.to-bytes s)` — the UTF-8 encoding `String → Bytes`. FOLD a constant string to a
/// `Core::BytesOf` whose elements are its UTF-8 bytes, each a fresh `UInt8` `Leaf::Int` synthesized into
/// the arena (the same shape `Bytes.of` of a byte-list builds — so it bakes at escape / consumes through
/// `Bytes.len`/`Bytes.at` identically, no string heap). A RUNTIME string emits `Core::StrToBytes` — the
/// encoding is TOTAL (a String IS a UTF-8 Bytes leaf), so it needs no conversion or validation, only a
/// flatten of the byte-rope to a canonical leaf (the runtime `bytes-compact` op), the exact inverse of the
/// runtime `str-from-bytes` decode. A poison operand propagates.
pub(super) fn lower_str_to_bytes(db: &mut Db, string: StructId) -> Core {
    match core_of(db, string) {
        Core::ConstStr(s) => {
            let elems: Vec<StructId> = s
                .as_bytes()
                .iter()
                .map(|&b| {
                    db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(b as i64),
                        radix: crate::ast::Radix::Dec,
                    })
                })
                .collect();
            trace!(target: "rcdzc::fold", len = elems.len(), "String.to-bytes folds a constant string to its UTF-8 bytes");
            Core::BytesOf {
                elems: elems.into(),
            }
        }
        Core::Poison(r) => Core::Poison(r),
        _ => Core::StrToBytes { string },
    }
}

/// Lower `(String.from-bytes b)` — the TOTAL UTF-8 decode `Bytes → (Option String)`. FOLD a
/// compile-time-visible constant `Bytes.of` (each element a constant `UInt8`) by strict UTF-8
/// (`std::str::from_utf8`, which rejects INVALID bytes, OVERLONG encodings, AND surrogate code points —
/// exactly the three failure modes the spec pins): well-formed → `(Some "<decoded>")` (a fresh
/// `Core::ConstStr` payload), ill-formed → `(None unit)` — built as a `Core::SumNew` at the result
/// Option's discs (`option_discs`, like `List.at`/`String.at`), riding the ordinary sum fold/escape/
/// match, no string heap. A runtime `Bytes` declines; a poison operand propagates. Never a trap — an
/// ill-formed sequence is DATA (`None`), the whole point of the total decode.
pub(super) fn lower_str_from_bytes(db: &mut Db, id: StructId, bytes: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "String.from-bytes result is not the built-in Option sum",
        ));
    };
    // Collect the raw bytes of a compile-time-visible `Bytes.of`; a runtime Bytes emits the runtime
    // `str-from-bytes` op (strict UTF-8 validate + consume/re-tag) wrapped into the Option sum.
    let Core::BytesOf { elems } = core_of(db, bytes) else {
        return Core::StrFromBytes {
            bytes,
            disc_some,
            disc_none,
        };
    };
    let mut raw = Vec::with_capacity(elems.len());
    for e in elems.iter().copied() {
        match core_of(db, e) {
            Core::ConstInt(v) => match v.to_i64() {
                Some(n) if (0..=255).contains(&n) => raw.push(n as u8),
                // A non-UInt8 element can't occur in a well-formed `Bytes.of` (range-checked at build),
                // but be defensive — decline rather than mis-decode.
                _ => {
                    return Core::Poison(Reject::decline(
                        "String.from-bytes: a byte element is not a UInt8",
                    ));
                }
            },
            // A `Bytes.of` with a RUNTIME byte element is itself a runtime Bytes value — route to the
            // runtime `str-from-bytes` op (which validates the materialized buffer) rather than fold.
            _ => {
                return Core::StrFromBytes {
                    bytes,
                    disc_some,
                    disc_none,
                };
            }
        }
    }
    // Strict UTF-8 decode: `from_utf8` yields the string iff every byte forms a shortest-form, non-
    // surrogate scalar sequence — the spec's well-formedness. Otherwise `None`.
    match std::str::from_utf8(&raw) {
        Ok(s) => {
            trace!(target: "rcdzc::fold", node = id.0, "String.from-bytes folds well-formed UTF-8 to Some");
            let payload = db.push_atom(crate::ast::Leaf::Str(s.into()));
            Core::SumNew {
                disc: disc_some,
                payloads: vec![payload].into(),
            }
        }
        Err(_) => {
            trace!(target: "rcdzc::fold", node = id.0, "String.from-bytes folds ill-formed UTF-8 to None");
            Core::SumNew {
                disc: disc_none,
                payloads: Vec::new().into(),
            }
        }
    }
}

/// Lower `(Option.expect sum message)` / `(Result.expect sum message)` — the unwrap-or-trap accessor. The
/// PRESENT variant is discriminant 0 (`Some`/`Ok`, the sum's FIRST variant — the shape the `expect` field
/// is added for). FOLD a compile-time-visible PRESENT variant (`Core::SumNew{disc:0, payloads:[p]}`) to
/// its payload `p` (the message is discarded). A constant ABSENT variant is a PROVABLE trap; not folded
/// yet (declines cleanly — no corpus case exercises a constant absent expect, and a codeless decline
/// grades Todo, never a miscompile). A runtime sum emits `Core::SumExpect` (disc probe → payload / trap).
/// A poison sum propagates. `message` is not lowered — the wasm trap carries no text.
/// Lower `(Record.project r (a c))` — narrow `r` to the named fields. FOLD over a compile-time-visible
/// `Core::Record`: build a NEW `Core::Record` holding only the named fields, each carrying `r`'s own value
/// occurrence (the value heap is immutable, so the result SHARES `r`'s field values — `type-system.md` §A
/// Record Row Operation Yields A New Value). The second operand is a LITERAL field-name list `(a c)` (labels
/// via `record_op_labels`, NOT an evaluated value). A named field absent from `r` is the CDZ0212 `infer`
/// reports; here the fold simply omits it (the reject denies the build, so this core is never emitted). A
/// poison operand propagates; a non-record / non-constant record declines (the runtime row op is a later
/// increment). A malformed label list is CDZ0201.
pub(super) fn lower_record_project(
    db: &mut Db,
    id: StructId,
    record: StructId,
    labels: StructId,
    drop: bool,
) -> Core {
    // FOLD over a compile-time-visible constant record — a DIRECT `Core::Record` OR a shared multi-use
    // `let` binding of one (`const_record_fields` follows the `LocalRef` binder). A multi-use `(let ((r
    // (record …))) … r …)` lowers each `r` to a `LocalRef`, so a naive `core_of` sees the binding not the
    // literal and used to decline "over a RUNTIME record" — misleading (every field is constant; single-use
    // `r` folds because it copy-propagates). A genuinely runtime record (param/call result) still declines.
    // A compile-time-visible record FOLDS (fields are literal occurrences). A RUNTIME record (param / call
    // result / PROJECTION) builds its kept fields from synth `(. record field)` projections
    // (`runtime_record_fields`) — the same source-reading `Record.with` uses; `is_runtime` then drives the
    // materialize-once let-bind so the shared operand evaluates ONCE (not per kept field).
    let (fields, is_runtime): (std::collections::BTreeMap<_, _>, bool) =
        match const_record_fields(db, record) {
            Some(f) => (f.iter().map(|(k, &v)| (k.clone(), v)).collect(), false),
            None => match core_of(db, record) {
                Core::Poison(r) => return Core::Poison(r),
                _ => match runtime_record_fields(db, record) {
                    Some(m) => (m, true),
                    None => {
                        return Core::Poison(Reject::unsupported(
                            "a record row operation over a runtime record is not supported",
                        ));
                    }
                },
            },
        };
    let Some(labels) = crate::resolve::record_op_labels(db, labels) else {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "the second operand is a list of field names, e.g. `(a c)`",
        ));
    };
    // `project` KEEPS the named fields; `without` keeps every field NOT named (the complement). Each
    // result field carries the operand's own value occurrence (const) or a `(. record field)` projection
    // (runtime) — the immutable heap shares them either way.
    let named: std::collections::BTreeSet<_> = labels.iter().cloned().collect();
    let kept: std::collections::BTreeMap<_, _> = fields
        .iter()
        .filter(|(k, _)| named.contains(*k) != drop)
        .map(|(k, &v)| (k.clone(), v))
        .collect();
    trace!(target: "rcdzc::fold", node = id.0, n = kept.len(), drop, is_runtime, "record project/without builds its result fields from a (constant or runtime) record");
    let record_core = Core::Record {
        fields: std::rc::Rc::new(kept),
    };
    materialize_row_op_operand(db, id, record, is_runtime, record_core)
}

/// Lower `(Record.merge a b)` — the UNION of two records' fields (`type-system.md` §Two Records Are
/// Combined Only When Their Field Sets Are Disjoint). FOLD two constant `Core::Record`s to a new one
/// holding every field of both (each carrying its source's value occurrence). The disjointness CDZ0211 is
/// `infer`'s; here a shared field would be silently overwritten by `b`, but the reject denies the build so
/// this core is never emitted. A poison operand propagates; a non-constant/non-record operand declines.
pub(super) fn lower_record_merge(db: &mut Db, id: StructId, a: StructId, b: StructId) -> Core {
    // Propagate a poison operand first (before probing shapes).
    if let Core::Poison(r) = core_of(db, a) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, b) {
        return Core::Poison(r);
    }
    // Each operand's fields come from its const record literal (fold path) OR — for a genuinely-RUNTIME
    // operand — synth `(. operand field)` projections (`runtime_record_fields`, the same helper the other
    // row-ops use). Merge is the UNION of both field sets, so BOTH operands contribute; either may be
    // runtime independently. `runtime_operands` collects the runtime ones so each is materialized ONCE
    // (its projections share its operand node — the reviewer-49d6eec14 hazard, here for two operands).
    let mut runtime_operands: Vec<StructId> = Vec::new();
    let operand_fields =
        |db: &mut Db,
         op: StructId,
         runtime: &mut Vec<StructId>|
         -> Option<std::collections::BTreeMap<crate::resolved::Symbol, StructId>> {
            match const_record_fields(db, op) {
                Some(f) => Some(f.iter().map(|(k, &v)| (k.clone(), v)).collect()),
                None => runtime_record_fields(db, op).inspect(|_| runtime.push(op)),
            }
        };
    let Some(fa) = operand_fields(db, a, &mut runtime_operands) else {
        return Core::Poison(Reject::unsupported(
            "Record.merge over a runtime record is not supported",
        ));
    };
    let Some(fb) = operand_fields(db, b, &mut runtime_operands) else {
        return Core::Poison(Reject::unsupported(
            "Record.merge over a runtime record is not supported",
        ));
    };
    // The union — `a`'s fields then `b`'s (a shared field would let `b` win, but the disjointness CDZ0211
    // `infer` reports denies the build, so this core is never emitted for overlapping sets). Each field
    // carries its source's value occurrence (const) or its `(. operand field)` projection (runtime).
    let mut union: std::collections::BTreeMap<_, _> = fa;
    for (k, v) in fb {
        union.insert(k, v);
    }
    trace!(target: "rcdzc::fold", node = id.0, n = union.len(), runtime = runtime_operands.len(), "Record.merge builds the union of two (constant or runtime) records");
    let record_core = Core::Record {
        fields: std::rc::Rc::new(union),
    };
    if runtime_operands.is_empty() {
        return record_core; // both operands folded — no shared runtime operand to materialize.
    }
    // Materialize EACH runtime operand once (a self-keyed `Core::Let` binding per operand, every projection
    // reading its shared `LocalRef`) — the two-operand generalization of `materialize_row_op_operand`.
    for &op in &runtime_operands {
        db.kept_bindings.insert(op);
    }
    let result_ty = crate::infer::type_of(db, id);
    let body = synth_core(db, record_core, result_ty);
    Core::Let {
        bindings: runtime_operands.iter().map(|&op| (op, op)).collect(),
        body,
    }
}

/// Lower `(Record.extend r #z v)` / `(Record.with r #z v)` — INSERT field `z ↦ v` into a constant
/// `Core::Record` (extend adds an absent field, with replaces a present one; the presence/absence
/// CDZ0211/0212 is `infer`'s, so the fold is one insert for both). Three operands
/// (DESIGN-record-update-syntax.md): the record, a `#z` field LABEL (`label_node`, read statically by
/// `read_label`), and the value `v` (its value occurrence carries into the new field). A poison operand
/// propagates; a non-constant/non-record operand, or a malformed `#field` label, declines/rejects.
pub(super) fn lower_record_insert(
    db: &mut Db,
    id: StructId,
    record: StructId,
    label_node: StructId,
    value: StructId,
) -> Core {
    // FOLD over a compile-time-visible constant record — a DIRECT `Core::Record` OR a shared multi-use
    // `let` binding of one (`const_record_fields` follows the `LocalRef` binder). A multi-use `(let ((r
    // (record …))) … r …)` lowers each `r` to a `LocalRef`, so a naive `core_of` sees the binding not the
    // literal and used to decline "over a RUNTIME record" — misleading (every field is constant; single-use
    // `r` folds because it copy-propagates). A genuinely runtime record (param/call result) still declines.
    // A compile-time-visible record FOLDS (its fields are literal occurrences). A RUNTIME record (param /
    // call result / PROJECTION `(. o pos)`) builds a fresh record whose UNCHANGED fields read the source via
    // synth `(. record field)` projections (`runtime_record_fields`) and whose named field carries `value` —
    // the same "yields a new value" result, just with projected sources instead of literal ones.
    // `is_runtime` is true when the unchanged fields read the source via synth `(. record field)`
    // PROJECTIONS (`runtime_record_fields`) — all sharing the ONE `record` operand. Those projections must
    // then read a MATERIALIZED operand (see the let-bind below), not the raw operand N times.
    let (mut out, is_runtime): (std::collections::BTreeMap<_, _>, bool) =
        match const_record_fields(db, record) {
            Some(fields) => (fields.iter().map(|(k, &v)| (k.clone(), v)).collect(), false),
            None => match core_of(db, record) {
                Core::Poison(r) => return Core::Poison(r),
                _ => match runtime_record_fields(db, record) {
                    Some(m) => (m, true),
                    None => {
                        return Core::Poison(Reject::unsupported(
                            "a record row operation over a runtime record is not supported",
                        ));
                    }
                },
            },
        };
    let Some(label) = crate::resolve::read_label(db, label_node) else {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "the second operand is a `#field` label, e.g. `#z`",
        ));
    };
    out.insert(label, value);
    trace!(target: "rcdzc::fold", node = id.0, n = out.len(), "Record.extend/with builds an insert into a (constant or runtime) record");
    let record_core = Core::Record {
        fields: std::rc::Rc::new(out),
    };
    materialize_row_op_operand(db, id, record, is_runtime, record_core)
}

/// Wrap a runtime record row-op's result so its shared `record` operand is EVALUATED ONCE. A runtime
/// row-op (`Record.with`/`project`/`without`/`merge`/`pop` over a genuinely-runtime record) builds its
/// result from synth `(. record field)` PROJECTIONS via [`runtime_record_fields`], EVERY one sharing the
/// ONE `record` operand node. The backend has no CSE, so each `Core::Proj` re-emits `record`'s computation
/// — an arity≥3 record (≥2 preserved fields) then evaluates the operand once PER preserved field: a perf
/// cliff for a pure operand and, for an EFFECTFUL operand (a perform-bearing def result), the effect fires
/// N times — an observable MISCOMPILE (reviewer post-merge finding on 49d6eec14). MATERIALIZE the operand
/// ONCE: a self-keyed `Core::Let { (record, record) }` whose body is the built result; marking `record` a
/// kept binding makes every `(. record field)` projection lower its operand to a shared `Core::LocalRef`
/// (a `local.get`), so the operand's computation emits exactly once. The runtime-compare / runtime-bin-match
/// materialize-once precedent ([`lower_runtime_compare`]). A CONST-folded result (`is_runtime == false`) has
/// literal field sources — no shared operand — so it is returned unwrapped. `id` is the row-op node (its
/// solved type is the result the `Core::Let` body carries — a record for with/project/without/merge, a
/// `(value, rest)` TUPLE for pop). `result_core` is that built body (a `Core::Record` or `Core::Tuple`).
pub(super) fn materialize_row_op_operand(
    db: &mut Db,
    id: StructId,
    record: StructId,
    is_runtime: bool,
    result_core: Core,
) -> Core {
    if !is_runtime {
        return result_core;
    }
    db.kept_bindings.insert(record);
    let result_ty = crate::infer::type_of(db, id);
    let body = synth_core(db, result_core, result_ty);
    Core::Let {
        bindings: vec![(record, record)].into(),
        body,
    }
}

/// Lower `(Record.pop r z)` — `(tuple (. r z) (r without z))`: the popped field's value paired with the
/// record of the remaining fields. Folds a constant `Core::Record` to a `Core::Tuple{elems: [value,
/// rest-record]}`. The absent-field CDZ0212 is `infer`'s (this fold assumes the field present — an absent
/// one leaves no value occurrence, so it declines defensively). A poison/non-constant operand
/// propagates/declines.
pub(super) fn lower_record_pop(
    db: &mut Db,
    id: StructId,
    record: StructId,
    name: StructId,
) -> Core {
    // A compile-time-visible record FOLDS (fields are literal occurrences). A RUNTIME record (param / call
    // result / projection) builds every field from a synth `(. record field)` projection
    // (`runtime_record_fields`) — the same source-reading `Record.with`/`project` use; `is_runtime` then
    // drives the materialize-once let-bind so the shared operand evaluates ONCE (not once for the popped
    // value + once per remaining field).
    let (fields, is_runtime): (std::collections::BTreeMap<_, _>, bool) =
        match const_record_fields(db, record) {
            Some(f) => (f.iter().map(|(k, &v)| (k.clone(), v)).collect(), false),
            None => match core_of(db, record) {
                Core::Poison(r) => return Core::Poison(r),
                _ => match runtime_record_fields(db, record) {
                    Some(m) => (m, true),
                    None => {
                        return Core::Poison(Reject::unsupported(
                            "Record.pop over a runtime record is not supported",
                        ));
                    }
                },
            },
        };
    let Some(label) = crate::resolve::read_label(db, name) else {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "the second operand is a field name, e.g. `z`",
        ));
    };
    let Some(&value) = fields.get(&label) else {
        return Core::Poison(Reject::decline(
            "Record.pop of an absent field (reported CDZ0212 by inference)",
        ));
    };
    // The remaining record — every field EXCEPT the popped one, each carrying its value occurrence (const)
    // or its `(. record field)` projection (runtime). Synthesized as its own occurrence (`synth_core`,
    // `Core::Record` + its `Ty::Record`) so it can be the tuple's second element (elements are node ids).
    let rest: std::collections::BTreeMap<_, _> = fields
        .iter()
        .filter(|(k, _)| **k != label)
        .map(|(k, &v)| (k.clone(), v))
        .collect();
    let rest_ty: std::collections::BTreeMap<_, _> = rest
        .keys()
        .map(|k| (k.clone(), crate::infer::type_of(db, rest[k])))
        .collect();
    let rest_record = synth_core(
        db,
        Core::Record {
            fields: std::rc::Rc::new(rest),
        },
        crate::ty::Ty::Record(std::rc::Rc::new(rest_ty)),
    );
    trace!(target: "rcdzc::fold", node = id.0, is_runtime, "Record.pop builds a (value, remaining-record) tuple from a (constant or runtime) record");
    // Both the popped `value` and every field of `rest_record` read the ONE `record` operand (via their
    // synth projections). Materialize it once so the runtime operand's computation emits a single time; the
    // helper's `type_of(id)` is the pop RESULT tuple type, which it wraps as the `Core::Let` body.
    let tuple = Core::Tuple {
        elems: std::rc::Rc::from([value, rest_record]),
    };
    materialize_row_op_operand(db, id, record, is_runtime, tuple)
}

/// Lower `(Tuple.concat a b)` — concatenate two constant `Core::Tuple`s: the elements of `a` in order
/// followed by `b`'s (each element carrying its source occurrence). A poison operand propagates; a
/// non-constant/non-tuple operand declines (the runtime op is a later increment).
pub(super) fn lower_tuple_cat(db: &mut Db, id: StructId, a: StructId, b: StructId) -> Core {
    match (core_of(db, a), core_of(db, b)) {
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        (Core::Tuple { elems: ea }, Core::Tuple { elems: eb }) => {
            let mut elems = ea.to_vec();
            elems.extend(eb.iter().copied());
            trace!(target: "rcdzc::fold", node = id.0, n = elems.len(), "Tuple.concat folds two constant tuples");
            Core::Tuple {
                elems: elems.into(),
            }
        }
        _ => Core::Poison(Reject::unsupported(
            "Tuple.concat over a runtime tuple is not supported",
        )),
    }
}

/// Synthesize a tuple VALUE node from element occurrences — a `Core::Tuple` (or `Core::Unit` for the
/// empty tuple, the empty-tuple-is-unit convention) with its `Ty` filled, so it can be an element of an
/// enclosing tuple (whose elements are node ids). Mirrors `Record.pop`'s remaining-record synthesis.
pub(super) fn synth_tuple(db: &mut Db, elems: Vec<StructId>) -> StructId {
    if elems.is_empty() {
        return synth_core(db, Core::Unit, crate::ty::Ty::Unit);
    }
    let tys: Vec<crate::ty::Ty> = elems
        .iter()
        .map(|&e| crate::infer::type_of(db, e))
        .collect();
    synth_core(
        db,
        Core::Tuple {
            elems: elems.into(),
        },
        crate::ty::Ty::Tuple(tys.into()),
    )
}

/// Lower `(Tuple.split-at t k)` — split a constant `Core::Tuple` at compile-time literal `k` into the
/// PAIR `(tuple <prefix> <suffix>)`: a prefix tuple of the first `k` elements and a suffix tuple of the
/// rest, each synthesized as its own occurrence (`synth_tuple`; an empty side is `unit`). An out-of-range
/// or non-literal `k` is the CDZ0201 `infer` reports (this fold declines defensively). A poison / non-
/// constant tuple operand propagates/declines.
pub(super) fn lower_tuple_split_at(
    db: &mut Db,
    id: StructId,
    tuple: StructId,
    pos: StructId,
) -> Core {
    let Core::Tuple { elems } = core_of(db, tuple) else {
        return match core_of(db, tuple) {
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::unsupported(
                "Tuple.split-at over a runtime tuple is not supported",
            )),
        };
    };
    let arity = elems.len() as i64;
    let k = match core_of(db, pos) {
        Core::ConstInt(v) => v.to_i64().filter(|&k| (0..=arity).contains(&k)),
        _ => None,
    };
    let Some(k) = k else {
        return Core::Poison(Reject::decline(
            "Tuple.split-at needs a compile-time position within the tuple's arity",
        ));
    };
    let k = k as usize;
    let prefix = synth_tuple(db, elems[..k].to_vec());
    let suffix = synth_tuple(db, elems[k..].to_vec());
    trace!(target: "rcdzc::fold", node = id.0, k, "Tuple.split-at folds to a (prefix, suffix) pair");
    Core::Tuple {
        elems: std::rc::Rc::from([prefix, suffix]),
    }
}

/// Lower `(Tuple.remove t)` — element 0 off: `(tuple (. t 0) <rest>)`, the rest a synthesized tuple of the
/// remaining elements (`(Tuple.split-at t 1)` with the singleton prefix unwrapped). A poison / non-
/// constant / empty tuple operand propagates/declines.
pub(super) fn lower_tuple_pop(db: &mut Db, id: StructId, tuple: StructId) -> Core {
    let Core::Tuple { elems } = core_of(db, tuple) else {
        return match core_of(db, tuple) {
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::unsupported(
                "Tuple.remove over a runtime tuple is not supported",
            )),
        };
    };
    let Some((&first, rest)) = elems.split_first() else {
        return Core::Poison(Reject::decline("Tuple.remove of an empty tuple"));
    };
    let rest_tuple = synth_tuple(db, rest.to_vec());
    trace!(target: "rcdzc::fold", node = id.0, "Tuple.remove folds to a (element0, rest) tuple");
    Core::Tuple {
        elems: std::rc::Rc::from([first, rest_tuple]),
    }
}

pub(super) fn lower_sum_expect(db: &mut Db, id: StructId, sum: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, sum) {
        return Core::Poison(r);
    }
    // The present variant is discriminant 0 (the sum's first variant). Confirm the scrutinee IS a sum.
    let crate::ty::Ty::Sum { .. } = crate::infer::type_of(db, sum) else {
        return Core::Poison(Reject::decline(
            "expect applies to an Option/Result sum value",
        ));
    };
    const DISC_PRESENT: u32 = 0;
    // FOLD a compile-time-visible present variant to its single payload.
    if let Core::SumNew { disc, payloads } = core_of(db, sum) {
        if disc == DISC_PRESENT && payloads.len() == 1 {
            trace!(target: "rcdzc::fold", node = id.0, "expect folds a constant present variant to its payload");
            return core_of(db, payloads[0]);
        }
        if disc != DISC_PRESENT {
            // A provably-ABSENT constant expect (`Option.expect None`, `Result.expect (Err …)`) — requiring
            // the value of a statically-known absent optional is a PROVABLE TRAP (core-semantics.md
            // §Requiring The Value Of An Optional Traps On Absence). Fold to `Core::Trap` (an `unreachable`)
            // — exactly what the runtime `Core::SumExpect` emits on its absent-disc branch, and the same
            // provable-trap lowering `T.of` out-of-range and a proven-overflow `*` fold to. The trap carries
            // no text (an `unreachable` has none), so a `(trap "m")` message-match still grades Todo — but
            // the OUTCOME is now the correct divergence rather than a decline.
            trace!(target: "rcdzc::fold", node = id.0, disc, "expect on a constant absent variant folds to a provable trap");
            return Core::Trap;
        }
    }
    // A runtime sum — probe the discriminant at run time, unwrap the payload or trap.
    Core::SumExpect {
        scrutinee: sum,
        disc_present: DISC_PRESENT,
    }
}

/// Lower `(Int64.checked-add a b)` / `(Int64.checked-mul a b)` — the FALLIBLE arithmetic companions of
/// the trapping `+`/`*`, returning `(Option T)`: `Some result` when it fits the width / `None` on
/// Materialize each of `operands` that REACHES A HOST CALL as a SINGLE evaluation before `body_core`
/// names it more than once — a self-keyed `Core::Let { (op, op) }` per such operand, marking it a kept
/// binding so every reference in the body lowers to a shared `Core::LocalRef` (one `local.get`), computing
/// the operand exactly once. Without this, a compose that names an operand in several positions (a
/// range-check's `operand > tmax` compare AND its `wrap(operand)` else; a checked-arith's overflow formula
/// AND its `Some` result) re-emits the operand PER REFERENCE — and for an EFFECTFUL operand (a host-call-
/// lifted value like `(hosti.base unit)`) the effect FIRES PER USE: the host op is invoked N times, draining
/// N queued responses / trapping when the responses run out (breaker adv-tof-host-u64: `Int64.of (host-u64
/// 1000)` spuriously trapped because the range-check re-invoked the host call). Mirrors
/// [`materialize_row_op_operand`] and the adv-62b `let` force-keep ([`core_reaches_host_call`]). A PURE
/// operand (no host call) is left as-is: duplicating a scalar recompute is sound (idempotent, no effect, no
/// heap), so it needs no Let-wrapping. Deduplicates a repeated operand id (e.g. `checked-add a a`).
pub(super) fn materialize_host_operands_once(
    db: &mut Db,
    id: StructId,
    operands: &[StructId],
    body_core: Core,
) -> Core {
    let mut bindings: Vec<(StructId, StructId)> = Vec::new();
    for &op in operands {
        let mut seen = std::collections::HashSet::new();
        if core_reaches_host_call(db, op, &mut seen) && !bindings.iter().any(|(b, _)| *b == op) {
            db.kept_bindings.insert(op);
            bindings.push((op, op));
        }
    }
    if bindings.is_empty() {
        return body_core;
    }
    let ty = crate::infer::type_of(db, id);
    let body = synth_core(db, body_core, ty);
    Core::Let {
        bindings: bindings.into(),
        body,
    }
}

/// overflow (numeric-model.md §Overflow Is Defined). FOLD a constant operand pair via `i64` checked
/// arithmetic (the SAME `checked_add`/`checked_mul` `fold_arith` uses to prove the trapping op's overflow
/// — but here overflow yields `None`, not a build error): in range → `Core::SumNew{disc_some, [result]}`
/// (the result a fresh `Core::ConstInt` synthesized into the arena, the `Some` payload — the shape
/// `List.at`/`String.at` use); overflow → `Core::SumNew{disc_none, []}`. Both fold to the ordinary Option
/// construction, riding the sum fold/escape/match. A runtime operand composes an overflow-check
/// (`materialize_host_operands_once` guards a host-lifted operand from double-firing); a poison operand
/// propagates.
pub(super) fn lower_checked_arith(
    db: &mut Db,
    id: StructId,
    prim: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "checked-arithmetic result is not the built-in Option sum",
        ));
    };
    match (core_of(db, lhs), core_of(db, rhs)) {
        (Core::ConstInt(a), Core::ConstInt(b)) => {
            // Evaluate over `i64` (the Stage default width) — the same range the trapping fold uses. A
            // later width stage generalizes the overflow test to the solved width.
            let (Some(x), Some(y)) = (a.to_i64(), b.to_i64()) else {
                // An operand beyond the machine range — a later width stage handles it; decline for now.
                return Core::Poison(Reject::unsupported(
                    "checked arithmetic on an operand beyond the evaluated width is not supported",
                ));
            };
            let checked = match prim {
                Prim::CheckedAdd => x.checked_add(y),
                Prim::CheckedSub => x.checked_sub(y),
                _ => x.checked_mul(y),
            };
            match checked {
                Some(n) => {
                    trace!(target: "rcdzc::fold", node = id.0, ?prim, result = n, "checked arithmetic folds to Some (in range)");
                    let payload = db.push_atom(crate::ast::Leaf::Int {
                        value: IntValue::from_i64(n),
                        radix: crate::ast::Radix::Dec,
                    });
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![payload].into(),
                    }
                }
                None => {
                    trace!(target: "rcdzc::fold", node = id.0, ?prim, "checked arithmetic folds to None (overflow)");
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    }
                }
            }
        }
        // A runtime operand: compose the overflow-detecting Some/None from existing Core — the wrapping
        // result plus the overflow predicate — no new Core variant. Restricted to the full 64-bit width
        // (Int64/UInt64): there the i64 register IS the value, so ADD/SUB's sign-bit test at bit 63 is exact
        // and MUL's division round-trip needs no narrowing. A NARROW width (whose stored representation makes
        // the sign bit width-relative) is a later increment — it declines cleanly rather than risk a wrong
        // overflow verdict (a checked op that mis-reported overflow would be a silent miscompile).
        _ => {
            let crate::ty::Ty::Int(it) = crate::infer::type_of(db, lhs) else {
                return Core::Poison(Reject::decline(
                    "checked arithmetic on a runtime non-integer operand has no meaning",
                ));
            };
            let signed = it.ground_signed();
            let int_ty = crate::ty::Ty::Int(it);
            let result_ty = crate::infer::type_of(db, id);
            // NARROW-WIDTH (8/16/32) checked ADD/SUB: widen both operands to Int64 (value-preserving —
            // sign/zero extend per the operand's signedness, the same `Wrap` widen `.of` uses), do the op in
            // Int64 where a narrow add/sub CANNOT overflow (operands are ≤32-bit, the exact sum/difference fits
            // i64), then RANGE-CHECK the exact result against the NARROW `[tmin,tmax]`: in range → `Some(wrap-
            // to-narrow)`, out → `None`. This sidesteps the width-relative sign-bit problem (the reason the
            // narrow path was pinned) by computing exactly in the wide accumulator and checking the narrow
            // bounds — the same interval the `.of` narrowing checks. Each operand is named ONCE (in its widen),
            // so no host-operand double-fire. MUL is included: the product of two ≤32-bit operands fits a
            // 64-bit accumulator (u32×u32 max = 2^64−2^33 < 2^64 fits UInt64; i32×i32 ≤ 2^62 fits Int64), so the
            // SAME widen-and-range-check works — with the accumulator SIGNEDNESS matching the op so the compare
            // reads the product's true magnitude (an unsigned narrow MUL needs a UInt64 accumulator + unsigned
            // compares; every other narrow op fits a signed Int64 accumulator).
            if it.ground_width() < 64 {
                let Some((Some(tmin), Some(tmax))) = resolved_int_bounds(it) else {
                    return Core::Poison(Reject::decline(
                        "a narrow checked op needs resolved integer bounds",
                    ));
                };
                // An unsigned narrow MUL widens into a UInt64 accumulator (its product can exceed i64's
                // positive range); every other narrow op (signed any, unsigned add/sub) fits a signed Int64.
                let acc_unsigned = !signed && matches!(prim, Prim::CheckedMul);
                let acc_ty = crate::ty::Ty::Int(crate::ty::IntTy::fixed(!acc_unsigned, 64));
                // Widen each operand to the accumulator (value-preserving — same-signedness extend), then the
                // exact wrapping op there (no overflow for narrow operands: sum/diff/product all fit 64 bits).
                let a64 = synth_core(
                    db,
                    Core::Convert {
                        op: Prim::Wrap,
                        operand: lhs,
                    },
                    acc_ty.clone(),
                );
                let b64 = synth_core(
                    db,
                    Core::Convert {
                        op: Prim::Wrap,
                        operand: rhs,
                    },
                    acc_ty.clone(),
                );
                let wrap_prim = match prim {
                    Prim::CheckedAdd => Prim::WrappingAdd,
                    Prim::CheckedSub => Prim::WrappingSub,
                    _ => Prim::WrappingMul,
                };
                let s64 = synth_core(
                    db,
                    Core::Arith {
                        op: wrap_prim,
                        lhs: a64,
                        rhs: b64,
                    },
                    acc_ty.clone(),
                );
                // The narrow value the Some arm carries (wrap the in-range exact result to the target width —
                // value-preserving in range).
                let narrow_s = synth_core(
                    db,
                    Core::Convert {
                        op: Prim::Wrap,
                        operand: s64,
                    },
                    int_ty.clone(),
                );
                let some = synth_core(
                    db,
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![narrow_s].into(),
                    },
                    result_ty.clone(),
                );
                let none = synth_core(
                    db,
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    },
                    result_ty.clone(),
                );
                // if s64 > tmax then None else if s64 < tmin then None else Some(narrow_s). The bound consts
                // + `s64` share the accumulator type, so the `Compare`s take its signedness — signed for the
                // Int64 accumulator (the exact result's true sign), UNSIGNED for the UInt64 mul accumulator
                // (reads the product's true magnitude; the `< tmin=0` check is then vacuous, as it should be).
                let hi = synth_core(db, Core::ConstInt(IntValue::from_i64(tmax)), acc_ty.clone());
                let lo = synth_core(db, Core::ConstInt(IntValue::from_i64(tmin)), acc_ty.clone());
                let over = synth_core(
                    db,
                    Core::Compare {
                        op: Prim::Gt,
                        lhs: s64,
                        rhs: hi,
                    },
                    crate::ty::Ty::Bool,
                );
                let under = synth_core(
                    db,
                    Core::Compare {
                        op: Prim::Lt,
                        lhs: s64,
                        rhs: lo,
                    },
                    crate::ty::Ty::Bool,
                );
                let under_case = synth_core(
                    db,
                    Core::If {
                        cond: under,
                        then_: none,
                        else_: some,
                    },
                    result_ty.clone(),
                );
                trace!(target: "rcdzc::lower", node = id.0, ?prim, signed, width = it.ground_width(), "runtime narrow checked add/sub → widen-to-i64 + range-check the narrow bounds");
                let if_core = Core::If {
                    cond: over,
                    then_: none,
                    else_: under_case,
                };
                // `s64` (the exact wide result) is named in both range-checks AND the Some payload, and its
                // subtree carries the widened operands — so materialize it ONCE when it reaches a host call
                // (else that host op fires per reference). A pure `s64` is left inline (a cheap recompute).
                return materialize_host_operands_once(db, id, &[s64], if_core);
            }
            // CHECKED-MUL: detect overflow with a DIVISION round-trip (no 128-bit multiply). `p = a *w b` is
            // the wraparound product; the true product fits iff — for `a != 0` — `p / a == b`. The signed
            // `div` (`div_s`) itself traps on its two edges (÷0, `Int64.min / -1`), so both are guarded away
            // BEFORE the division runs (Core::If evaluates only the taken branch):
            //   signed:   a==0 -> Some(p);  a==-1 -> (b==Int64.min ? None : Some(p));  else (p/a==b ? Some(p):None)
            //   unsigned: a==0 -> Some(p);  else (p/a==b ? Some(p):None)
            // `a==0` gives `p==0` (always in range); `a==-1` is the ONLY case where `p/a` could be
            // `Int64.min/-1`, so it is decided explicitly (overflow iff `b==Int64.min`, i.e. `-1 * Int64.min
            // = 2^63` is out of range) — matching the trapping-`*` guard's `div_s` MIN/-1 note. Verified on
            // breaker's ladder incl. the killer `-2^31 * 2^32 = Int64.min` EXACT-fit (p/a recovers b, so Some).
            if matches!(prim, Prim::CheckedMul) {
                let p = synth_core(
                    db,
                    Core::Arith {
                        op: Prim::WrappingMul,
                        lhs,
                        rhs,
                    },
                    int_ty.clone(),
                );
                let some_p = synth_core(
                    db,
                    Core::SumNew {
                        disc: disc_some,
                        payloads: vec![p].into(),
                    },
                    result_ty.clone(),
                );
                let none = synth_core(
                    db,
                    Core::SumNew {
                        disc: disc_none,
                        payloads: Vec::new().into(),
                    },
                    result_ty.clone(),
                );
                let zero = synth_core(db, Core::ConstInt(IntValue::from_i64(0)), int_ty.clone());
                let a_eq_0 = synth_core(
                    db,
                    Core::Compare {
                        op: Prim::Eq,
                        lhs,
                        rhs: zero,
                    },
                    crate::ty::Ty::Bool,
                );
                // else (a != 0): `p / a == b` -> Some(p), else None. `Prim::Div` selects div_s / div_u from
                // the operand type; the guards above keep it off both trap edges.
                let pdiv = synth_core(
                    db,
                    Core::Arith {
                        op: Prim::Div,
                        lhs: p,
                        rhs: lhs,
                    },
                    int_ty.clone(),
                );
                let pdiv_eq_b = synth_core(
                    db,
                    Core::Compare {
                        op: Prim::Eq,
                        lhs: pdiv,
                        rhs,
                    },
                    crate::ty::Ty::Bool,
                );
                let div_case = synth_core(
                    db,
                    Core::If {
                        cond: pdiv_eq_b,
                        then_: some_p,
                        else_: none,
                    },
                    result_ty.clone(),
                );
                let else_branch = if signed {
                    let neg1 =
                        synth_core(db, Core::ConstInt(IntValue::from_i64(-1)), int_ty.clone());
                    let a_eq_neg1 = synth_core(
                        db,
                        Core::Compare {
                            op: Prim::Eq,
                            lhs,
                            rhs: neg1,
                        },
                        crate::ty::Ty::Bool,
                    );
                    let min = synth_core(
                        db,
                        Core::ConstInt(IntValue::from_i64(i64::MIN)),
                        int_ty.clone(),
                    );
                    let b_eq_min = synth_core(
                        db,
                        Core::Compare {
                            op: Prim::Eq,
                            lhs: rhs,
                            rhs: min,
                        },
                        crate::ty::Ty::Bool,
                    );
                    // a == -1: `-1 * b` overflows iff b == Int64.min; otherwise the product `-b` fits.
                    let neg1_case = synth_core(
                        db,
                        Core::If {
                            cond: b_eq_min,
                            then_: none,
                            else_: some_p,
                        },
                        result_ty.clone(),
                    );
                    synth_core(
                        db,
                        Core::If {
                            cond: a_eq_neg1,
                            then_: neg1_case,
                            else_: div_case,
                        },
                        result_ty.clone(),
                    )
                } else {
                    div_case
                };
                trace!(target: "rcdzc::lower", node = id.0, signed, "runtime checked-mul → division round-trip (a==0 / a==-1 guards keep div off its trap edges)");
                let if_core = Core::If {
                    cond: a_eq_0,
                    then_: some_p,
                    else_: else_branch,
                };
                // A host-lifted operand is named in the product AND the division/compare — materialize once.
                return materialize_host_operands_once(db, id, &[lhs, rhs], if_core);
            }
            // `s` = the two's-complement wraparound result — the value the checked op returns when it does
            // NOT overflow (and the value it discards when it does). `Core::Arith` with a WRAPPING prim
            // selects the raw machine add/sub (no trap); at 64 bits the width-mask is a no-op.
            let wrap_prim = if matches!(prim, Prim::CheckedAdd) {
                Prim::WrappingAdd
            } else {
                Prim::WrappingSub
            };
            let s = synth_core(
                db,
                Core::Arith {
                    op: wrap_prim,
                    lhs,
                    rhs,
                },
                int_ty.clone(),
            );
            // The overflow predicate (`Bool`).
            let ovf = if signed {
                // SIGNED two's-complement overflow — a sign-bit test on a bitwise combination:
                //   ADD: `((a ^ s) & (b ^ s)) < 0` — both operands shared a sign the result does not.
                //   SUB: `((a ^ b) & (a ^ s)) < 0` — the operands disagreed in sign AND the result took the
                //        wrong one (`a - b` overflows exactly when `a` and `b` differ in sign and `s`
                //        differs in sign from `a`).
                let (p, q) = if matches!(prim, Prim::CheckedAdd) {
                    let axs = synth_core(
                        db,
                        Core::Arith {
                            op: Prim::BitXor,
                            lhs,
                            rhs: s,
                        },
                        int_ty.clone(),
                    );
                    let bxs = synth_core(
                        db,
                        Core::Arith {
                            op: Prim::BitXor,
                            lhs: rhs,
                            rhs: s,
                        },
                        int_ty.clone(),
                    );
                    (axs, bxs)
                } else {
                    let axb = synth_core(
                        db,
                        Core::Arith {
                            op: Prim::BitXor,
                            lhs,
                            rhs,
                        },
                        int_ty.clone(),
                    );
                    let axs = synth_core(
                        db,
                        Core::Arith {
                            op: Prim::BitXor,
                            lhs,
                            rhs: s,
                        },
                        int_ty.clone(),
                    );
                    (axb, axs)
                };
                let both = synth_core(
                    db,
                    Core::Arith {
                        op: Prim::BitAnd,
                        lhs: p,
                        rhs: q,
                    },
                    int_ty.clone(),
                );
                let zero = synth_core(db, Core::ConstInt(IntValue::from_i64(0)), int_ty.clone());
                synth_core(
                    db,
                    Core::Compare {
                        op: Prim::Lt,
                        lhs: both,
                        rhs: zero,
                    },
                    crate::ty::Ty::Bool,
                )
            } else {
                // UNSIGNED overflow — a wrap-below test (the `Compare` is unsigned, derived from the
                // UInt64 operand type):
                //   ADD: `s <u a` — the sum wrapped below an addend.
                //   SUB: `a <u b` — the minuend is smaller, so `a - b` underflows.
                let (lo, hi) = if matches!(prim, Prim::CheckedAdd) {
                    (s, lhs)
                } else {
                    (lhs, rhs)
                };
                synth_core(
                    db,
                    Core::Compare {
                        op: Prim::Lt,
                        lhs: lo,
                        rhs: hi,
                    },
                    crate::ty::Ty::Bool,
                )
            };
            // `if <overflow> then None else Some(s)`.
            trace!(target: "rcdzc::lower", node = id.0, ?prim, signed, "runtime checked add/sub → if <overflow-predicate> then None else Some(wrap result)");
            let none = synth_core(
                db,
                Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new().into(),
                },
                result_ty.clone(),
            );
            let some = synth_core(
                db,
                Core::SumNew {
                    disc: disc_some,
                    payloads: vec![s].into(),
                },
                result_ty,
            );
            let if_core = Core::If {
                cond: ovf,
                then_: none,
                else_: some,
            };
            // The compose names `lhs`/`rhs` in several positions (the wrapping result, the overflow
            // formula) — materialize a HOST-LIFTED operand ONCE so its effect does not fire per reference.
            materialize_host_operands_once(db, id, &[lhs, rhs], if_core)
        }
    }
}

/// Lower `(Int64.wrapping-add a b)` / `(Int64.wrapping-mul a b)` — two's-complement wraparound, NEVER
/// trapping (numeric-model.md §Overflow Is Defined — the modular value outcome). FOLD a constant operand
/// pair via `i64` `wrapping_add`/`wrapping_mul`, then MASK the result to the op's SOLVED width (`wrap_to`,
/// mod 2^w with sign-extension for a signed narrow type) — a wrapping op has a defined modular outcome, so
/// a NARROW overflow (`(UInt8.wrapping-mul 20 20) = 400 → 144`) must WRAP, never fit-reject. Without the
/// mask the unmasked `ConstInt(400)` reaches select's literal-width gate and is wrongly rejected CDZ0302
/// (the checked-op reject), diverging from the RUNTIME narrow-wrap path (which masks at the backend) — see
/// the `df9f369b` runtime witnesses. At Int64 the mask to 64 bits is a no-op. A runtime operand becomes a
/// `Core::Arith` carrying the WRAPPING prim — the backend selects the RAW machine `i64.add`/`i64.mul`
/// (which already wraps), NOT the checked/trapping path the `+`/`*` prims take. A poison operand propagates.
pub(super) fn lower_wrapping_arith(
    db: &mut Db,
    id: StructId,
    prim: Prim,
    lhs: StructId,
    rhs: StructId,
) -> Core {
    let a = core_of(db, lhs);
    let b = core_of(db, rhs);
    match (a, b) {
        (Core::ConstInt(x), Core::ConstInt(y)) => {
            let (Some(x), Some(y)) = (x.to_i64(), y.to_i64()) else {
                return Core::Poison(Reject::unsupported(
                    "wrapping arithmetic on an operand beyond the evaluated width is not supported",
                ));
            };
            let n = match prim {
                Prim::WrappingAdd => x.wrapping_add(y),
                Prim::WrappingSub => x.wrapping_sub(y),
                _ => x.wrapping_mul(y),
            };
            // MASK the raw i64 result to the op's solved integer width — a wrapping op's outcome is the
            // value MODULO 2^w (sign-extended for a signed narrow type), so a narrow overflow wraps rather
            // than fit-rejecting at select. A non-integer/unsolved result type leaves the i64 value as-is
            // (a later stage grounds it); at Int64 the 64-bit mask is a no-op.
            let folded = match crate::infer::type_of(db, id) {
                crate::ty::Ty::Int(it) => {
                    IntValue::from_i64(n).wrap_to(it.ground_signed(), it.ground_width())
                }
                _ => IntValue::from_i64(n),
            };
            trace!(target: "rcdzc::fold", ?prim, result = n, "wrapping arithmetic folds to a constant (masked to the solved width)");
            Core::ConstInt(folded)
        }
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // ALGEBRAIC IDENTITY: one operand is a constant making the wrapping op a no-op (`a +% 0`,
        // `a *% 1`) or a constant (`a *% 0 → 0`) — elide the op. Shares the checked-arith `arith_identity`
        // helper (which now handles the wrapping prims), so the two families stay in lockstep.
        (lc, rc) => {
            if let Some(simplified) = arith_identity(db, prim, lhs, &lc, rhs, &rc) {
                trace!(target: "rcdzc::lower", ?prim, "wrapping-arithmetic identity simplified (op elided)");
                return simplified;
            }
            // A runtime operand — the RAW (non-trapping) machine op, selected in the backend from this prim.
            Core::Arith { op: prim, lhs, rhs }
        }
    }
}

/// Lower `(Bytes.of list)` — construct a byte sequence from a list of `Int64` in `0..=255`. Folds only
/// a compile-time-visible `Core::ListNew` operand (a runtime list source is a later increment → declines
/// cleanly). Each element must fold to a constant in range: a value `< 0` or `> 255` is a compile-time
/// trap (CDZ0304, matching the runtime `bytes-set` guard — `numeric-model.md` §A Constant Operation With
/// No Value Is Rejected At Compile Time); a non-constant element declines (its `Bytes.of` can't be baked
/// yet). On success produces `Core::BytesOf { elems }` carrying the element occurrences — the backend
/// bakes/builds the sequence. A poison list propagates.
pub(super) fn lower_bytes_of(db: &mut Db, id: StructId, list: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, list) {
        return Core::Poison(r);
    }
    let Core::ListNew { elems } = core_of(db, list) else {
        // A runtime list (a parameter, a push-built list) is a later increment — decline cleanly.
        return Core::Poison(Reject::unsupported(
            "Bytes.of of a runtime list is not supported (only a visible list literal)",
        ));
    };
    // Each element is a `UInt8` (the `Bytes.of : (List UInt8) → Bytes` scheme). A CONSTANT element
    // outside `0..=255` is not a UInt8 — reject it as an OUT-OF-RANGE WIDTH literal (CDZ0302), NOT a
    // runtime trap: under the UInt8 model an ill-typed byte cannot be constructed at all, and to truncate
    // a wider value into a byte the program writes `(UInt8.wrap n)` explicitly. (The list-element
    // width-check does not yet flow the UInt8 bound through `(list …)` unification on its own, so the
    // constant bound is enforced here — with the width code, matching the type story.) A RUNTIME element
    // (a `UInt8` param, or `(UInt8.wrap n)`) is IN RANGE BY ITS TYPE and passes through — `select` emits
    // its i32 value into `bytes-set`, so `(Bytes.of (list (UInt8.wrap n)))` builds a byte from a runtime
    // value (the LEB128 encoder). The `Core::BytesOf` is built either way; a CONSTANT one bakes at escape
    // (R1), a RUNTIME one builds on the rope heap + escapes via the looping walker (L2b).
    for &e in elems.iter() {
        match core_of(db, e) {
            Core::Poison(r) => return Core::Poison(r),
            Core::ConstInt(v) => match v.to_i64() {
                Some(n) if (0..=255).contains(&n) => {}
                _ => {
                    trace!(target: "rcdzc::fold", node = id.0, "Bytes.of element is not a UInt8 → CDZ0302");
                    // Anchor at the offending ELEMENT (not the whole `Bytes.of` / list), and offer the
                    // truncation the message names as a structural fix: wrap the wide value in
                    // `(UInt8.wrap …)`, which the reader accepts as the dotted member call and which
                    // truncates to the low 8 bits (heuristic — truncation is one valid repair; the author
                    // might instead have meant a different value, so `--verify-fixes` confirms it).
                    return Core::Poison(
                        Reject::coded(
                            Code::IntOutOfRange,
                            "a byte must be a UInt8 (0..=255); truncate a wider value with UInt8.wrap",
                        )
                        .at(e)
                        .with_fix(crate::diag::Fix::wrap_heuristic(
                            e,
                            "(UInt8.wrap ",
                            ")",
                            "truncate to a byte with `UInt8.wrap`",
                        )),
                    );
                }
            },
            // A runtime UInt8 element — in range by its type; `select` emits its value into `bytes-set`.
            _ => {}
        }
    }
    trace!(target: "rcdzc::lower", node = id.0, len = elems.len(), "Bytes.of → Core::BytesOf");
    // `elems` is the `Core::ListNew`'s `Rc<[StructId]>` — reuse the shared slice (a `Bytes.of` of a
    // constant list is its elements as bytes), so this is a refcount bump, not a copy.
    Core::BytesOf { elems }
}

/// Lower `(Bytes.at bytes index)` — the fallible indexed byte read. FOLD when `bytes` is a visible
/// `Core::BytesOf` AND `index` folds to a constant: an in-range index (`0 <= i < len`) yields `(Some
/// byte)` — a `Core::SumNew` at the `Some` disc carrying the byte as a constant `Int64` — and an
/// out-of-range index (negative or `>= len`) yields `None`. Otherwise emit the runtime `Core::BytesAt`
/// (a bounds-checked `bytes-get`). Mirrors `lower_list_at`, but the element is always a byte → `Int64`.
pub(super) fn lower_bytes_at(db: &mut Db, id: StructId, bytes: StructId, index: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, index) {
        return Core::Poison(r);
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Bytes.at result is not the built-in Option sum",
        ));
    };
    // FOLD a `Bytes.of` indexed by a constant integer. An OUT-OF-BOUNDS constant index folds to `None`
    // regardless of the elements (the length is statically known). An IN-BOUNDS index folds to `Some
    // <byte>` ONLY when that element is a compile-time CONSTANT: the `Some` payload is an `Int64`, and a
    // constant byte's core folds through that width, but a RUNTIME element occurrence is a `UInt8` (an i32
    // value) that would sit in the i64 `Some(Int64)` payload UN-WIDENED → invalid wasm ("expected i64,
    // found i32"). So a runtime-element in-bounds read falls through to the runtime `Core::BytesAt` below,
    // which reads the byte and zero-extends it to the payload's i64 width. (`Bytes.at (Bytes.of (list 5))
    // 0)` folds; `Bytes.at (Bytes.of (list n)) 0` with `n` runtime takes the runtime read.)
    if let (Core::BytesOf { elems }, Core::ConstInt(i)) = (core_of(db, bytes), core_of(db, index)) {
        match i.to_i64() {
            Some(n) if n >= 0 && (n as usize) < elems.len() => {
                if matches!(core_of(db, elems[n as usize]), Core::ConstInt(_)) {
                    trace!(target: "rcdzc::fold", node = id.0, index = n, "Bytes.at folds to Some (in-bounds constant index + constant element)");
                    return Core::SumNew {
                        disc: disc_some,
                        payloads: vec![elems[n as usize]].into(),
                    };
                }
                // A runtime element at an in-bounds constant index — fall through to the runtime read
                // (which widens the byte to the Int64 payload); the constant fold would not widen it.
            }
            _ => {
                trace!(target: "rcdzc::fold", node = id.0, "Bytes.at folds to None (out-of-bounds constant index)");
                return Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new().into(),
                };
            }
        }
    }
    // A runtime bytes/element or runtime index — emit the bounds-checked runtime read.
    Core::BytesAt {
        bytes,
        index,
        disc_some,
        disc_none,
    }
}

/// Lower `(Bytes.concat a b)`. FOLD when BOTH operands are visible `Core::BytesOf` literals: the result
/// is a single `Core::BytesOf` whose elements are `a`'s then `b`'s (each already a range-checked constant
/// byte occurrence), so a constant concat bakes with no runtime op. Otherwise emit `Core::BytesConcat`. A
/// poison operand propagates.
pub(super) fn lower_bytes_concat(db: &mut Db, lhs: StructId, rhs: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, lhs) {
        return Core::Poison(r);
    }
    if let Core::Poison(r) = core_of(db, rhs) {
        return Core::Poison(r);
    }
    if let (Core::BytesOf { elems: a }, Core::BytesOf { elems: b }) =
        (core_of(db, lhs), core_of(db, rhs))
    {
        let mut elems = a.to_vec();
        elems.extend(b.iter().copied());
        trace!(target: "rcdzc::fold", len = elems.len(), "Bytes.concat folds two constant sequences");
        return Core::BytesOf {
            elems: elems.into(),
        };
    }
    // Neither operand is a plain constant `BytesOf` (that pair returned above), but a baked `Core::ConstBytes`
    // may be involved (the `Ast.encode` fold produces one). If BOTH operands are still compile-time-visible
    // bytes, fold to a single baked `ConstBytes` — so a `(Ast.decode (Bytes.concat (Ast.encode …) (Bytes.of
    // …)))` still const-folds through the decode. `const_byte_slice` reads both a ConstBytes and a
    // BytesOf-of-constants, so a mixed pair folds too.
    if let (Some(a), Some(b)) = (const_byte_slice(db, lhs), const_byte_slice(db, rhs)) {
        let mut raw = a;
        raw.extend_from_slice(&b);
        trace!(target: "rcdzc::fold", len = raw.len(), "Bytes.concat folds a constant sequence involving a ConstBytes");
        return Core::ConstBytes(raw.into());
    }
    Core::BytesConcat { lhs, rhs }
}

/// Lower `(Bytes.slice bytes start len)` — the fallible sub-range read. Emits the runtime
/// `Core::BytesSlice`, which bounds-checks (`start >= 0`, `len >= 0`, `start + len <= bytes-len`) and
/// yields `Some(bytes-slice)` in range / `None` out — never trapping (the runtime `bytes-slice` traps on
/// OOB, so the emit guards first). A CONSTANT slice (`Bytes.of` sliced by constant `start`/`len`) FOLDS:
/// out-of-range → `None`; in-range → `Some(Bytes.of <sub-range>)`, a synthesized `Core::BytesOf` carrying
/// the selected element occurrences (its core + type PRE-FILLED so it lowers/types/escapes/compares like
/// any constant `Bytes.of` — same shape `String.slice`/`String.to-bytes` synthesize a folded payload). A
/// runtime bytes/start/len takes the runtime path; the runtime `Some(Bytes)` payload is a Bytes HANDLE
/// (no box). Mirrors `lower_bytes_at`, extended to the compound `Some` payload.
pub(super) fn lower_bytes_slice(
    db: &mut Db,
    id: StructId,
    bytes: StructId,
    start: StructId,
    len: StructId,
) -> Core {
    for op in [bytes, start, len] {
        if let Core::Poison(r) = core_of(db, op) {
            return Core::Poison(r);
        }
    }
    let Some((disc_some, disc_none)) = option_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Bytes.slice result is not the built-in Option sum",
        ));
    };
    // A CONSTANT slice — a visible `Bytes.of` sliced by constant `start`/`len` — folds at compile time.
    if let (Core::BytesOf { elems }, Core::ConstInt(s), Core::ConstInt(l)) =
        (core_of(db, bytes), core_of(db, start), core_of(db, len))
    {
        let n = elems.len() as i128;
        match (s.to_i64(), l.to_i64()) {
            // In range (`start >= 0`, `len >= 0`, `start + len <= bytes-len`) → `Some(Bytes.of <sub>)`.
            // The payload is a synthesized node whose core is a `Core::BytesOf` of the selected element
            // occurrences (already range-checked constant bytes) — its `core`/`ty` are pre-filled so it
            // rides the ordinary constant-`Bytes.of` fold/escape/equality (both `core_of` and `type_of`
            // short-circuit on a filled memo slot), no runtime op. `start == len == 0` yields the empty
            // sequence (present, not None).
            (Some(s), Some(l)) if s >= 0 && l >= 0 && (s as i128) + (l as i128) <= n => {
                let sub: Vec<StructId> = elems[s as usize..(s + l) as usize].to_vec();
                // A fresh occurrence to carry the folded sub-sequence. Its leaf is a placeholder (a
                // `Leaf::Bytes` of the raw sub-bytes, purely so an inspected node is self-consistent);
                // the `core`/`ty` pre-fill below is authoritative — `core_of`/`type_of` short-circuit on
                // a filled slot, so the node never re-resolves through the leaf.
                let raw: Vec<u8> = elems[s as usize..(s + l) as usize]
                    .iter()
                    .filter_map(|&e| match core_of(db, e) {
                        Core::ConstInt(v) => v
                            .to_i64()
                            .filter(|n| (0..=255).contains(n))
                            .map(|n| n as u8),
                        _ => None,
                    })
                    .collect();
                let payload = db.push_atom(crate::ast::Leaf::Bytes(raw.into()));
                db.core.fill(payload, Core::BytesOf { elems: sub.into() });
                db.types.fill(payload, crate::ty::Ty::Bytes);
                trace!(target: "rcdzc::fold", node = id.0, start = s, len = l, "Bytes.slice folds to Some (in-range constant)");
                return Core::SumNew {
                    disc: disc_some,
                    payloads: vec![payload].into(),
                };
            }
            // Provably out of range → `None`.
            _ => {
                trace!(target: "rcdzc::fold", node = id.0, "Bytes.slice folds to None (out-of-range constant)");
                return Core::SumNew {
                    disc: disc_none,
                    payloads: Vec::new().into(),
                };
            }
        }
    }
    Core::BytesSlice {
        bytes,
        start,
        len,
        disc_some,
        disc_none,
    }
}
