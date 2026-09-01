use super::*;

/// Whether the handle an expression's emit leaves on the stack is a NEW OWNED reference the current
/// frame must reclaim, or a BORROW another owner (a parameter's caller, a `let`'s binding-slot drop)
/// already accounts for. Drives whether the `value-eq` emit `drop`s an operand after the borrowing
/// compare — an OWNED temporary must be dropped (else it leaks), a BORROW must NOT (else double-free).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HandleOwnership {
    /// A fresh allocation the frame owns — a constructor result (`SumNew`/`Tuple`/`Record`/`ListNew`) or
    /// a call's returned value (ownership transfers out of the callee). The `value-eq` emit drops it.
    Owned,
    /// A reference the frame only borrows — a parameter (the caller owns it) or a kept `let`-binding (the
    /// `Core::Let` emit drops it at scope end). The `value-eq` emit must NOT drop it.
    Borrowed,
}

impl HandleOwnership {
    /// Reclaim the handle held in scratch `slot` IFF this operand was `Owned` — the shared tail of every
    /// borrowing op (`value-eq`/`value-cmp`/`value-eq-shaped`, the runtime BigInt ops): an OWNED temporary
    /// operand must be dropped after the borrowing call (else it leaks), a `Borrowed` reference must be left
    /// to its owner (dropping it would be a double-free). Emits `local.get slot ; drop` for `Owned`, nothing
    /// for `Borrowed`.
    pub(crate) fn drop_slot_if_owned(self, slot: u32, out: &mut Emit) {
        if self == HandleOwnership::Owned {
            out.push(Lir::LocalGet(slot));
            out.push(Lir::CallImport(OP_DROP));
        }
    }
}

/// Classify a BORROWING op's handle OPERAND ownership, or DECLINE (`Err`) a shape whose ownership this
/// analysis cannot prove — reject-don't-miscompile: a wrong guess would leak or double-free the heap.
/// Used by every op that BORROWS a heap operand and returns a fresh/scalar result (so the emit must drop
/// each OWNED-temporary operand but leave a borrowed reference to its owner): `value-eq` and the runtime
/// BigInt ops (`bigint-add`/…/`bigint-cmp`/`bigint-to-i64-checked`, which `unbox_bigint`-BORROW their
/// operands). A constructor / call is `Owned`; a parameter / kept-local reference is `Borrowed`. An `if`/
/// `match`/`let` is classified by its result sub-expression(s) — but ONLY when every branch agrees (a
/// mixed owned/borrowed result cannot be dropped uniformly, so it declines). Anything else declines.
/// Whether operand `id`'s solved type is a `String` (possibly through a nominal/symbol wrapper) — the
/// type whose runtime rep can be a NON-CANONICAL rope (`String.concat` → `bytes-concat`). Such an operand
/// of `value-eq`/`champ_eq` (a physical-byte compare) is canonicalized with `bytes-compact` before the
/// compare so a rope and its flat twin compare equal. A `Symbol` is a nominal over a String leaf (same
/// rope-capable rep); peel nominals so a `Symbol`/String-newtype operand is compacted too.
/// Whether an operand is a rope-capable text/byte value that must be `bytes-compact`ed before the physical
/// `champ_eq` compare — a `String`/`Symbol` OR a `Bytes`. A runtime `Bytes` can be a `bytes-concat`/`.slice`
/// ROPE (physical bytes ≠ a flat leaf of equal content), exactly like a `String.concat` rope, so it is
/// canonicalized the same way (the runtime `bytes-compact` is refcount-neutral, so it is safe on an owned
/// OR a borrowed operand). Used at BOTH physical-byte compare sites: the DIRECT `Core::ValueEq` operand
/// path AND the Map/Set KEY path (`key_needs_compaction`). With the key path now Bytes-compacting,
/// `ty_heap_walkable` admits a `Bytes` leaf (nested in a compound / a Set/Map key) — the nested/keyed
/// companion of the direct-operand Bytes `=`.
pub(crate) fn operand_is_string_or_bytes(db: &mut Db, id: StructId) -> bool {
    fn peel(ty: &Ty) -> &Ty {
        match ty {
            Ty::Nominal { inner, .. } => peel(inner),
            other => other,
        }
    }
    matches!(peel(&type_of(db, id)), Ty::String | Ty::Symbol | Ty::Bytes)
}

/// Whether a Map/Set KEY operand needs `bytes-compact` before the CHAMP `champ_hash`/`champ_eq` — an
/// OWNED runtime String OR Bytes (a `String.concat`/`Bytes.concat`/`.slice` rope, whose physical bytes
/// differ from a flat twin's of equal content, so it would hash into a different slot and never match its
/// flat twin). This is the KEY-path companion of the `Core::ValueEq` compaction (`731dbf09`): both
/// `value-eq` and the map/set key path use `champ_eq` over physical bytes, so a rope must be canonicalized
/// at BOTH. Bytes is included alongside String/Symbol (`operand_is_string_or_bytes`) because a `Bytes`
/// value has the SAME rope representation and the same physical-byte CHAMP key contract — the reasoning is
/// verbatim the String story. Compacts a String/Bytes key of ANY ownership: `bytes-compact` is
/// REFCOUNT-NEUTRAL (it flattens the node IN PLACE and returns the SAME handle — a no-op on an already-flat
/// leaf), so it is safe on an OWNED or a BORROWED key alike (verbatim the `elem_needs_rope_compaction`
/// reasoning at the compound-construction sites). This FIXES a wasm WRONG-VALUE: a BORROWED runtime rope/
/// slice key (a `sum-payload`/`Option.expect` binder / a kept `let`-local of a `Bytes.slice` result) was
/// previously NOT compacted (the old `Owned`-only gate, on a false "compact would consume it" belief), so
/// its raw slice-VIEW node (`[off, len]`) reached `champ_hash` and hashed differently from an equal-content
/// flat Bytes — the lookup missed while value-eq said equal (equal-means-same-key violated). Because
/// compact is in-place-same-handle, a borrowed key STAYS the owner's handle after compaction, so
/// `key_handle_is_owned_temporary` (the drop gate) still decides drop-vs-keep on the operand's ACTUAL
/// ownership — a borrowed key is not dropped (no double-free), an owned key is dropped as before.
pub(crate) fn key_needs_compaction(db: &mut Db, key: StructId) -> bool {
    operand_is_string_or_bytes(db, key)
}

/// Whether a type CONTAINS a `List` anywhere (the type is a List, or a Tuple/Record/Sum/Nominal/Qty/Frame
/// whose component does). A List is an RRB vector — element-canonical but NOT shape-canonical (a concat-
/// built and a push-built equal-element list have different internal trees), so the tagless CHAMP byte-walk
/// would place two EQUAL list values in different slots. When such a value is a Map/Set KEY, the key must be
/// `value-canonicalize`d at the key site so the walk becomes exact. A Set/Map is NOT itself a list (it is
/// canonical at its own level), but a list buried inside a Set-of-lists / Map-with-list-values is the
/// documented residual `value_canonicalize_shaped` leaves — so we do NOT descend into Set/Map here (matching
/// the runtime canonicalize, which dups a Set/Map as-is). `seen` breaks a recursive Sum.
pub(crate) fn ty_contains_list(db: &mut Db, ty: &Ty, seen: &mut Vec<StructId>) -> bool {
    match ty {
        Ty::List(_) => true,
        Ty::Tuple(elems) => {
            let elems: Vec<Ty> = elems.to_vec();
            elems.iter().any(|e| ty_contains_list(db, e, seen))
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().any(|v| ty_contains_list(db, v, seen))
        }
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return false;
            }
            seen.push(*decl);
            let Some(variant_count) = db.type_decl_by_occ(*decl).map(|t| t.variants.len()) else {
                seen.pop();
                return false;
            };
            let mut found = false;
            for disc in 0..variant_count {
                let ctor = db
                    .type_decl_by_occ(*decl)
                    .and_then(|t| t.variants.get(disc))
                    .and_then(|v| v.ctor);
                if let Some(ctor) = ctor
                    && let Some(payload_ty) =
                        crate::infer::payload_ty_at_instantiation(db, ctor, ty)
                    && ty_contains_list(db, &payload_ty, seen)
                {
                    found = true;
                    break;
                }
            }
            seen.pop();
            found
        }
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            ty_contains_list(db, &inner, seen)
        }
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_contains_list(db, &inner, seen)
        }
        _ => false, // scalars, String, Bytes, Char, Set, Map (canonical at own level), Fn, etc.
    }
}

/// Whether a Map/Set KEY must be `value-canonicalize`d before it reaches the tagless CHAMP key path: its
/// type is a `List` or CONTAINS one. This is the `value-canonicalize` analogue of `key_needs_compaction`
/// (which handles the String/Bytes-rope axis). Unlike a rope compaction, this reshapes the RRB spine, so it
/// needs the key's shape descriptor baked at the key site (see the emit).
pub(crate) fn key_needs_canonicalize(db: &mut Db, key: StructId) -> bool {
    let ty = type_of(db, key);
    ty_contains_list(db, &ty, &mut Vec::new())
}

/// Whether the key/element handle left on the stack after `emit` (+ optional box, + optional compact) is
/// an OWNED TEMPORARY the frame must `drop` after a BORROWING key op (`map-lookup`/`set-contains`, which
/// read the key without consuming it) — vs a BORROW of a live owner it must NOT drop. Owned iff the key
/// was BOXED (a scalar → a fresh `box-*` leaf), or COMPACTED (an owned rope → a fresh flat leaf), or the
/// key OPERAND itself is a fresh owned handle (a constructor / call / const compound). A BORROWED
/// String/compound key — a parameter, a kept `let`-local, or a `sum-payload`/`arr-get` projection of a
/// still-live value — is used AS-IS (no box, no compact), so dropping it frees a reference its owner still
/// holds: a use-after-free MISCOMPILE. This is exactly the two-live-matched-String-payloads shape (a
/// tree-walker looking up a node's OWN key and its CHILD's key, both `String` sum-payload projections of
/// live nodes) — the second borrowed key was freed under its owner, flipping the comparison and dropping a
/// per-node decision (a silent wrong count). Declines (via `heap_operand_ownership`) a key whose ownership
/// cannot be proved — reject-don't-miscompile, never a double-free. Mirrors the `Core::ValueEq` ownership
/// gate, the sibling String-payload family (`731dbf09`).
pub(crate) fn key_handle_is_owned_temporary(
    db: &mut Db,
    key: StructId,
    key_ty: &Ty,
) -> Result<bool, Reject> {
    if box_op_for(db, key, key_ty)?.is_some() {
        return Ok(true); // a scalar key → a fresh `box-*` leaf the op borrows, then we drop
    }
    if key_needs_canonicalize(db, key) {
        return Ok(true); // a list-typed/-containing key → a FRESH owned canonical value we drop
    }
    // A String/Bytes key may be `bytes-compact`ed (`key_needs_compaction`), but compact is REFCOUNT-NEUTRAL
    // and IN-PLACE (same handle), so it does NOT change ownership: a compacted OWNED rope is still owned
    // (dropped here), a compacted BORROWED rope is still the owner's handle (NOT dropped — dropping it would
    // free a reference the owner still holds, a use-after-free). So the drop decision — whether compacted or
    // not — is the operand's ACTUAL ownership: drop iff a fresh owned handle (a constructor / call / const
    // compound / owned rope); a borrowed param/local/projection is left to its owner.
    Ok(heap_operand_ownership(db, key)? == HandleOwnership::Owned)
}

/// Emit the KEY CANONICALIZATION at a Map/Set key site: the boxed key handle is ON TOP OF THE STACK; replace
/// it with its `value-canonicalize`d form so a list-typed/-containing key hashes into the correct CHAMP slot
/// (the RRB-shape-canonicality fix). Bakes the key's shape descriptor into a fresh scratch Bytes slot (the
/// same const-Bytes build `Core::ValueCmp` uses), calls `value-canonicalize(key, desc)` (→ a FRESH owned
/// canonical key; the input is borrowed and its own reclamation is unchanged — value-canonicalize borrows),
/// then drops the descriptor temporary. Declines if the key type has no bakeable descriptor (reject-don't-
/// miscompile — the pre-fix byte-walk still applies, only the false-miss persists rather than a bad emit).
/// Called ONLY when `key_needs_canonicalize` (a list-containing key). Two fresh i32 scratch slots (key stash
/// + descriptor); `high` is advanced past them.
pub(crate) fn emit_key_canonicalize(
    db: &mut Db,
    key: StructId,
    key_ty: &Ty,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    // Bake the descriptor from the passed `key_ty`; but that field can be an undetermined `Var` (a list
    // element/key of an EMPTY collection — `Set.of (list)` / `Map.empty` — carries a `Var` `elem_ty`/`key_ty`
    // field, so `value_cmp_shape_descriptor` finds no shape). `key_needs_canonicalize` already decided YES
    // from the NODE's resolved `type_of` (`List Int64`), so the field/node type disagree exactly like the
    // `box_op_for`-vs-`box_op_ty` node-aware family. Prefer the field type, but fall back to the node's
    // resolved type before declining — so an empty-collection list key canonicalizes like a pinned one,
    // instead of a false-miss-vs-decline asymmetry between the Set-of-empty and Map-of-empty paths.
    let desc = match crate::lower::value_cmp_shape_descriptor(db, key_ty) {
        Some(d) => d,
        None => {
            let resolved = type_of(db, key);
            match crate::lower::value_cmp_shape_descriptor(db, &resolved) {
                Some(d) => d,
                None => {
                    // Neither the field nor the node's resolved key type bakes a shape — the key's element
                    // type is genuinely UNDETERMINED at this (already-monomorphized) emit site, e.g.
                    // `(Set.of (list (list)))` whose inner empty-list element nothing constrains. This is a
                    // determinacy fault, not a feature gap: reject it CODED (CDZ0203 "annotate the type",
                    // the escape-result determinacy reject's twin) rather than the former CODELESS decline
                    // that let a not-fully-determined program slip through to a shape-less lower bail. A
                    // DETERMINED key (`(Set.of (list (list 1)))`, or `(: (list) (List Int64))`) bakes a
                    // descriptor and never reaches here; a GENERIC `Set a`/`Map a _` def is monomorphized
                    // before emit, so its key is concrete by the time this runs — no false reject
                    // (seq-286 / fuzzer #5, v-compiler-primitives + v-deferral-declines routed).
                    return Err(Reject::coded(
                        crate::diag::Code::TypeMismatch,
                        format!(
                            "a Set/Map key's type `{}` is not fully determined — annotate it \
                             (e.g. `(: (list) (List Int64))`) so its keys have a canonical form for \
                             comparison",
                            resolved.render_name(&db.name_ctx())
                        ),
                    ));
                }
            }
        }
    };
    // `value-canonicalize` BORROWS its input and returns a FRESH owned canonical value — so the RAW key on
    // the stack must be reclaimed HERE iff it was an OWNED TEMPORARY (a fresh `List.concat`/build result),
    // else it leaks; a BORROWED key (a param / kept-local / a live projection) is left to its owner (dropping
    // it would free a live reference — a UAF). Mirrors the box/compact ownership discipline.
    let raw_owned = matches!(heap_operand_ownership(db, key), Ok(HandleOwnership::Owned));
    let key_slot = *high;
    let desc_slot = *high + 1;
    *high = desc_slot + 1;
    scratch_ty.insert(key_slot, ValType::I32);
    scratch_ty.insert(desc_slot, ValType::I32);
    // Stash the raw key handle (top of stack), clearing the stack for the descriptor bake.
    out.push(Lir::LocalSet(key_slot)); // [] (raw key stashed)
    // Bake the descriptor Bytes into desc_slot (build then store — the ValueCmp bake pattern).
    out.push(Lir::ConstI32(desc.len() as i32));
    out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [desc-buf]
    for (j, &byte) in desc.iter().enumerate() {
        out.push(Lir::ConstI32(j as i32));
        out.push(Lir::ConstI32(byte as i32));
        out.push(Lir::CallImport(OP_BYTES_SET)); // [desc-buf]
    }
    out.push(Lir::LocalSet(desc_slot)); // [] (descriptor stored)
    // canonicalize(raw, desc) → fresh owned canonical key on the stack (replacing the raw key).
    out.push(Lir::LocalGet(key_slot)); // [raw]
    out.push(Lir::LocalGet(desc_slot)); // [raw, desc]
    out.push(Lir::CallImport(OP_VALUE_CANONICALIZE)); // [canon-key] (borrows raw + desc)
    // Drop the borrowed-only descriptor Bytes temporary (value-canonicalize borrowed it).
    out.push(Lir::LocalGet(desc_slot));
    out.push(Lir::CallImport(OP_DROP));
    // Reclaim the raw key if it was an owned temporary (canonicalize only borrowed it).
    if raw_owned {
        out.push(Lir::LocalGet(key_slot));
        out.push(Lir::CallImport(OP_DROP));
    }
    Ok(())
}

/// Whether a compound ELEMENT (tuple/record/list element, sum payload) is a rope-capable byte value — a
/// `String`/`Symbol` (a `String.concat` rope) or a `Bytes` (a `Bytes.concat`/`.slice` rope), peeling
/// nominals. Such a leaf STORED INSIDE a compound must be CANONICALIZED with `bytes-compact` at the
/// construction site, exactly as `op_box_float` canonicalizes a NaN when a float leaf is boxed: the value
/// heap is TAGLESS, so `champ_eq`/`champ_hash`'s structural walk compares a nested leaf by its PHYSICAL
/// raw bytes and cannot know a child is a rope (vs a compound), so a rope leaf nested in a tuple/record/
/// sum/map-key compares UNEQUAL to its flat twin (and a compound map key containing one lands in a
/// different CHAMP slot). Compacting on construction means no compound ever holds a rope, so the walk's
/// physical compare is exact — the nested-leaf twin of the `Core::ValueEq`/key-path top-level compaction.
/// `bytes-compact` is REFCOUNT-NEUTRAL (it flattens the node IN PLACE and returns the same handle, a
/// no-op on an already-flat leaf) and `bytes_flatten` is content-preserving hence safe even on a SHARED
/// node, so it is sound for an element of ANY ownership (an owned `String.concat` result, or a BORROWED
/// String param the caller could have passed as a rope — the case a naive owned-only compile-time fix
/// would miss).
pub(crate) fn elem_needs_rope_compaction(db: &mut Db, id: StructId) -> bool {
    fn peel(ty: &Ty) -> &Ty {
        match ty {
            Ty::Nominal { inner, .. } => peel(inner),
            other => other,
        }
    }
    matches!(peel(&type_of(db, id)), Ty::String | Ty::Symbol | Ty::Bytes)
}

/// Whether `id`'s solved type is BigInt-VALUED — a bare `Ty::BigInt` OR a quantity over a BigInt
/// magnitude (`Ty::Qty { inner: BigInt }`). A `(Qty BigInt u)` erases to its inner BigInt handle, so
/// every place that materializes / classifies a constant BigInt as a heap handle (the `Core::ConstInt`
/// emit choke-point, the const-materialize-ops inserters, the borrow-ownership classifier) must treat a
/// BigInt-inner quantity the same — else a `(Qty.of (BigInt.of k) u)` constant emits as a raw `i64.const`
/// where an i32 handle is expected (invalid wasm). One helper so the peel is consistent across all sites.
pub(crate) fn is_bigint_valued(db: &mut Db, id: StructId) -> bool {
    // PEEL nominal + quantity via `peel_qty_ty` (strip_nominal → peel Qty → strip_nominal), NOT a bare
    // `Ty::BigInt`/`Ty::Qty{BigInt}` match: a single-variant single-payload NEWTYPE over BigInt — e.g.
    // `(type W (Mk BigInt))` — erases to `Ty::Nominal { inner: BigInt }`, so a constant `(Mk 1)` used as a
    // runtime value (a call arg) is a `Core::ConstInt` typed `Nominal{BigInt}`. The old bare match missed the
    // nominal wrapper → the ConstInt emit fell to the raw `i64.const` path → an i32 handle was expected at the
    // call → INVALID module (FACE-B of the nonzero-BigInt-recursive miscompile). `peel_qty_ty` reaches the
    // inner BigInt through the newtype (and a nominal-over-Qty-BigInt), the exact strip→peel→strip the
    // Float32 nominal-over-Qty fix (PR#743) established for its twin.
    matches!(peel_qty_ty(type_of(db, id)), Ty::BigInt)
}

/// The byte offset in the shared host `mem` where a RUNTIME string host-arg's bytes are marshaled (the
/// `_mem` runtime-arg path). The const-string host args occupy `[0, max(offset+len))` (the data segment);
/// the runtime scratch buffer starts past that — the max const-string end rounded up to a 256-byte
/// boundary, plus a 1 KiB gap (headroom so a future data-segment growth does not overlap). The shared mem
/// is 1 page (64 KiB); a runtime string longer than `65536 - scratch_base` would overrun — the caller
/// bounds this by the single-arg + fixed-buffer contract (a huge runtime string is a later increment; the
/// value-heap rope's length is not statically known here, so an over-long string is a runtime concern the
/// host boundary already sizes its read to `len`). Deterministic (no allocation state) — same base every call.
pub(crate) fn host_arg_scratch_base(layout: &Layout) -> u32 {
    let const_end = layout
        .host_strings
        .iter()
        .map(|(s, off)| off + s.len() as u32)
        .max()
        .unwrap_or(0);
    const_end.div_ceil(256) * 256 + 1024
}

pub(crate) fn heap_operand_ownership(db: &mut Db, id: StructId) -> Result<HandleOwnership, Reject> {
    match core_of(db, id) {
        // Constructors and calls produce a fresh owned reference (ownership transfers out). A map
        // construction/update (`map-empty`+inserts, `map-insert`, `map-remove`) returns a fresh owned
        // map handle exactly like a list/tuple constructor — the `value-eq` emit drops it after the compare.
        Core::SumNew { .. }
        | Core::Tuple { .. }
        | Core::Record { .. }
        | Core::ListNew { .. }
        // A list PRODUCER returns a FRESH owned list handle exactly like a `ListNew` constructor:
        // `vec-push` (`ListPush`) CONSUMES its input list + element and returns a NEW owned list (the old
        // version untouched — persistence); `vec-concat` (`ListConcat`) consumes both operands and hands
        // back a new vector; `vec-update` (`ListUpdate`) returns the updated list as a new handle. So such a
        // list as a BORROWING-op operand (`List.len`/`List.at`/`= `) is an owned temporary the emit reclaims
        // after the borrow — the LIST analog of the `BytesConcat`/`BytesSlice` producers below. WITHOUT
        // this, `List.len (List.push (build …) x)` / `List.len (List.concat a b)` / `List.at (List.update l
        // i x) j` fell to the `_ => decline` default, so the reclaim gate never fired and the fresh list
        // LEAKED one vector per call (value correct — a leak, not a miscompile). Classifying `ListPush`
        // Owned reclaims only the fresh PUSH RESULT that a borrowing op leaves un-consumed; the ACCUMULATOR
        // pattern `(build i n (List.push acc i))` returns the push as its TAIL (never a borrowing-op
        // operand), so its ownership is never consulted here and the FBIP single-consume fast path is
        // untouched. A shared/borrowed `acc` (rc>1) is already `dup`'d before the push by the Perceus retain,
        // so `vec-push` produces a genuinely fresh result — dropping it can never double-free the live `acc`.
        | Core::ListPush { .. }
        | Core::ListPrepend { .. }
        | Core::ListConcat { .. }
        | Core::ListUpdate { .. }
        // A fallible READ that returns an `(Option T)` builds a FRESH owned `sum-new` Some shell (or a
        // nullary None) around the extracted/copied payload: `List.at`/`Bytes.at` (`vec-get`/`bytes-get`
        // into `Some`), `Map.lookup` (the looked-up value dup'd into `Some`). So the Option RESULT is an
        // owned temporary — when it is a `Option.expect`/match SCRUTINEE, the `SumExpect`/`MatchSum` emit
        // reclaims the shell after the payload read (else the shell LEAKS one cell per call — the
        // live-objects-gate find). These BORROW their collection (that reclaim is handled at the op's own
        // emit); here we classify the fallible RESULT they hand out, which is a fresh owned sum.
        | Core::ListAt { .. }
        | Core::BytesAt { .. }
        // NOTE: `String.at` (`Core::StrAt`) also returns a FRESH owned `Some(char-view)`, but it is
        // DELIBERATELY NOT classified Owned here — a GLOBAL reclassification perturbs the MatchSum Stage-B
        // extraction-consume reclaim (a String.at-view consumed by an allowlisted `String.concat` nets +1 =
        // a latent Stage-B imbalance, v-mem-safety's separate follow-up). Instead the (2) SumExpect view/shell
        // reclaim treats a `StrAt` scrutinee as owned LOCALLY (in the SumExpect `reclaim_shell` emit, gated by
        // the `sumexpect_view_reclaim`/`sumexpect_shell_reclaim` membership) — same local>global discipline as
        // the reclaim_bytes-local (2) check, so the MatchSum/value-eq/Stage-B consumers see StrAt UNCHANGED.
        | Core::MapLookup { .. }
        | Core::MapNew { .. }
        | Core::MapInsert { .. }
        | Core::MapRemove { .. }
        // `Map.merge` (`MapMerge`) returns a FRESH owned map handle — `map-merge` CONSUMES both operands and
        // hands back a new CHAMP map (the map analog of `ListConcat`). As a BORROWING-op operand (`Map.len
        // (Map.merge a b)` / `= (Map.merge …) m`) it is an owned temporary the emit must reclaim after the
        // borrow. WITHOUT this it fell to `_ => decline`, so the reclaim gate never fired and the fresh
        // merged map LEAKED its root cell (value correct — a leak, not a miscompile; WAT-confirmed: the
        // map-merge result was consumed by a borrowing `map-size` with no drop). The merge's ENTRIES are
        // dup'd from the operands (or immortal statics), so dropping only the fresh spine never double-frees.
        | Core::MapMerge { .. }
        // A constant string/bytes materializes a FRESH owned byte-leaf handle (`bytes-alloc`+`bytes-set`,
        // see the `Core::ConstStr`/`ConstBytes`/`BytesOf` emit), so — like a constructor — the `value-eq`
        // emit drops it after the borrowing compare. This is the `(= h "+")` shape: comparing a runtime
        // payload string against a constant-string literal.
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::BytesOf { .. }
        // A runtime Bytes/String PRODUCER returns a FRESH owned handle: `bytes-concat`/`bytes-slice`/
        // `bytes-compact` each consume their operand(s) and hand back a new sequence; `str-from-bytes`
        // transfers the validated buffer out as a String; `str-to-bytes` (= `bytes-compact`) flattens the
        // string's byte-rope out as a fresh Bytes leaf. So such a value as a DIRECT `value-eq` operand is
        // owned and the emit drops it after the borrowing compare — the `(= (String.to-bytes s) b"…")` shape
        // the compiler-in-Cadenza codec's byte round-trip compares.
        | Core::BytesConcat { .. }
        | Core::BytesSlice { .. }
        | Core::BytesCompact { .. }
        // `str-slice` (`String.slice`) returns a FRESH owned `(Option String)` — a fallible read that copies
        // the slice range into a new `Some(str)` (or `None`), exactly like `BytesAt`/`BytesSlice` above. So a
        // `String.slice` result used as a MatchSum SCRUTINEE is an OWNED computed boxed-sum, and its shell +
        // consumed payload must be reclaimed by the shell-reclaim child-dup path. WITHOUT this arm it fell to
        // the `_ => decline` ("ownership this backend cannot yet prove") default, so `sum_shell_reclaim_ok` /
        // `collect_shell_reclaim_child_dups` bailed and a payload CONSUMED MORE THAN ONCE off the slice (the
        // adv54b class: `tail` = `String.slice`'s payload fed to two `String.to-bytes` under one
        // `Bytes.concat`) got NO child-`dup` → the shared payload's ref was released twice → DOUBLE-FREE.
        | Core::StrSlice { .. }
        | Core::StrFromBytes { .. }
        | Core::StrToBytes { .. }
        // `str-nfc-normalize` (FINDING #23) hands out a FRESH owned String handle: it CONSUMES its operand and
        // returns either that same handle (already NFC) or a fresh normalized leaf with the original dropped —
        // either way an owned reference transfers out, exactly like `StrToBytes`/`BytesConcat`. So as a
        // borrowing-op operand it is Owned and the emit reclaims it after the borrow.
        | Core::NfcNormalize { .. }
        // `Blake3.of` on a runtime Bytes (`hash-blake3`, op 91) BORROWS its operand and returns a FRESH
        // owned 32-byte Bytes leaf. So a `Blake3.of` result used as a `value-eq` operand — the P4
        // dispatch shape `(= (Blake3.of msg-contract) id)` / `(= (Blake3.of x) (Blake3.of y))` — is an
        // owned temporary the borrow must reclaim. WITHOUT this it fell to `_ => decline` ("ownership this
        // backend cannot yet prove"), blocking a runtime-digest equality (a completeness gap, not a
        // miscompile) even though a `Bytes.of` of the same content compares fine.
        | Core::Blake3Of { .. }
        // `Ast.encode` (op 93) BORROWS its Ast operand and returns a FRESH owned Bytes document; `Ast.print`
        // (op 92) likewise returns a FRESH owned String. So such a result used as a BORROWING-op operand — a
        // bare `(= (Ast.encode a) (Ast.encode b))`, or the PROBE of `Set.contains`/`Map.lookup` keyed by
        // arena Bytes (`(Set.contains s (Ast.encode …))`) — is an owned temporary the borrow must reclaim.
        // WITHOUT this both fell to `_ => decline` ("ownership this backend cannot yet prove"): CHAMP
        // construction dedups arena Bytes fine (the constructor path never consults this), but the query-
        // entry / bare-compare lowering DECLINED (breaker xf3/xf4 + beq bare-`=`). The Ast twin of `Blake3Of`
        // / `ValueEncode` above.
        | Core::AstEncode { .. }
        | Core::AstPrint { .. }
        // A runtime `(bin …)` construction builds a FRESH owned Bytes on the rope heap (`bytes-alloc` +
        // per-segment range-check-and-write, exactly like `BytesOf`), so as a `value-eq` operand it is
        // Owned and the emit drops it after the borrowing compare. WITHOUT this a runtime `(bin …)` result
        // — spec'd a Bytes value — fell to the `_ => decline` below, so `(= (bin (u8 v)) b"…")` DECLINED
        // "an ownership this backend cannot yet prove" even though a `Bytes.of` of the same content compares
        // fine (a completeness gap, not a miscompile). `BinBitsBuild` (a `(bits v k)` run) is the same.
        | Core::BinBuild { .. }
        | Core::BinBitsBuild { .. }
        // A dependent-size / final-rest bin-match PAYLOAD read returns a FRESH owned Bytes: `BinSizedRead`
        // and `BinRestRead` both emit `dup(scrutinee); bytes-slice(copy, off, len)`, and `bytes-slice`
        // CONSUMES the dup'd copy + hands back a NEW owned slice (exactly like `BytesSlice`). So the slice
        // as a borrowing-op operand (`Bytes.len payload` — the payload binder lowers to an INLINE
        // `BinSizedRead`, so `BytesLen`'s operand IS this node) is an owned temporary the borrow must
        // reclaim. WITHOUT this the slice fell to `_ => decline`, `BytesLen`'s reclaim gate never fired,
        // and a fresh slice LEAKED one Bytes cell per dependent-size match (value-correct — a leak, not a
        // miscompile; witnessed by `dependent_size_bin_match_payload_read_leaves_no_live_objects`). The
        // Bytes analog of the `ListConcat`/`ListPush` producer gap fixed earlier.
        | Core::BinSizedRead { .. }
        | Core::BinRestRead { .. }
        // `Value.encode` returns a FRESH owned `Bytes` document handle (the runtime `value-encode` op mints
        // a new doc, borrowing its value operand), and `Value.decode` returns a FRESH owned value handle
        // (`sum-new(Some, decoded)` over the fresh `value-decode` result, or the nullary `None`). So a
        // `Value.encode`/`Value.decode` RESULT used as a borrowing-op operand — the round-trip `(let ((bs
        // (Value.encode v))) (Value.decode bs) …)` where `bs` feeds `value-decode` (a borrow) — is an owned
        // temporary the borrow must reclaim. WITHOUT this it fell to `_ => decline` ("ownership this backend
        // cannot yet prove"), blocking the whole R2 round-trip at emit (v-inference's decode-grounding got it
        // past typing, then this fired). The Bytes/value analog of the collection-producer arms below.
        | Core::ValueEncode { .. }
        | Core::ValueDecode { .. }
        // `Char.from-int` (`IntToCharChecked`) wraps the boxed codepoint into a FRESH owned `(Option Char)`
        // sum (`disc_some`/`disc_none`), exactly like `ValueDecode`'s `sum-new(Some, …)`/`None`. So its result
        // used as a MatchSum SCRUTINEE — `(match (Char.from-int n) ((Some c) …) ((None _) …))` — is an OWNED
        // computed boxed-sum whose Option SHELL the shell-reclaim must drop. WITHOUT this it fell to the
        // `_ => decline` default, so `sum_shell_reclaim_ok` bailed and the Option shell LEAKED one cell per
        // match (arm- AND payload-independent — the shell, not the boxed Char; v-mem's chfi triage). The Char
        // twin of the `StrSlice`(Option String) / `ValueDecode`(Option value) fresh-owned-sum producer arms.
        | Core::IntToCharChecked { .. }
        // A set construction/update/algebra (`set-empty`+inserts, `set-insert`, `set-remove`, union/
        // intersection/difference) returns a fresh owned set handle — the `value-eq` emit drops it.
        | Core::SetOf { .. }
        | Core::SetInsert { .. }
        | Core::SetRemove { .. }
        | Core::SetAlgebra { .. }
        // A collection ENUMERATION returns a FRESH owned `List` handle: `map-to-list` materializes the map's
        // entries as a new `List (Tuple k v)` (canonical key order), `set-to-list` the set's elements as a new
        // `List`. So the enumeration RESULT as a borrowing-op operand (`List.len (Map.to-list m)`) is an owned
        // temporary the borrow must reclaim. WITHOUT this they fell to `_ => decline`, the borrowing consumer's
        // reclaim gate (e.g. `ListLen`) never fired, and the fresh result list (+ its boxed entries) LEAKED
        // per call — value-correct, a leak not a miscompile; the enumeration analog of the `ListConcat`/
        // `ListUpdate` producer gap. (This governs the to-list RESULT; a leak of an owned-temporary SOURCE
        // handed TO to-list is a separate face — the to-list op borrows its source — tracked by
        // `map_or_set_to_list_over_an_owned_temporary_source_leaks_it_known_gap`.)
        | Core::MapToList { .. }
        | Core::SetToList { .. }
        // A BigInt PRODUCER returns a fresh owned handle: `bigint-of-i64` mints a leaf, and each
        // `bigint-add`/`-sub`/`-mul`/`-div` re-boxes a normalized result (the operands are borrowed, the
        // result is new). So a BigInt operand that is itself the result of another BigInt op is owned —
        // the enclosing op drops it after borrowing. (`BigIntToI64` returns an i64 scalar, never a handle,
        // so it is not a heap operand and never reaches here.)
        | Core::BigIntOfI64 { .. }
        | Core::BigIntBinOp { .. }
        // `rational-num`/`rational-den` return a FRESH owned BigInt handle (they unbox the Rational's
        // component + re-box it, borrowing the Rational), so a `Rational.numerator`/`denominator` result
        // used as a borrowing-op operand (e.g. `Int64.of (Rational.numerator r)`) is Owned — the enclosing
        // op drops it after borrowing.
        | Core::RationalNum { .. }
        | Core::RationalDen { .. }
        // A Rational PRODUCER likewise returns a fresh owned handle: `rational-of` (`RationalOfInts`/
        // `RationalOfIntWiden`) builds a new 2-handle node, and each `rational-add`/…-`div` re-normalizes
        // into a new node. So a Rational operand that is itself a Rational op's result is owned.
        | Core::RationalOfInts { .. }
        | Core::RationalOfIntWiden { .. }
        | Core::RationalBinOp { .. }
        // A HOST/PEER call returning a COMPOUND yields a fresh OWNED handle (a peer-bound effect returns a
        // runtime value the consumer now owns — the shared-runtime handle transport, U5/U11), exactly like
        // a defined-func `Call`. So a peer-returned compound projected/consumed here is an owned temporary
        // the enclosing op reclaims (U13) rather than leaking until run-end.
        | Core::HostCall { .. }
        | Core::Call { .. } => Ok(HandleOwnership::Owned),
        // A CONSTANT typed `BigInt` materializes to a FRESH owned handle at `emit` (the `Core::ConstInt`
        // arm routes a BigInt-typed constant through `bigint-of-i64`), exactly like `ConstStr` above — so
        // as a borrowing-op operand it is Owned and the emit drops it. This is what lets `Int64.of (if c
        // (BigInt.of 1) (BigInt.of 2))` narrow a BigInt-valued `if` whose branches are constant BigInts.
        // A constant BigInt (bare OR a BigInt-inner quantity — `is_bigint_valued` peels the `Qty`)
        // materializes to a FRESH owned handle at emit (the `Core::ConstInt` arm routes it through
        // `bigint-of-i64`), exactly like `ConstStr` — so as a borrowing bigint-op operand it is Owned and
        // the emit drops it. Covers `(+ (Qty.of (BigInt.of v) m) (Qty.of (BigInt.of 100) m))` (runtime +
        // constant BigInt quantity) and `Int64.of (if c (BigInt.of 1) (BigInt.of 2))`.
        Core::ConstInt(_) if is_bigint_valued(db, id) => Ok(HandleOwnership::Owned),
        // A CONSTANT Rational likewise materializes to a FRESH owned handle at `emit` (`bigint-of-i64` ×2
        // + `rational-of`), so as a borrowing-op operand it is Owned.
        Core::ConstRational(_, _) => Ok(HandleOwnership::Owned),
        // A `Core::Closure` builds a FRESH owned cell (`arr-alloc` of the code index + the dup'd captures),
        // ownership transferring out exactly like a `SumNew`/`Tuple`/`Record` constructor — so as the operand
        // of a BORROWING op it is an owned temporary the enclosing op reclaims after the borrow. This is the
        // SITE-A part (a): the eta-closure `((if c (T.Mk 0) (T.Mk 10)) 5)` distributes the projection into the
        // `if`, so the `CallClosure` operand is an `If`-of-partial-ctors whose arms are each a fresh owned
        // closure; the `Core::If` join (`join_arm_ownership`) can now flip that operand to Owned, which is the
        // precondition v-effects' CallClosure emit (part b) gates its `cell_slot` drop on. Classifying a
        // Closure Owned is UNCONDITIONALLY correct (a freshly-built cell is genuinely owned); the SOUNDNESS
        // that keeps a SHARED/curried/re-applied/forced-thunk cell from being wrongly dropped lives ENTIRELY
        // in v-effects' emit gate (full-arity apply + non-escaping result), NOT here — this arm only states
        // "the cell is owned", never "the cell may be dropped". No reclaim consumer today takes a Fn-typed
        // operand except that CallClosure emit, so until part (b) lands this arm is observably inert (a Fn
        // cannot be a List.len/value-eq/map-key/sum-scrutinee operand — those are type errors), which is what
        // lets part (a) land independently.
        Core::Closure { .. } => Ok(HandleOwnership::Owned),
        // A reference to a parameter or a kept `let`-binding — the owner elsewhere reclaims it.
        Core::Param { .. } | Core::LocalRef { .. } => Ok(HandleOwnership::Borrowed),
        // A list REST-BINDER read (`(list _ .. r)` → a `SumPayload` whose path's SOLE/final step is a
        // `RestFrom`) is the EXCEPTION: its emit is `dup(scrutinee); vec-drop(k)`, and `vec-drop` CONSUMES
        // the dup'd copy and hands back a NEW owned tail vector (the scrutinee's own count nets to zero —
        // see the `RestFrom` emit). So the extracted rest is a FRESH OWNED temporary, exactly like a
        // `ListConcat`/`ListUpdate` producer — NOT a borrow. A borrowing consumer (`List.len r`) must
        // therefore reclaim it after the borrow (else the fresh tail LEAKS one vector + its shared leaf per
        // read — the lm5 live-objects find), and as an arm RESULT it transfers out (Owned). Each reference
        // re-emits its own `dup`+`vec-drop` (an inline `SumPayload`, one per occurrence), so classifying it
        // Owned drops exactly one fresh vector per emit — never a double-free. Any OTHER path (a plain
        // `Payload`/`Elem` read) still BORROWS, as below.
        Core::SumPayload { path, .. }
            if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_))) =>
        {
            Ok(HandleOwnership::Owned)
        }
        // A payload/element READ (`sum-payload`/`arr-get`) BORROWS its operand — the enclosing compound
        // owns the sub-value, so the read yields a borrowed handle the `value-eq` emit must NOT drop
        // (`sum-payload`/`arr-get` read without transferring ownership; see `binding_escapes`). This is
        // the shape a recursive tree-walker compares — `(= h "+")` where `h` is a variant's tuple-payload
        // element bound via `SumPayload`. `SumExpect` (an `Option.expect` payload read) borrows likewise.
        // (A `String.at` `Some` payload is a rope slice, but it is COMPACTED at the producer — the `StrAt`
        // Some-branch flattens the slice before wrapping it — so the extracted payload is a flat leaf that
        // `value-eq` compares correctly without reclassifying this borrow as owned; see `Core::StrAt` emit.)
        Core::SumPayload { .. } | Core::SumExpect { .. } | Core::Proj { .. } => {
            Ok(HandleOwnership::Borrowed)
        }
        // Control flow: the operand's value is produced on one of several paths, so its ownership is the
        // JOIN of the reachable results — OWNED only when EVERY path provably yields a fresh owned
        // temporary (so the single post-compare drop is correct on all paths), else BORROWED. Classifying
        // BORROWED is always leak-safe: the emit then does NOT drop the operand, so a path that actually
        // produced an owned temporary merely LEAKS it (the conservative bias `binding_escapes` states — a
        // false "borrowed" only leaks) rather than risk freeing a borrowed path's still-live value under
        // its owner (a double-free). This mirrors the standalone-function path exactly: a body returning a
        // borrowed match payload leaves the value un-dropped and leaks the scrutinee it borrows from.
        //
        // `if` joins both arms; `let` forwards its body; a `match` (scalar / sum / list) joins its arm
        // bodies (`join_arm_ownership` / `sum_cont_ownership`). A bare-`Leaf`-rooted sum match folds to its
        // body in `lower` and never reaches here as a `MatchSum`.
        Core::If { then_, else_, .. } => Ok(join_arm_ownership(db, [then_, else_])),
        Core::Let { body, .. } => heap_operand_ownership(db, body),
        Core::Match { arms, .. } => {
            Ok(join_arm_ownership(db, arms.iter().map(|a| a.body)))
        }
        Core::MatchList { arms, .. } => {
            Ok(join_arm_ownership(db, arms.iter().map(|a| a.body)))
        }
        Core::MatchSum { root, .. } => Ok(sum_cont_ownership(db, &root)),
        // When the operand's ownership (its aliasing status — whether the enclosing op may reclaim it or must
        // leave it to another owner) cannot be established by any arm above, DECLINE rather than emit a
        // component whose dup/drop placement would be a guess: the aliasing discipline could not be proven
        // safe here, so refusing is the sound outcome, not an unchecked emit with unspecified aliasing.
        //= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
        //# The compiler MUST reject a program whose aliasing the memory discipline cannot establish as safe, rather than emit a component with unspecified aliasing behavior.
        _ => Err(Reject::unsupported(
            "borrowing op operand has an ownership this backend cannot prove",
        )),
    }
}

/// The JOIN of several result positions' ownership for a borrowing-op operand (see
/// [`heap_operand_ownership`]): [`HandleOwnership::Owned`] iff EVERY body is provably `Owned`, otherwise
/// [`HandleOwnership::Borrowed`]. A body whose ownership cannot be proven counts as `Borrowed` — the
/// leak-safe join value, so an unhandled arm shape never declines the whole match (it just leaves the
/// operand un-dropped, a leak, never a double-free). Empty (a match with no arms cannot reach a value)
/// is `Borrowed` — the safe default.
pub(crate) fn join_arm_ownership(
    db: &mut Db,
    bodies: impl IntoIterator<Item = StructId>,
) -> HandleOwnership {
    for body in bodies {
        if !matches!(heap_operand_ownership(db, body), Ok(HandleOwnership::Owned)) {
            return HandleOwnership::Borrowed;
        }
    }
    HandleOwnership::Owned
}

/// Ownership of a sum-match CONTINUATION as a borrowing-op operand — the join over every LEAF body the
/// decision tree can reach (mirrors `cont_child_ids`): a `Guarded` arm joins its body with the
/// fall-through `els`, a `LitTest` joins its `then_`/`els`, a `Switch` joins all its arms'
/// continuations. `Owned` iff every reachable leaf is provably `Owned`, else `Borrowed` (leak-safe).
pub(crate) fn sum_cont_ownership(db: &mut Db, cont: &crate::core::SumCont) -> HandleOwnership {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            if matches!(
                heap_operand_ownership(db, *body),
                Ok(HandleOwnership::Owned)
            ) {
                HandleOwnership::Owned
            } else {
                HandleOwnership::Borrowed
            }
        }
        crate::core::SumCont::Guarded { body, els, .. } => {
            match (
                heap_operand_ownership(db, *body),
                sum_cont_ownership(db, els),
            ) {
                (Ok(HandleOwnership::Owned), HandleOwnership::Owned) => HandleOwnership::Owned,
                _ => HandleOwnership::Borrowed,
            }
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            match (sum_cont_ownership(db, then_), sum_cont_ownership(db, els)) {
                (HandleOwnership::Owned, HandleOwnership::Owned) => HandleOwnership::Owned,
                _ => HandleOwnership::Borrowed,
            }
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms.iter() {
                if sum_cont_ownership(db, &a.cont) == HandleOwnership::Borrowed {
                    return HandleOwnership::Borrowed;
                }
            }
            HandleOwnership::Owned
        }
    }
}
