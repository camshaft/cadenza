use super::*;

/// The runtime op that BOXES the node at `id` (a tuple/record element) into a u32 heap handle, by its
/// solved type: an integer → `box-int` (an i64 payload), a boolean → `box-bool`. A COMPOUND element (a
/// nested tuple/record) is ALREADY a u32 handle — it is `arr-set` into the parent array as-is, with no
/// box op — so this returns `Ok(None)` for a compound (the caller skips the box). A type with no heap
/// representation at all (a function/type-value) DECLINES. Reads the solved type.
pub(super) fn box_op(db: &mut Db, id: StructId) -> Result<Option<&'static str>, Reject> {
    let ty = type_of(db, id);
    box_op_ty(db, &ty)
}

/// The box op for a collection ELEMENT/KEY/VALUE node, given the collection's DECLARED slot type. Prefers
/// the collection's declared type (which grounds a bare-literal element to the collection's width), but
/// FALLS BACK to the element NODE's own concrete type when the declared type is an unresolved `Var`/`Any`.
/// A collection built EMPTY — `(Set.of (list))`, `(Map.empty)` — leaves its element/key type an
/// unconstrained var: inference never unifies a later `Set.insert`/`Map.insert`'s element type back onto
/// the empty collection's element var, so the declared type stays `Var`. `box_op_ty` defaults a `Var` to
/// `box-int` (the uniform heap cell — correct for a genuinely-dead phantom position), which WRONGLY boxes
/// a live HEAP-HANDLE element (a String, a nested compound) as an integer → an invalid module. The
/// inserted element is a live value with a SOLVED type, so box it by that — exactly as `Core::Tuple` boxes
/// each element by its own node type via `box_op`. (A resolved declared type is authoritative and used
/// as-is; only an unresolved declared type defers to the element node.)
pub(super) fn box_op_for(
    db: &mut Db,
    node: StructId,
    declared: &Ty,
) -> Result<Option<&'static str>, Reject> {
    if matches!(declared, Ty::Var(_) | Ty::Any) {
        // Both the declared slot type AND (below) the node type are unresolved. `box_op(node)` →
        // `box_op_ty(Ty::Var)` = `box-int` (i64), the "dead phantom position" default. But if the value
        // node PROVABLY produces a live i32 HEAP HANDLE (a `Map.empty`/`sum-new`/`vec-*`/record/tuple —
        // known by its Core shape, independent of its unresolved type), box-int'ing it feeds an i32 to the
        // i64 `box-int` → an invalid module (the compiler-ml `function[27]` freeze: an at-scale
        // `Tree.Arena(Map.empty, …)` field whose element type never got pinned). A handle is already the
        // heap cell — store it AS-IS (`Ok(None)`), the same as a RESOLVED handle type takes. This is the
        // Var-default-is-unsafe-for-a-live-handle safety net; it fires only in the both-unresolved case,
        // so a genuinely-dead phantom position still takes the uniform box-int cell.
        if node_produces_heap_handle(db, node) {
            return Ok(None);
        }
        box_op(db, node)
    } else {
        box_op_ty(db, declared)
    }
}

/// Whether the value node PROVABLY produces a u32 heap HANDLE (an i32 that is a heap-cell handle, not a
/// boxed scalar) — read from its Core SHAPE, independent of its (possibly unresolved) solved type. Used by
/// [`box_op_for`] to avoid box-int'ing a live handle when the slot type is an unresolved `Var`/`Any` (which
/// would default to the i64 `box-int` and reject an i32 handle). Conservative: only shapes whose emit
/// leaves a handle on the stack. A scalar/const/arith node is NOT a handle (returns false → box normally).
pub(super) fn node_produces_heap_handle(db: &mut Db, node: StructId) -> bool {
    match core_of(db, node) {
        // A SumNew is a handle UNLESS it is an enum-disc (a bare i32 discriminant that DOES box as int).
        Core::SumNew { .. } => !node_is_enum_disc(db, node),
        Core::MapNew { .. }
        | Core::MapInsert { .. }
        | Core::MapRemove { .. }
        | Core::ListNew { .. }
        | Core::ListPush { .. }
        | Core::ListPrepend { .. }
        | Core::ListConcat { .. }
        | Core::ListUpdate { .. }
        | Core::SetOf { .. }
        | Core::SetInsert { .. }
        | Core::SetRemove { .. }
        | Core::SetAlgebra { .. }
        | Core::Record { .. }
        | Core::Tuple { .. }
        | Core::BytesOf { .. }
        | Core::BytesConcat { .. } => true,
        _ => false,
    }
}

/// The box op for a solved TYPE directly (not a node) — used where a map's key/value type is known but
/// no representative node is at hand (a `Map.lookup` value unbox reads `val_ty`). Mirrors [`box_op`].
pub(super) fn box_op_ty(db: &Db, ty: &Ty) -> Result<Option<&'static str>, Reject> {
    // An ENUM-DISCRIMINANT sum is a bare i32 discriminant, NOT a heap handle, so as a nested element it
    // boxes exactly like an integer (`box-int`, with the i32→i64 extend the caller applies) — checked
    // before the `Ty::Sum` "already a handle" arm below.
    if ty_is_enum_disc(db, ty) {
        return Ok(Some(OP_BOX_INT));
    }
    match ty {
        Ty::Int(_) => Ok(Some(OP_BOX_INT)),
        // A CHAR is an i32 code-point scalar (Char-rep 1/N); it boxes into the i64 heap cell exactly like a
        // narrow int (`box-int`, with the i32→i64 extend `is_narrow_int` now applies for `Ty::Char`), so a
        // Char can be a tuple/record element, sum payload, or map/set member (Char-rep 4/N). Read back with
        // `get-int` + the i64→i32 narrow (`get_op_ty`/`needs_get_int_narrow`).
        Ty::Char => Ok(Some(OP_BOX_INT)),
        Ty::Bool => Ok(Some(OP_BOX_BOOL)),
        // A FLOAT boxes into its width's dedicated leaf: Float64 → `box-float` (an `f64` slot, `valtype_of`
        // → F64, IS `box-float`'s arg), Float32 → `box-float32` (an `f32` slot IS `box-float32`'s arg). So
        // NEITHER needs coercion — the shared `emit_box_i32_to_i64_extend` before the box op is a no-op for
        // a non-int/non-enum-disc value. A Float32 gets its OWN 4-byte leaf (not a promoted f64) so its
        // canonical byte form + value-encode render are the f32's, not an f64's (`0.1f32` → `0.1`).
        Ty::Float(ft) if ft.ground_width() == 64 => Ok(Some(OP_BOX_FLOAT)),
        Ty::Float(ft) if ft.ground_width() == 32 => Ok(Some(OP_BOX_FLOAT32)),
        // A nested compound — a tuple/record, a SUM (its `sum-new` handle), a LIST (`vec-*` handle), a
        // MAP (its CHAMP `map-*` handle), a BYTES sequence (`bytes-*` handle), or a STRING (a UTF-8
        // byte-leaf handle) — is already a u32 handle, so it is `arr-set` into the parent array (or used
        // as a sum payload / a map key or value) as-is, no box op. A CLOSURE (`Ty::Fn`) is likewise a u32
        // cell handle (`valtype_of(Ty::Fn) = I32`) — a closure captured BY another closure is stored
        // as-is. A `Ty::Map` here is what lets a MAP be a KEY or VALUE of another map — its handle is
        // CANONICAL by construction (order-independent CHAMP), so the outer map's `champ_hash`/`champ_eq`
        // walk over the nested map key is exact, exactly as a nested tuple/record key already is.
        Ty::Tuple(_)
        | Ty::Record(_)
        | Ty::Sum { .. }
        | Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        | Ty::Bytes
        | Ty::String
        // A Symbol is a String byte-leaf at run time (the tagless heap has no `Shape::Sym`; a Symbol is
        // represented + compared exactly as its content String — see `Symbol.of`'s `bytes-compact` retag),
        // so as a tuple/record/list/map/set element it is ALREADY a heap handle stored as-is, exactly like
        // a String element. Without this a `(tuple (Symbol.of …) …)` element declined "needs the value
        // heap", while the identical String element worked (the rust backend already maps `Ty::Symbol` →
        // `String`; this brings the wasm compound-element layout to the same parity).
        | Ty::Symbol
        // A BigInt is already a heap handle (its sign-magnitude leaf — `bigint-of-i64`/arithmetic return
        // handles), so as a nested element it is stored as-is, exactly like a String/List/Map handle.
        | Ty::BigInt
        // A Rational is likewise already a heap handle (a normalized 2-BigInt-handle node — `rational-of`/
        // the arithmetic return handles), so as a map key / set element / tuple element it is stored
        // as-is; its `champ_eq`/`champ_hash` descend the two child leaves (value equality by component).
        | Ty::Rational
        | Ty::Fn(_, _) => Ok(None),
        // A quantity erases to its inner numeric type (`lower` strips the `Qty`), so box it by that inner
        // type — a `(Qty Int64 u)` element boxes exactly as an `Int64` element.
        Ty::Qty { inner, .. } => box_op_ty(db, inner),
        // A NOMINAL newtype erases to its inner (the tag adds nothing to the runtime representation) — box
        // by the inner type. A scalar-inner newtype boxes as its scalar; a recursive newtype's inner is a
        // finite `Ty::Sum`-back-edged compound (a handle), boxed as-is. This is what lets a newtype value
        // (incl. a recursive one's self-referential cell) sit in a tuple/sum/collection slot.
        Ty::Nominal { inner, .. } => box_op_ty(db, inner),
        // A FREE var / `Any` element — inference never determined this position's type. That happens ONLY
        // when NO value ever flows through it: a DEAD match arm (`(match (Some (Ok x)) … ((Err e) e) …)`
        // where no `Err` is ever built leaves the Err type a free var, and its `e` read is unreachable), or
        // a phantom parameter. Ground it to the uniform i64 heap cell (`box-int`) — the SAME default a
        // deferred integer width takes: since no value observably flows, any consistent representation is
        // correct, and the unreachable read/store just needs SOME valid op to emit. This lets a program
        // that MATCHES an un-built variant (a total match on a `Result` only ever `Ok`) compile, rather
        // than declining the dead arm's phantom-typed payload read. (A LIVE value never has a free-var
        // type here — inference would have solved it — so this cannot mask a real unresolved-type bug.)
        Ty::Var(_) | Ty::Any => Ok(Some(OP_BOX_INT)),
        Ty::Unit => Ok(None),
        other => Err(Reject::unsupported(format!(
            "a tuple element of type {} requires the value heap, which is not supported",
            other.render_name(&db.name_ctx())
        ))),
    }
}

/// The runtime op that UNBOXES a u32 heap handle back to the value the node at `id` projects — the dual
/// of [`box_op`], keyed by this projection's solved type: an integer → `get-int`, a boolean →
/// `get-bool`. A COMPOUND projection (the element is itself a nested tuple/record) needs NO unbox — the
/// handle `arr-get` yields IS the nested compound — so this returns `Ok(None)` (the caller uses the
/// handle as-is). A projection of a type with no heap representation declines.
pub(super) fn get_op(db: &mut Db, id: StructId) -> Result<Option<&'static str>, Reject> {
    let ty = type_of(db, id);
    get_op_ty(db, &ty)
}

/// The unbox op for a solved TYPE directly (not a node) — the dual of [`box_op_ty`], used where a value
/// type is known but no node is at hand (a `Map.lookup` reads its `Some` payload back by `val_ty`).
pub(super) fn get_op_ty(db: &Db, ty: &Ty) -> Result<Option<&'static str>, Reject> {
    // An ENUM-DISCRIMINANT sum was boxed as an integer (see `box_op_ty`), so it is read back with
    // `get-int` (and the caller narrows i64→i32) — NOT used as a handle. Checked before the `Ty::Sum` arm.
    if ty_is_enum_disc(db, ty) {
        return Ok(Some(OP_GET_INT));
    }
    match ty {
        Ty::Int(_) => Ok(Some(OP_GET_INT)),
        // A CHAR reads back from its i64 heap cell with `get-int`; `needs_get_int_narrow` (which now covers
        // `Ty::Char`) narrows the i64 to the i32 code-point slot. The dual of `box_op_ty(Char)` (Char-rep 4/N).
        Ty::Char => Ok(Some(OP_GET_INT)),
        Ty::Bool => Ok(Some(OP_GET_BOOL)),
        // A FLOAT reads back with its width's op: Float64 → `get-float` (an `f64`), Float32 → `get-float32`
        // (an `f32`, the value's machine slot) — the duals of `box_op_ty`'s Float arms, both coercion-free.
        Ty::Float(ft) if ft.ground_width() == 64 => Ok(Some(OP_GET_FLOAT)),
        Ty::Float(ft) if ft.ground_width() == 32 => Ok(Some(OP_GET_FLOAT32)),
        // A nested compound / SUM / LIST / MAP / BYTES / STRING handle `arr-get` (or `sum-payload`) yields
        // is used as-is — no unbox. A CLOSURE (`Ty::Fn`) is a u32 cell handle too — a captured fn-typed
        // value (`Core::Captured` of a closure) reads back the handle directly, ready for a `call_indirect`.
        // A `Ty::Map` here is the dual of `box_op_ty`'s Map arm: a map read back from a heap slot (a map
        // stored as another map's key/value, or a tuple/sum element) is used as its handle directly.
        Ty::Tuple(_)
        | Ty::Record(_)
        | Ty::Sum { .. }
        | Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        | Ty::Bytes
        | Ty::String
        // A Symbol reads back as its String byte-leaf handle directly (dual of `box_op_ty`'s Symbol arm —
        // a Symbol IS a String at run time, so no unbox), so a Symbol tuple/record/element read is used
        // as-is like a String element.
        | Ty::Symbol
        // A BigInt handle read back from a heap slot is used as-is (dual of `box_op_ty`'s BigInt arm).
        | Ty::BigInt
        // A Rational handle read back from a heap slot is used as-is (dual of `box_op_ty`'s Rational arm).
        | Ty::Rational
        | Ty::Fn(_, _) => Ok(None),
        // A quantity erases to its inner numeric type — unbox by that inner type (the dual of `box_op_ty`).
        Ty::Qty { inner, .. } => get_op_ty(db, inner),
        // A NOMINAL newtype erases to its inner — unbox by the inner type (the dual of `box_op_ty`'s
        // Nominal arm), so a newtype value read back from a heap slot uses the right unbox.
        Ty::Nominal { inner, .. } => get_op_ty(db, inner),
        // A FREE var / `Any` projection — a DEAD arm reading an un-built variant's phantom payload (the dual
        // of `box_op_ty`'s free-var arm). Ground to `get-int` (the i64 cell): the read is unreachable (no
        // value of this type ever flows), so the op only needs to be VALID, not value-correct. This lets a
        // total match on a partly-un-built sum (`(Result C ?)` only ever `Ok`) compile its dead `Err` arm.
        Ty::Var(_) | Ty::Any => Ok(Some(OP_GET_INT)),
        Ty::Unit => Ok(None),
        other => Err(Reject::unsupported(format!(
            "projecting a tuple element of type {} requires the value heap, which is not supported",
            other.render_name(&db.name_ctx())
        ))),
    }
}

/// Push the inline-unit sentinel a UNIT value occupies when it crosses into a value-heap slot. A `Unit`
/// has NO machine slot (`valtype_of(Unit) = None`), so the value itself (`Core::Unit`) emits NOTHING — but
/// a heap slot (a tuple/record element, a sum payload, a collection key/value/element, a closure capture)
/// still needs SOME handle to keep its positional index aligned, so it holds `IMM_UNIT`: a low-bit-tagged
/// immediate (RC-noop, no runtime import), the SAME sentinel a nullary/Unit-payload sum uses (see the
/// `Core::SumNew` single-payload arm). Without this a Unit element pushed nothing and the following
/// `arr-set`/`sum-new` underflowed the stack → an INVALID module.
pub(super) fn emit_unit_slot(out: &mut Emit) {
    out.push(Lir::ConstI32(super::super::runtime_abi::IMM_UNIT as i32));
}

/// Emit the built-in `None` Option handle: `sum-new(disc_none, IMM_UNIT)`. The nullary variant's unit
/// payload is the inline-unit CONSTANT (`IMM_UNIT`), NOT a runtime `arr-alloc(0)` CALL — the runtime's
/// `arr-alloc(0)` returns exactly `imm_unit()`, so pushing the derived constant is equivalent and drops one
/// import call per `None` (the same optimization the `Core::SumNew` nullary path uses). Shared by the
/// `List.at`/`Map.lookup`/`String.at`/`Bytes.at` and the `?`-desugar None arms; leaves one handle on the
/// stack. The caller owns the enclosing `Else`/`End` and any owned-temp reclaim around the arm.
pub(super) fn emit_none_option(disc_none: u32, out: &mut Emit) {
    out.push(Lir::ConstI32(disc_none as i32)); // [disc_none]
    emit_unit_slot(out); // [disc_none, unit-payload]
    out.push(Lir::CallImport(OP_SUM_NEW)); // [None-handle]
}

/// Emit `probed-disc == disc` given the probed discriminant already on the stack. `disc == 0` is a single
/// `i32.eqz` (the FIRST declared variant `Some`/`Ok`/… — the common first-arm test), not the two-instruction
/// `const 0 ; i32.eq`; a nonzero disc is `const disc ; i32.eq`. The sum-discriminant twin of the scalar/probe
/// eqz special case (cycle 43). Leaves a 0/1 i32 on the stack for the following `if`/`select`.
pub(super) fn push_disc_eq(disc: u32, out: &mut Emit) {
    if disc == 0 {
        out.push(Lir::I32Eqz);
    } else {
        out.push(Lir::ConstI32(disc as i32));
        out.push(Lir::I32Eq);
    }
}

/// Emit the heap-STORE tail AFTER a value node has been emitted (its machine slot is on the stack, or —
/// for a `Unit` — NOTHING was pushed). Leaves exactly ONE handle for the following `arr-set`/`sum-new`/
/// insert: a SCALAR boxes (extending a narrow int i32→i64 first, as `box-int` takes an i64 cell); a
/// COMPOUND is already a u32 handle, left as-is; a UNIT substitutes the inline-unit sentinel (the value
/// pushed nothing). `boxed` is the caller's `box_op`/`box_op_for` classification — passed, not recomputed,
/// because a collection slot's box type may come from a DECLARED type, not the node — and both a compound
/// handle AND a Unit map to `None`, so the Unit case is distinguished by the node's own solved type
/// (stripping a nominal, exactly as `box_op_ty`'s recursion does). Any rope compaction the caller applies
/// stays AFTER this call (compaction is String/Bytes-only, never a Unit).
pub(super) fn emit_heap_store_tail(
    db: &mut Db,
    node: StructId,
    boxed: Option<&'static str>,
    out: &mut Emit,
) {
    match boxed {
        Some(op) => {
            emit_box_i32_to_i64_extend(db, node, out);
            out.push(Lir::CallImport(op));
        }
        None if matches!(type_of(db, node).strip_nominal(), Ty::Unit) => emit_unit_slot(out),
        None => {}
    }
}

/// Emit the heap-READ tail AFTER `arr-get`/`sum-payload`/`vec-get` has pushed the slot's handle — the dual
/// of [`emit_heap_store_tail`]. A SCALAR unboxes (`get-int`/`get-bool`/…, then a narrow int narrows
/// i64→i32); a COMPOUND handle is used as-is; a UNIT DROPS the sentinel handle the read yielded, because a
/// projected `Unit` leaves NO machine value (`valtype_of(Unit) = None`). Without the drop the `IMM_UNIT`
/// handle stayed on the stack where nothing was expected → a stack-type mismatch (an INVALID module).
/// `unboxed` is the caller's `get_op`/`get_op_ty` result (`None` for BOTH a compound and a Unit; the Unit
/// case is distinguished by `id`'s solved type).
pub(super) fn emit_heap_read_tail(
    db: &mut Db,
    id: StructId,
    unboxed: Option<&'static str>,
    out: &mut Emit,
) {
    match unboxed {
        Some(op) => {
            out.push(Lir::CallImport(op));
            if needs_get_int_narrow(db, id) {
                out.push(Lir::I32WrapI64);
            }
        }
        None if matches!(type_of(db, id).strip_nominal(), Ty::Unit) => out.push(Lir::Drop),
        None => {}
    }
}

// The heap stores an integer as an i64 cell (`box-int` takes `s64`, `get-int` returns `s64`), but a
// NARROW-width integer (`Int8`/`Int16`/`Int32`/`UInt8`…) lives in an i32 machine slot. So a narrow
// element must be EXTENDED i32→i64 before `box-int`, and a narrow projection NARROWED i64→i32 after
// `get-int` — otherwise the emitted `box-int`/op has a mismatched operand slot (i32 vs the i64 the heap
// ABI expects, or i64 vs the i32 a narrow op expects) and wasm rejects the function. A full-width
// integer (Int64/UInt64) already occupies the i64 slot; a boolean is an i32 the heap boxes as-is. This
// is the heap-boundary analogue of `emit_wrap`'s slot move (the heap holds a width-erased i64 cell; the
// compiler normalizes at the box/unbox edge).

/// True iff the node at `id` is a NARROW integer (an i32-slot integer: width ≤ 32). Such a value must
/// cross the i64 heap-cell boundary with an explicit slot conversion.
pub(super) fn is_narrow_int(db: &mut Db, id: StructId) -> Option<Machine> {
    // `strip_nominal`: an ERASED single-variant newtype over a narrow int — `(type W (Wrap UInt8))` — has
    // the SAME machine rep as its inner narrow int (a raw i32 slot), so it must widen i32→i64 before
    // `box-int` exactly as a bare narrow int does. WITHOUT the strip, a `(W.Wrap n)` element/payload node
    // typed `Ty::Nominal(W, Int(u8))` returned None → the widen was skipped → `box-int` got a raw i32 → an
    // INVALID component (`expected i64, found i32`) when the erased narrow newtype was boxed into a tuple/
    // sum/list element. A bare `UInt8` (`Ty::Int(u8)`) matched + widened fine; only the newtype-wrapped
    // narrow int slipped through. (A nominal over a FULL-width Int64 strips to a non-slot32 int → still None,
    // no extend, correct.)
    //
    // PEEL `Ty::Qty`: a quantity over a narrow int — `(Qty Int8 u)` — erases to its inner narrow int (the
    // unit is a compile-time value; the runtime rep IS the inner narrow int's raw i32 slot), so it needs
    // the SAME i32→i64 widen before `box-int` and i64→i32 narrow after `get-int`. Without the peel, a
    // `(Qty Int8 u)` stored as a MAP VALUE / heap slot returned None here → the narrow after `get-int` was
    // skipped → `get-int`'s i64 was left where the i32 narrow-int slot was expected (e.g. a `Map.lookup`
    // result escaping the Option match as a Qty) → INVALID component (`expected i32, found i64`). The
    // peel mirrors the sibling `box_op_ty`/`get_op_ty`/`is_heap_type` arms that already descend into a
    // `Ty::Qty` inner. (A bare narrow int + an erased narrow newtype already matched; only a Qty-wrapped
    // narrow int slipped through — the value-form materialization twin of the arith-dispatch peel family.)
    let solved = type_of(db, id);
    let stripped = solved.strip_nominal();
    let inner = match stripped {
        Ty::Qty { inner, .. } => inner.strip_nominal(),
        other => other,
    };
    match inner {
        Ty::Int(it) => {
            let m = Machine::of(*it);
            m.slot32.then_some(m)
        }
        // A CHAR is a Unicode code point in an i32 machine slot (`valtype_of(Ty::Char) = I32`, Char-rep
        // 1/N; `int_ty_of(Char) = fixed(signed, 32)`). Boxed into an i64 heap cell as a compound element /
        // sum payload (`box_op_ty(Char) = box-int`, `get_op_ty(Char) = get-int`, Char-rep 4/N), it needs the
        // SAME i32→i64 extend before `box-int` and i64→i32 narrow after `get-int` a narrow int does — so it
        // rides this predicate exactly like `Int32`. Code points are 0..=0x10FFFF (bit 31 clear), so
        // sign-vs-zero extend is immaterial; `signed` matches `int_ty_of(Char)` for consistency.
        Ty::Char => {
            let m = Machine::of(IntTy::fixed(true, 32));
            m.slot32.then_some(m)
        }
        _ => None,
    }
}

/// After `get-int`ing a value BACK from an i64 heap cell, does it need narrowing i64→i32 to its machine
/// slot? True for a NARROW int (its slot is i32) OR an ENUM-DISC value (a bare i32 discriminant boxed as
/// an i64 cell — the exact dual of `emit_box_i32_to_i64_extend`'s extend-on-store). Without the enum-disc
/// case an enum reached through a heap slot (a tuple/record element, a sum payload, an expect payload, a
/// closure capture) reads back as an i64 where its i32 discriminant slot is expected → wasm rejects the
/// module (`type mismatch: expected i32, found i64`). `get_op_ty`'s enum-disc arm returns `get-int` on the
/// promise that "the caller narrows i64→i32" — this is that narrow.
pub(super) fn needs_get_int_narrow(db: &mut Db, id: StructId) -> bool {
    is_narrow_int(db, id).is_some() || node_is_enum_disc(db, id)
}

/// Before `box-int`ing a value into a heap cell, widen it from an i32 slot to the i64 `box-int` expects.
/// Fires for a NARROW int (extended by ITS sign) OR an ENUM-DISC value (a bare i32 discriminant, extended
/// UNSIGNED — a discriminant is a small non-negative index). A full-width i64 int, or a non-scalar, needs
/// no extend. Shared by every `box-int` payload/element site (a sum payload, a tuple/record element, a
/// closure capture, a map value) — an enum-disc payload (`(Some (Green))`, a `Color` element in a tuple)
/// must widen exactly like a narrow int, or an i32 reaches the i64 `box-int` and wasm rejects the module.
pub(super) fn emit_box_i32_to_i64_extend(db: &mut Db, id: StructId, out: &mut Emit) {
    // S141 (NON-TAIL width mismatch, fuzzer nontail witness): an ascription `(: e Narrow)` ERASES in
    // lowering (`Resolved::Annot` → `core_of(expr)`, lower.rs) with NO coercion, so `type_of(id)` is the
    // narrow (i32-slot) ascribed type while the value ACTUALLY emitted is `e`'s. When `e` is a `Core::Call`
    // whose callee committed to an i64 result — a recursive helper whose result type stayed a var / widened
    // to `Int64` in its own body, so its emitted wasm function is `(result i64)` — the stack holds an i64,
    // and the `is_narrow_int` extend below would run `i64.extend_i32_u` on it → "type mismatch: expected
    // i32, found i64" (invalid wasm). Mirror #5749's `emit_tail` fix: reconcile against the value's ACTUAL
    // emitted valtype (the callee's committed result valtype, the SAME source of truth as its `(result …)`
    // decl), NOT `type_of(id)` which the ascription narrowed. `box-int`'s heap cell is i64, so an
    // already-i64 value needs NO extend. (A callee that genuinely returns the narrow int emits `(result
    // i32)` → `call_result_valtype` is I32 → the extend below fires correctly, unchanged.)
    if call_result_valtype(db, id) == Some(ValType::I64) {
        return;
    }
    if let Some(m) = is_narrow_int(db, id) {
        out.push(if m.signed {
            Lir::I64ExtendI32S
        } else {
            Lir::I64ExtendI32U
        });
    } else if node_is_enum_disc(db, id) {
        // An enum-disc value is a non-negative i32 discriminant → zero-extend to the i64 cell.
        out.push(Lir::I64ExtendI32U);
    }
}

/// The ACTUAL emitted result valtype of `id` WHEN it lowers to a direct `Core::Call` — the callee's
/// COMMITTED result valtype (the `valtype_of` of its body's solved type, all params bound so the body type
/// IS the result), which is EXACTLY what the callee's emitted wasm function declares as its `(result …)`
/// and therefore what a `call` to it leaves on the stack. This is NOT `type_of(id)`: a call-site ascription
/// `(: (rec …) UInt8)` erases in lowering and narrows `type_of(id)` while the callee still returns its own
/// (possibly i64) width. Mirrors `emit_tail`'s #5749 callee-valtype derivation, reused at the non-tail box
/// boundary (`emit_box_i32_to_i64_extend`). `None` for a non-Call node or a callee with no inspectable body
/// (an import) — in which case the caller keeps its `type_of`-driven coercion.
pub(super) fn call_result_valtype(db: &mut Db, id: StructId) -> Option<ValType> {
    let callee = match core_of(db, id) {
        Core::Call { callee, .. } => callee,
        _ => return None,
    };
    let body = db.defs.get(callee).and_then(|d| d.body)?;
    valtype_of(&type_of(db, body))
}

/// A selected function body: its flat instruction sequence, the value types of its declared (non-
/// parameter) locals in slot order, its parameter value types, and its solved return type (for the
/// type section). A body may take parameters and declare locals (the scratch a guarded operation
/// reserves, and any persistent slot a kept `let` binding holds).
#[derive(Clone)]
pub struct SelectedFunc {
    pub params: Vec<ValType>,
    pub ret: Ty,
    pub code: Vec<Lir>,
    pub declared: Vec<ValType>,
    /// The AST occurrence this function's body was selected from — the source-attribution anchor for
    /// debug info (`DESIGN-debug-info-rcdzc.md` §2.1b). Carried so `serialize` can pair each function's
    /// emitted code byte range with the source `StructId` (→ span, via the `spans` sidecar), which is
    /// what a DWARF `DW_TAG_subprogram`'s `low_pc`/`high_pc` + line-program rows need. This is
    /// FUNCTION-granularity attribution; per-statement (per-`Lir`-run) is a later refinement that needs
    /// threading the current node through the emit family. `None` for a synthesized function (an escape
    /// walker) with no single source body.
    pub src_body: Option<StructId>,
    /// Named SCALAR locals for debug-info variable inspection (D3, `DESIGN-debug-info-rcdzc.md` §2.4) —
    /// each a `(wasm local slot, source name, solved scalar type)`. Populated with the function's scalar
    /// PARAMETERS (slots `0..n`, the common `print n` target). A `DW_TAG_variable` DIE emits from each so
    /// a debugger can read the value. A compound (heap-handle) param is omitted — DWARF cannot walk the
    /// tagless heap (§3), so only scalars appear. Function-scoped `let`-bindings also land here; MATCH
    /// binders — live only within one match expression, in a REUSED scratch slot — are NOT flat locals
    /// but scoped `scopes` below. Empty unless debug is requested.
    pub locals: Vec<LocalVar>,
    /// Scalar MATCH-BINDER lexical scopes (D3, `DESIGN-debug-info-rcdzc.md` §2.4): each a `[Lir start,
    /// end)` range + the binder locals visible there. Distinct from `locals` because a match binder is
    /// live only within its match (its slot is later reused), so the backend fences it in a
    /// `DW_TAG_lexical_block` with a PC range rather than a function-scoped `DW_TAG_variable`. Ranges are
    /// post-peephole Lir indices; `dwarf_funcs_for` maps them to code offsets. Empty unless debug.
    pub scopes: Vec<MatchScope>,
    /// Per-CONSTRUCT source line markers (`DESIGN-debug-line-granularity-rcdzc.md`): `(Lir index,
    /// source occurrence)` at each point a distinct source construct's evaluation begins, in emission
    /// order, remapped through the peephole pass. The backend turns each into a `.debug_line` row (one
    /// per source LINE the code visits, after dedup/collapse), so a debugger steps line-by-line instead
    /// of resting on the function's opening line. Empty for a single-construct body → the line program
    /// falls back to one function-entry row (the function-granularity behavior, preserved). Always
    /// collected (cheap); only READ under a debug target.
    pub stmt_lines: Vec<(u32, StructId)>,
}

/// A named scalar local for debug info (D3): its wasm local slot, source name, solved scalar type, and
/// whether it is a function PARAMETER (vs a `let`-binding local) — so the backend picks
/// `DW_TAG_formal_parameter` vs `DW_TAG_variable`.
#[derive(Clone, Debug)]
pub struct LocalVar {
    pub slot: u32,
    pub name: String,
    pub ty: Ty,
    pub is_param: bool,
}
