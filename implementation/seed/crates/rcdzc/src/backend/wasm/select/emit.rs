use super::*;

/// Whether `id` is a NESTED-COMPOUND `Core::Proj` whose emit DUP'd the extracted child into a standalone
/// OWNED handle — i.e. the SAME gate the `Core::Proj` emit (this file) uses to `dup`-child + `drop`-record:
/// operand is OWNED (a fresh producer, e.g. `(mk i)`) + NOT slot-materialized + the projected element is a
/// NESTED-COMPOUND heap child (`get_op` `None`, not a scalar copy, not `Unit`). When true, the extracted
/// child is a fresh standalone-owned handle (rc1) that a BORROWING scalar-read consumer
/// (`Map.len`/`List.len`/`Bytes.len`/`Set.len`) must `drop` after its borrow — else it leaks (the Map.len-
/// over-a-projected-fresh-record disjoint-slot leak, corpus-05 #4547). DROP-IFF-DUP'D: this MUST mirror the
/// `Core::Proj` emit's dup gate EXACTLY — a mismatch is a double-free (drop with no dup) or a leak (dup with
/// no drop). Scalar elements (`get_op` `Some`) copy out and are NEVER dup'd here (so this returns false).
fn owned_proj_child_dupd(db: &mut Db, id: StructId, slots: &HashMap<StructId, u32>) -> bool {
    if let Core::Proj { operand, .. } = core_of(db, id) {
        !slots.contains_key(&operand)
            && matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            )
            && matches!(get_op(db, id), Ok(None))
            && !matches!(type_of(db, id).strip_nominal(), Ty::Unit)
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // A node MATERIALIZED into a scratch slot reads back as a `local.get`, not a recomputation. A
    // sum-match evaluates a non-reusable scrutinee ONCE into a slot (`emit_sum_match_arms`) and records
    // `(scrutinee-id → slot)` here, so every per-probe / per-payload re-reference of the scrutinee reads
    // the slot instead of rebuilding the value (which for a `List.at`/call scrutinee would both recompute
    // AND collide with arm-body scratch — an invalid module). Keyed by the node's OWN occurrence, distinct
    // from the binder-keyed `Param`/`LocalRef` entries, so this never shadows a binding.
    if let Some(&slot) = slots.get(&id) {
        out.push(Lir::LocalGet(slot));
        return Ok(());
    }
    // EMIT-WALK INSTRUCTION BUDGET (finding-24 sibling): the Core IR is a DAG the emit walk serializes as a
    // TREE (re-descending each shared `StructId`), so a handler whose compound threaded state is re-expanded
    // by each dispatch routing through the branching arm explodes super-linearly (driver = dispatches-
    // through-arm, not branch count K). Check the emitted-instruction count on EVERY node entry, so a super-linear body trips
    // this MID-walk and declines cleanly — before the ~593KB that breaks the engine's function-size/locals
    // limit ever serializes (reject-not-miscompile). A legitimately large LINEAR function stays well under
    // the bound; see `EMIT_INSTRUCTION_BUDGET`. The durable linear fix is sharing-aware emit (a follow-up).
    if out.code.len() > EMIT_INSTRUCTION_BUDGET {
        return Err(Reject::decline(
            "emit-walk instruction budget exceeded: a handler-derived Core DAG serializes as a tree \
             whose expansion exceeds the wasm function-size limit (a resume/next-state fan-out re-expanded \
             by each dispatch routing through the branching arm over a compound threaded state); pending \
             sharing-aware emit that binds a shared subtree once",
        ));
    }
    // EMIT-WALK SCRATCH-LOCALS BUDGET (the SECOND axis): a per-branch-compound-recompute body (rps1) can
    // slip UNDER the instruction budget yet mint scratch slots past the wasm ~50000 per-function locals cap
    // ("too many locals" — an INVALID module). Bound the running high-water `*high` too, so such a body
    // declines cleanly rather than overrunning the cap. See `EMIT_LOCALS_BUDGET`.
    if *high > EMIT_LOCALS_BUDGET {
        return Err(Reject::decline(
            "emit-walk scratch-locals budget exceeded: a handler-derived Core DAG serializes as a tree \
             whose per-branch recompute mints scratch slots past the wasm per-function locals limit; \
             pending sharing-aware emit that binds a shared subtree once",
        ));
    }
    // DEBUG (per-construct line rows): mark this node's first instruction with its source occurrence,
    // so a `.debug_line` row maps the code offset to its line. Recorded for every USER node (a prelude/
    // synthesized node has no span; `Emit::mark` dedups a repeated offset). The operand-consuming
    // helpers (`emit_operand`/`emit_checked_arith`/…) mark their own child ids too, so a construct whose
    // operands this `emit` never re-enters (an inline `Param`/`ConstInt`) still gets attributed there.
    if db.is_user_node(id) {
        out.mark(id);
    }
    match core_of(db, id) {
        Core::ConstInt(v) => {
            // A CONSTANT typed `BigInt` reaching `emit` is used as a RUNTIME VALUE (a map key/value, a set
            // element, a call argument, an op operand) — every such context wants an i32 HANDLE, not a raw
            // `i64.const`. A folded `(BigInt.of <const>)` is a `Core::ConstInt` typed BigInt; emitting it
            // as a bare scalar pushes an i64 where a handle is expected → an invalid module (the map-key /
            // call-arg miscompiles). MATERIALIZE it as a fresh BigInt leaf via `bigint-of-i64` (the value
            // fits i64 — a beyond-i64 constant BigInt is not yet built and declines earlier). A CONSTANT
            // BigInt that is a whole nullary EXPORT takes the baked-bytes `constant_value_form` path and
            // never reaches here; this is only the in-body runtime-value use.
            if is_bigint_valued(db, id) {
                // Materialize the constant as a heap leaf: fits-i64 via `bigint-of-i64`, beyond-i64 via
                // `bigint-of-bytes` on its baked canonical sign-magnitude bytes (`emit_const_bigint_leaf`).
                // `is_bigint_valued` also fires for a BigInt-inner quantity (`(Qty.of (BigInt.of k) u)`) —
                // it erases to this ConstInt and equally needs the handle, not a raw `i64.const`.
                emit_const_bigint_leaf(&v, out);
                return Ok(());
            }
            // Ground the literal to the machine width its solved type fixes. The constant must FIT the
            // width (checked at annotation time; a value that does not fit never reaches here for an
            // annotated literal), then it is emitted as the two's-complement BIT PATTERN of that width.
            // For an UNSIGNED value at/above the signed max (`UInt64.max = 2^64-1`), the bit pattern is
            // a negative i64 (`-1`) — correct at the machine level; the boundary lifts it back as u64.
            let it = int_ty_of(db, id);
            let width = it.ground_width();
            trace!(target: "rcdzc::select", node = id.0, signed = it.ground_signed(), width, "ground integer literal to its machine width");
            if !v.fits_width(it.ground_signed(), width) {
                trace!(target: "rcdzc::select", node = id.0, width, "literal does not fit its width (CDZ0302)");
                return Err(Reject::coded(
                    Code::IntOutOfRange,
                    "integer literal does not fit its width",
                ));
            }
            if width <= 32 {
                out.push(Lir::ConstI32(v.to_i32_bits(width)));
            } else {
                out.push(Lir::ConstI64(v.to_i64_bits()));
            }
            Ok(())
        }
        Core::ConstBool(b) => {
            out.push(Lir::ConstI32(if b { 1 } else { 0 }));
            Ok(())
        }
        // A constant string reaching `emit` as an in-body runtime VALUE — a string used where a heap
        // handle is needed (a map KEY/VALUE, a boxed element, a value threaded to a consumer), NOT folded
        // away. Build it as a FLAT UTF-8 BYTE LEAF, exactly as `Core::BytesOf` builds a byte sequence:
        // `bytes-alloc(len)` then a `bytes-set` per byte. This is BYTE-IDENTICAL to the runtime `str-new`
        // op (`op_str_new(s) = alloc(Vec::new(), s.into_bytes())` — a flat arity-0 leaf whose raw is the
        // UTF-8 bytes), so a String value built here is CANONICAL and `champ_eq`/`value-eq` compares two
        // equal strings correctly (raw-byte compare). We build via the lowerable `bytes-*` ops rather than
        // `str-new` because `str-new` takes a component-model `string` (canonical-ABI ptr+len+realloc), so
        // it is NOT callable from a core function body (`lowerable: false`); the byte-leaf build reaches the
        // identical rep with core-scalar ops. The reader already NFC-normalized the string, so the bytes
        // are canonical. (A CONSTANT string still folds in `lower` — a `= "a" "a"` never reaches here; this
        // is the path for a string that must become a runtime handle, e.g. `(map ("a" 1))`'s key.)
        Core::ConstStr(s) => {
            // §2d STATIC STRINGS: a constant String is a flat UTF-8 byte-leaf — an equal hoist target to a
            // constant Bytes (`try_emit_static_bytes` covers it via `constant_string_value`). Route it to
            // its build-once immortal global; else build the leaf inline below.
            if try_emit_static_bytes(db, id, layout, out) {
                return Ok(());
            }
            let bytes = s.as_bytes();
            out.push(Lir::ConstI32(bytes.len() as i32)); // [len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            for (i, &byte) in bytes.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf] (bytes-set returns the buffer)
            }
            Ok(()) // leaves [buf] — the string's flat UTF-8 byte-leaf handle (== str-new's rep)
        }
        // A constant char reaching `emit` as an in-body runtime VALUE — used where a machine slot is
        // needed (an `if`-branch join it flows through, a param/local it binds, a `Char.to-int` operand
        // that could not fold because a sibling arm made the char runtime). A `Char` is a Unicode SCALAR
        // VALUE, machine-identical to a 32-bit int, so it emits as a bare `i32.const` of its code point —
        // exactly the narrow-scalar `Core::ConstInt` emit (`valtype_of(Ty::Char) == I32`). Equality/
        // ordering on two CONSTANT chars still folds in `lower` and never reaches here; this is the
        // runtime-value use (Char-rep 1/N). The code point fits i32 (`0..=0x10FFFF`, surrogates excluded).
        Core::ConstChar(c) => {
            out.push(Lir::ConstI32(c as u32 as i32));
            Ok(())
        }
        // A constant `Rational` reaching `emit` as an in-body RUNTIME VALUE (a call arg, a map/set element,
        // an operand of a runtime rational op) MATERIALIZES to a runtime rational node: box each component
        // (num, den) as a BigInt leaf via `bigint-of-i64`, then `rational-of` (which consumes the two
        // handles + normalizes — the pair is already normalized, so this is idempotent). Both components
        // fit i64 for a materializable constant; a component beyond i64 declines (the arbitrary-magnitude
        // rational-component leaf is a later slice — no current case builds one). The whole-export constant
        // Rational takes the baked-bytes `constant_value_form` path and never reaches here — this is the
        // in-body runtime-value use, the analogue of the `Core::ConstInt`-typed-BigInt materialization.
        Core::ConstRational(n, d) => match (n.to_i64(), d.to_i64()) {
            (Some(nv), Some(dv)) => {
                out.push(Lir::ConstI64(nv));
                out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big]
                out.push(Lir::ConstI64(dv));
                out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big, den-big]
                out.push(Lir::CallImport(OP_RATIONAL_OF)); // → [rational handle]
                Ok(())
            }
            _ => Err(Reject::unsupported(
                "a constant Rational with a component beyond i64 is not supported at run time",
            )),
        },
        // The canonical NaN emits an `f64.const`/`f32.const` of the canonical NaN bit pattern at the
        // node's solved width — the same machine-slot value a returned NaN leaves on the stack (a NaN is a
        // real Float value that crosses the boundary, unlike a char). `f32::NAN`/`f64::NAN` are the one
        // canonical quiet NaN, matching the fold's `to_f64_bits` comparison basis.
        Core::ConstFloatNan => {
            let width = match peel_qty_ty(crate::infer::type_of(db, id)) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            if width == 32 {
                out.push(Lir::F32ConstBits(f32::NAN.to_bits()));
            } else {
                out.push(Lir::F64ConstBits(f64::NAN.to_bits()));
            }
            Ok(())
        }
        // Positive INFINITY emits an `f{32,64}.const` of the infinity bit pattern at the node's solved
        // width — a real Float value that crosses the boundary, exactly like the finite/NaN const. `INFINITY`
        // has one exact, target-independent bit form (no canonicalization, unlike NaN's platform payload).
        Core::ConstFloatInf => {
            let width = match peel_qty_ty(crate::infer::type_of(db, id)) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            if width == 32 {
                out.push(Lir::F32ConstBits(f32::INFINITY.to_bits()));
            } else {
                out.push(Lir::F64ConstBits(f64::INFINITY.to_bits()));
            }
            Ok(())
        }
        // A float CONSTANT emits an `f64.const`/`f32.const` of its canonical bit pattern at the node's
        // SOLVED width — the value a float occupies in its machine slot, and what an export returning a
        // float leaves on the stack (the boundary lifts it to the component `f64`/`f32`). A `Float32`
        // constant rounds the exact `Decimal` through binary32 (`as f32`) and emits `f32.const`. The width
        // is read off the solved type (the same read the boundary valtype uses).
        Core::ConstFloat(d) => {
            // PEEL `Ty::Qty`: a quantity over a Float32 — `(Qty Float32 u)` — erases to its inner f32
            // machine slot (the unit is a compile-time value), so the constant must emit `f32.const`. Without
            // the peel a `(Qty Float32)` node missed the `Ty::Float` arm and fell to the `_ => 64` default →
            // an `f64.const` while the heap box/get op is `box-float32`/`get-float32` (f32) → an INVALID
            // module (`expected f32, found f64`) when a `(Qty Float32)` constant is boxed into a heap slot
            // (e.g. a `Map.insert` value). The float twin of the `int_ty_of`/`is_narrow_int` Qty peels — the
            // Float32 value-form width reader was the last un-peeled const-width site.
            let width = match peel_qty_ty(crate::infer::type_of(db, id)) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => 64,
            };
            if width == 32 {
                let bits = (f64::from_bits(d.to_f64_bits()) as f32).to_bits();
                out.push(Lir::F32ConstBits(bits));
            } else {
                out.push(Lir::F64ConstBits(d.to_f64_bits()));
            }
            Ok(())
        }
        Core::Unit => {
            // Unit occupies no slot and pushes nothing.
            Ok(())
        }
        // A runtime RECORD — the SAME positional heap array as a tuple, its fields in canonical
        // (key-sorted) order (the `BTreeMap` iteration order, which the value-form renderer and every
        // `arr-get` index agree on). Field names are compile-time information the runtime does not hold;
        // at run time a record IS a tuple. So build it exactly as a tuple: `arr-alloc(n)`, then each
        // field value boxed by its type and `arr-set` into its sorted position. Leaves the record's u32
        // handle on the stack. (A record consumed only to read a field folds away in `lower`; a record
        // that survives to selection is a genuine runtime value — e.g. one that escapes to the host.)
        Core::Record { fields } => {
            // §2d STATIC COMPOUNDS: a markable constant record in the build-once table is built ONCE
            // (immortal) by the `start` init; read it here with a bare `global.get`. Else build inline below.
            if try_emit_static_compound(db, id, layout, out) {
                return Ok(());
            }
            out.push(Lir::ConstI32(fields.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            // The record's OWN solved type carries each field's DECLARED type by name — box each field by
            // THAT, not by the field-value NODE's type (which, at scale, can resolve to a bare `Ty::Var`
            // for an empty-collection field like `Map.empty` whose element vars never got pinned). A `Var`
            // node type defaults to `box-int` (i64) in `box_op_ty`, which WRONGLY boxes a live i32 Map
            // handle → "expected i64 found i32" at `Component::new` (the compiler-ml `function[27]` freeze:
            // `Tree.Arena(Map.empty, 0, Map.empty)`). `box_op_for` prefers the declared field type (here
            // `Ty::Map` → `Ok(None)`, store the handle as-is) and falls back to the node only when the
            // declared field type is itself unresolved — exactly as `Core::Set`/`Map` element boxing does.
            let field_tys = match crate::infer::type_of(db, id).strip_nominal() {
                crate::ty::Ty::Record(m) => Some((*m).clone()),
                _ => None,
            };
            for (i, (name, &value)) in fields.iter().enumerate() {
                // Each field starts its scratch ABOVE the running high-water, NOT at a fixed `base` — the
                // same disjoint-slot discipline `Core::Tuple`/`Core::ListNew` apply. A field initialized by
                // a checked-arith op stashes an i64 into a scratch slot; if a sibling (or an enclosing
                // call-boundary) slot at that number was already typed i32, reusing it re-types one wasm
                // local to two widths → an invalid module (`expected i64, found i32`). This is the
                // composed-call miscompile: `(f (f (record (a (+ (. r a) 1)) (b (. r b)))))` had field
                // `a`'s i64 arith `$r` collide with a record-assembler i32 slot. Advancing `field_base`
                // past each field's high-water hands each field fresh, never-typed slots.
                let field_base = base.max(*high);
                // [arr] ; push index ; push (box, if scalar) the field value ; arr-set → [arr]
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                // A bare `ConstFloat` field value takes the DECLARED field width, not its own default
                // `Float64`: a `(: x Float32)` field initialized by a bare `1.5` has value-node type
                // `Float64`, so `emit`'s `Core::ConstFloat` would push an `f64.const` while `box-float32`
                // expects `f32` → an INVALID module. Ground it to `f32.const` here (the record-field twin of
                // the if-branch/match-arm bare-ConstFloat grounding); other field shapes emit normally.
                if let Some(crate::ty::Ty::Float(dft)) =
                    field_tys.as_ref().and_then(|m| m.get(name))
                    && dft.ground_width() == 32
                    && let Core::ConstFloat(d) = core_of(db, value)
                {
                    out.push(Lir::F32ConstBits(
                        (f64::from_bits(d.to_f64_bits()) as f32).to_bits(),
                    ));
                } else {
                    emit(db, value, slots, field_base, high, scratch_ty, layout, out)?; // [arr, i, value]
                }
                // A scalar element boxes to a handle (a NARROW int first extends i32→i64, as box-int
                // takes an i64 cell); a nested compound is ALREADY a u32 handle → `arr-set` it directly;
                // a UNIT field pushed nothing → its slot holds the inline-unit sentinel.
                let boxed = match field_tys.as_ref().and_then(|m| m.get(name)) {
                    Some(declared) => box_op_for(db, value, declared)?,
                    None => box_op(db, value)?,
                };
                emit_heap_store_tail(db, value, boxed, out); // [arr, i, handle]
                // Canonicalize a rope-capable String/Bytes field to a flat leaf on construction (see the
                // `Core::Tuple` arm) — a record IS a tuple at run time, so the same nested-rope face.
                if elem_needs_rope_compaction(db, value) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [arr, i, flat-leaf]
                }
                out.push(Lir::CallImport(OP_ARR_SET)); // → [arr]
            }
            Ok(()) // leaves [arr] — the record handle
        }
        // A runtime TUPLE — build it on the value heap: `arr-alloc(n)` leaves the array handle on the
        // stack, then for each element push `(handle, index, boxed-elem)` and `arr-set` (which returns
        // the handle, threading it to the next element). The handle stays on the operand stack across
        // elements — no scratch local — because `arr-set` returns it. Each element is BOXED to a u32
        // handle by its type (`box-int`/`box-bool`); the tuple itself is a u32 handle.
        Core::Tuple { elems } => {
            // §2d STATIC COMPOUNDS: a markable constant tuple in the build-once table is built ONCE (immortal)
            // by the `start` init; read it here with a bare `global.get` instead of the per-eval arr-alloc +
            // boxed arr-set. Else build inline below.
            if try_emit_static_compound(db, id, layout, out) {
                return Ok(());
            }
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            // Box each element by the tuple's OWN solved element type (positional), not the element NODE's
            // type — same fix as `Core::Record`: an at-scale empty-collection element (`Map.empty`) can
            // reach here with a `Ty::Var` node type that `box_op_ty` defaults to `box-int` (i64), wrongly
            // boxing a live i32 handle. `box_op_for` prefers the declared element type, falling back to the
            // node only when the declared is itself unresolved.
            let elem_tys = match crate::infer::type_of(db, id).strip_nominal() {
                crate::ty::Ty::Tuple(ts) => Some(ts.clone()),
                _ => None,
            };
            // Each element starts its scratch ABOVE the high-water the PREVIOUS elements reached, NOT at a
            // fixed `base`. An element that stashes a value in a scratch slot at a given TYPE (a
            // `SumExpect`/match materializing an i32 heap handle) fixes that slot's declared type; a LATER
            // element reusing the same slot number at a DIFFERENT width (`(+ i 1)` → i64) would re-type it,
            // an invalid module (`expected i64, found i32`). Advancing `elem_base` past each element's
            // high-water keeps sibling elements on disjoint slots. (A scalar element leaves `*high` where it
            // was, so this is a no-op for the common all-scalar tuple — byte-identical there.)
            for (i, &elem) in elems.iter().enumerate() {
                let elem_base = *high;
                // [arr] ; push index ; push (box, if scalar) the element ; arr-set → [arr]
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit(db, elem, slots, elem_base, high, scratch_ty, layout, out)?; // [arr, i, elem]
                // A scalar element boxes (a NARROW int extends i32→i64 first, box-int takes i64); a
                // nested compound is ALREADY a u32 handle → `arr-set` it directly, no box; a UNIT element
                // pushed nothing → its slot holds the inline-unit sentinel.
                let boxed = match elem_tys.as_ref().and_then(|ts| ts.get(i)) {
                    Some(declared) => box_op_for(db, elem, declared)?,
                    None => box_op(db, elem)?,
                };
                emit_heap_store_tail(db, elem, boxed, out); // [arr, i, handle]
                // CANONICALIZE a rope-capable String/Bytes element to a flat leaf on construction (the
                // nested-leaf twin of the `op_box_float` normalize-on-construct + the top-level `=`
                // compaction), so the tagless `champ_eq`/`champ_hash` walk compares a nested string by
                // content, not rope-physical bytes. rc-neutral (see `elem_needs_rope_compaction`).
                if elem_needs_rope_compaction(db, elem) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [arr, i, flat-leaf]
                }
                out.push(Lir::CallImport(OP_ARR_SET)); // → [arr]
            }
            Ok(()) // leaves [arr] — the tuple handle
        }
        // A runtime LIST — BULK BUILD: lay the elements into a flat `arr` (exactly as a tuple:
        // `arr-alloc N` + a boxed `arr-set` per element), then ONE `vec-of-arr` to turn that array into
        // the persistent vector. This replaces the old `vec-empty` + N× `vec-push` — N persistent-trie
        // CONSTRUCTORS each consuming+rebuilding the whole vector (O(N) handle allocs) — with a single
        // bulk build (`vec-of-arr` is zero-copy for a ≤32-element list: the arr node IS the trie leaf,
        // reused by move). Each scalar element is BOXED to a u32 handle (a narrow int extended i32→i64
        // first); a nested compound is already a handle. `arr-len 0` yields the empty vector, so `(list)`
        // is `arr-alloc 0` + `vec-of-arr` (no push-chain special case).
        Core::ListNew { elems } => {
            // §2d STATIC (small constant list): a markable constant list (≤32) in the build-once table is
            // built ONCE (immortal) by the `start` init; read it here with a bare `global.get` instead of the
            // per-eval arr-alloc + boxed arr-set + vec-of-arr. Else build inline below.
            if try_emit_static_compound(db, id, layout, out) {
                return Ok(());
            }
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            // Per-element scratch above the running high-water (see `Core::Tuple` — sibling elements of
            // different widths must not share a slot number).
            for (i, &elem) in elems.iter().enumerate() {
                let elem_base = *high;
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit(db, elem, slots, elem_base, high, scratch_ty, layout, out)?; // [arr, i, elem]
                let boxed = box_op(db, elem)?;
                emit_heap_store_tail(db, elem, boxed, out); // [arr, i, handle]
                // Canonicalize a rope-capable String/Bytes element to a flat leaf on construction (see
                // the `Core::Tuple` arm) — a list element nested in a value-eq'd/keyed compound is the
                // same nested-rope face.
                if elem_needs_rope_compaction(db, elem) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [arr, i, flat-leaf]
                }
                out.push(Lir::CallImport(OP_ARR_SET)); // → [arr]
            }
            out.push(Lir::CallImport(OP_VEC_OF_ARR)); // [arr] → [list]
            Ok(()) // leaves [list] — the list handle
        }
        // `List.len` — emit the list handle, then `vec-len` (→ u32 length, an i32 slot). `List.len`'s
        // type is `Int64` (an i64 slot), so EXTEND the i32 length to i64 (unsigned — a length is
        // non-negative). Without this the op left an i32 on the stack where an i64 is expected (the
        // function result, or any enclosing i64 context), so a `List.len` whose list came through a
        // runtime `if` (not folded to a constant) emitted a module that FAILED wasm validation
        // ("expected i64, found i32"). A folded `List.len` over a literal never reaches here (it becomes
        // a `ConstInt`), which is why the constant control validated while the runtime case did not.
        Core::ListLen { operand } => {
            // RECLAMATION (mirror the scalar-element `Core::Proj` reclaim): `vec-len` only BORROWS the list
            // and returns a scalar COUNT, retaining nothing from the sequence. If the operand is a fresh
            // OWNED TEMPORARY (a call result, a constructor — `heap_operand_ownership == Owned`) rather than
            // a BORROW of a live binding (a param/local the owner reclaims), nothing else drops it, so it
            // LEAKS one heap cell per call (`(List.len (build …))`). Stash it in a scratch slot across the
            // borrowing `vec-len`, then `drop` it — the count is already a scalar on the stack. A BORROWED
            // operand is left to its owner (declines to Owned only on a proven-fresh producer, else Borrowed
            // — leak-safe: an unproven ownership just leaves it un-dropped, never double-frees).
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            );
            if reclaim {
                let list_slot = base;
                *high = (*high).max(list_slot + 1);
                scratch_ty.insert(list_slot, ValType::I32);
                emit(db, operand, slots, base + 1, high, scratch_ty, layout, out)?; // [list]
                out.push(Lir::LocalTee(list_slot)); // [list], list_slot = the owned list
                out.push(Lir::CallImport(OP_VEC_LEN)); // → [len:i32] (borrows the list)
                out.push(Lir::LocalGet(list_slot)); // [len, list]
                out.push(Lir::CallImport(OP_DROP)); // → [len] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [len:i64] — List.len : Int64
                return Ok(());
            }
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [list]
            out.push(Lir::CallImport(OP_VEC_LEN)); // → [len:i32]
            out.push(Lir::I64ExtendI32U); // → [len:i64] — List.len : Int64
            Ok(())
        }
        // `Bytes.of` — build the byte sequence on the rope heap. `bytes-alloc(len)` leaves a fresh buffer;
        // then for each element `[buf] ; index ; byte ; bytes-set` — and `bytes-set(buf,index,value) ->
        // buf` RETURNS the buffer (FBIP in-place), so the buffer threads through with no scratch local. A
        // byte is a RAW i32 in `0..=255` (NOT boxed like a list element); every element folded to a
        // constant in range at lowering (`lower_bytes_of`), so each is pushed as an `i32.const`. WARNING: the
        // byte value uses `Lir::ConstI32`, which the serializer writes as a SIGNED LEB — a raw byte ≥ 64
        // would sign-extend negative if hand-emitted, but `Lir::ConstI32` handles the signed encoding, so
        // there is no raw-opcode hazard here (the seed's `sleb128` bug was in hand-written opcode bytes).
        Core::BytesOf { elems } => {
            // §2d STATIC BYTES: a fully-constant literal in the build-once table is materialized ONCE (the
            // `start` init) into an IMMORTAL module global; read it with a bare `global.get` (see
            // `try_emit_static_bytes`). Falls through to the per-eval inline build below when not hoisted.
            if try_emit_static_bytes(db, id, layout, out) {
                return Ok(());
            }
            out.push(Lir::ConstI32(elems.len() as i32)); // [len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            for (i, &elem) in elems.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [buf, index]
                // Push the element's BYTE VALUE (an i32 in 0..=255). A CONSTANT folds to an inline
                // `i32.const`; a RUNTIME element (a `UInt8` param, or `(UInt8.wrap n)`) is emitted — its
                // solved type is a narrow UInt8, so it already lives in an i32 machine slot (no extend,
                // no box: `bytes-set` takes a raw i32 byte). This is what lets the LEB128 encoder's
                // `(Bytes.of (list (UInt8.wrap n)))` build a byte from a runtime value.
                match core_of(db, elem) {
                    Core::ConstInt(v) => {
                        let byte =
                            v.to_i64()
                                .filter(|n| (0..=255).contains(n))
                                .ok_or_else(|| {
                                    Reject::coded(
                                        Code::IntOutOfRange,
                                        "a Bytes.of element is not a UInt8 (0..=255)",
                                    )
                                })? as i32;
                        out.push(Lir::ConstI32(byte)); // [buf, index, byte]
                    }
                    _ => {
                        // A runtime UInt8 — emit its value (an i32 slot); it is in 0..=255 by its type.
                        emit(db, elem, slots, base, high, scratch_ty, layout, out)?; // [buf, index, byte]
                    }
                }
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]  (bytes-set returns the buffer)
            }
            Ok(()) // leaves [buf] — the bytes handle
        }
        // A baked byte-constant — the leaf twin of `BytesOf`. Same `bytes-alloc`+`bytes-set` sequence,
        // reading each byte from the constant slice (already in 0..=255) rather than a child node. Leaves
        // the bytes handle on the stack, byte-identical to a `BytesOf` of the same constant elements.
        Core::ConstBytes(bytes) => {
            // §2d STATIC BYTES: a baked constant is an equal hoist target to a `BytesOf` of constants
            // (`constant_bytes_value` covers both) — route it to its build-once immortal global too.
            if try_emit_static_bytes(db, id, layout, out) {
                return Ok(());
            }
            out.push(Lir::ConstI32(bytes.len() as i32)); // [len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            for (i, &byte) in bytes.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
            }
            Ok(()) // leaves [buf] — the bytes handle
        }
        // A runtime `(bin …)` construction of fixed-width INTEGER segments. Alloc a buffer of the total
        // static width, then per segment: materialize its int value in an i64 scratch slot and write its `w`
        // bytes big-endian (`le` reversed) via `bytes-set` (which returns the buffer, so it threads on the
        // stack). A per-segment range-check is emitted below, but it is a DEFENSIVE BACKSTOP that is normally
        // DEAD: a `(uN v)`/`(iN v)` segment REQUIRES `v` to have the segment's exact width type (infer.rs
        // `seg_value_ty`), so a value that does not fit is a COMPILE-TIME type error (CDZ0203), never a
        // runtime trap — the value reaching here provably fits. Uses TWO scratch slots: `buf` (the byte
        // buffer handle) and `val` (the current segment's i64 value); both above `base`.
        Core::BinBuild { segs } => {
            let total: u32 = segs.iter().map(|s| s.width as u32).sum();
            // The current segment's value lives in an i64 scratch slot (range-checked, then its bytes
            // extracted by shift/mask). The byte buffer handle is THREADED ON THE STACK: `bytes-set`
            // returns the buffer, so each write leaves `[buf]` for the next — exactly like `BytesOf`.
            let val_slot = base;
            *high = (*high).max(base + 1);
            scratch_ty.insert(val_slot, ValType::I64);
            out.push(Lir::ConstI32(total as i32)); // [total]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            let mut offset: u32 = 0;
            for s in &segs {
                let w = s.width as u32;
                let bits = w * 8;
                // Materialize the segment value in the i64 slot (a narrow int emits an i32 → extend by
                // its OWN signedness; an Int64 is already i64). Stack still just `[buf]` after the set.
                // The value sub-expression's transient scratch must FLOAT above the high-water mark, not
                // reuse a fixed `base + 1`: two segments each with a `(g x)` closure application (or any
                // slot-typed temp) would otherwise alias one wasm local at two widths — segment 1 an i32
                // closure cell, segment 2 an i64 arith stash — re-typing it → "expected i64, found i32"
                // (the disjoint-slot discipline `emit_checked_arith`/`emit_call_args` follow for siblings).
                let seg_base = (val_slot + 1).max(*high);
                emit(db, s.value, slots, seg_base, high, scratch_ty, layout, out)?; // [buf, val:i32|i64]
                emit_box_i32_to_i64_extend(db, s.value, out);
                out.push(Lir::LocalSet(val_slot)); // val := value:i64  → [buf]
                // RANGE CHECK (defensive backstop — normally DEAD, since the segment's width type already
                // bounds `val`; a value that does not fit is a compile-time CDZ0203): the value must fit the
                // segment's (signed, bits) width, else trap. Width 8 (an i64 holds every i64) needs no check.
                // Signed: `-(2^(bits-1)) <= val < 2^(bits-1)`; unsigned: `0 <= val < 2^bits`. Emitted as
                // `(low-fail | high-fail) → trap`.
                if bits < 64 {
                    if s.signed {
                        let hi = (1i64 << (bits - 1)) - 1;
                        let lo = -(1i64 << (bits - 1));
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(hi));
                        out.push(Lir::I64GtS); // val > hi
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(lo));
                        out.push(Lir::I64LtS); // val < lo
                        out.push(Lir::I32Or);
                        out.push(Lir::IfUnreachableEnd); // → trap "binary value does not fit segment"
                    } else {
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(0));
                        out.push(Lir::I64LtS); // val < 0
                        out.push(Lir::LocalGet(val_slot));
                        out.push(Lir::ConstI64(1i64 << bits)); // 2^bits
                        out.push(Lir::I64GeS); // val >= 2^bits (val < 2^63 since bits<64, so signed cmp ok)
                        out.push(Lir::I32Or);
                        out.push(Lir::IfUnreachableEnd); // → trap
                    }
                }
                // Write the `w` bytes MSB-first; `le` reverses the buffer position. Each `bytes-set`
                // consumes `[buf, pos, byte]` and returns `[buf]`, threading the buffer.
                for p in 0..w {
                    let shift = (w - 1 - p) * 8;
                    let pos = if s.little_endian {
                        offset + (w - 1 - p)
                    } else {
                        offset + p
                    };
                    // stack is [buf]
                    out.push(Lir::ConstI32(pos as i32)); // [buf, pos]
                    out.push(Lir::LocalGet(val_slot)); // [buf, pos, val:i64]
                    if shift > 0 {
                        out.push(Lir::ConstI64(shift as i64));
                        out.push(Lir::I64ShrU);
                    }
                    out.push(Lir::I32WrapI64);
                    out.push(Lir::ConstI32(0xff));
                    out.push(Lir::I32And); // [buf, pos, byte:i32]
                    out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
                }
                offset += w;
            }
            Ok(()) // leaves [buf] — the bytes handle
        }
        // A RUN of `(bits v k)` bit-fields with a runtime value, packed MSB-first into a fresh `Bytes`. The
        // run is byte-aligned (CDZ0220), so the total byte count + every flush position + the bit-cursor are
        // STATIC (all `k` are compile-time constants) — only the field values are runtime. `acc` (an i64
        // slot) accumulates the open bits MSB-first, flushing whole bytes from its top as they close, exactly
        // like the constant packer in `lower_bin_build`. The byte buffer is THREADED ON THE STACK like
        // `BinBuild`. Each field emits a range-check (`0 <= v < 2^k`) as a DEFENSIVE BACKSTOP that is
        // normally DEAD: a `(bits v k)` field REQUIRES `v : (UInt k)` (infer.rs `seg_value_ty`), so a value
        // that does not fit is a compile-time CDZ0203, not a runtime trap — the value reaching here fits.
        Core::BinBitsBuild { fields } => {
            let total_bits: u32 = fields.iter().map(|f| f.k).sum();
            let total_bytes = total_bits / 8; // byte-aligned (CDZ0220) — exact
            let val_slot = base;
            let acc_slot = base + 1;
            *high = (*high).max(base + 2);
            scratch_ty.insert(val_slot, ValType::I64);
            scratch_ty.insert(acc_slot, ValType::I64);
            out.push(Lir::ConstI32(total_bytes as i32)); // [total]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(acc_slot)); // acc := 0  → [buf]
            let mut nbits: u32 = 0; // open bits since the last byte boundary (STATIC)
            let mut out_pos: u32 = 0; // running byte position in the buffer (STATIC)
            for f in &fields {
                let k = f.k; // 1..=56 (guarded at lower)
                // Materialize the field value in the i64 slot (a narrow int extends by its own signedness).
                emit(db, f.value, slots, base + 2, high, scratch_ty, layout, out)?; // [buf, val:i32|i64]
                emit_box_i32_to_i64_extend(db, f.value, out);
                out.push(Lir::LocalSet(val_slot)); // val := value:i64  → [buf]
                // RANGE CHECK (defensive backstop — normally DEAD, the `(UInt k)` field type already bounds
                // `val`): a k-bit UNSIGNED field, so `0 <= val < 2^k` (k ≤ 56 → 2^k is a positive i64).
                //
                // `ceil = 2^k` and `mask = 2^k - 1` are computed in u128 so a WIDE field cannot overflow the
                // HOST shift computing the constant (mirrors the rust backend's u128 fix, `73efc94ae`):
                // `1i64 << k` is `i64::MIN` at k==63 and a shift-overflow (panic/UB) at k==64. `lower`
                // (`lower.rs`) caps a RUNTIME bit-field at k ≤ 56 (a wider one declines "…wider than 56 bits
                // is not yet built"), so k in 57..=64 is not reachable here TODAY — and for every reachable
                // k ≤ 56 the `as i64` cast is EXACT, so the emitted `ConstI64`s (hence the module bytes) are
                // byte-identical to the old `1i64 << k` form. This purely hardens rcdzc's own constant
                // computation so it stays correct (no host overflow) if that cap ever moves.
                let ceil = (1u128 << k) as i64; // 2^k
                let mask = ((1u128 << k) - 1) as i64; // 2^k - 1
                out.push(Lir::LocalGet(val_slot));
                out.push(Lir::ConstI64(0));
                out.push(Lir::I64LtS); // val < 0
                out.push(Lir::LocalGet(val_slot));
                out.push(Lir::ConstI64(ceil));
                out.push(Lir::I64GeS); // val >= 2^k
                out.push(Lir::I32Or);
                out.push(Lir::IfUnreachableEnd); // → trap "binary value does not fit segment"  → [buf]
                // acc = (acc << k) | (val & ((1<<k)-1))
                out.push(Lir::LocalGet(acc_slot));
                out.push(Lir::ConstI64(k as i64));
                out.push(Lir::I64Shl); // acc << k
                out.push(Lir::LocalGet(val_slot));
                out.push(Lir::ConstI64(mask));
                out.push(Lir::I64And); // val & mask
                out.push(Lir::I64Or);
                out.push(Lir::LocalSet(acc_slot)); // → [buf]
                nbits += k;
                // Flush every whole byte from the TOP of the accumulator (MSB-first), masking the flushed
                // high bits off `acc` after each byte (identical to the constant packer).
                while nbits >= 8 {
                    let shift = nbits - 8;
                    out.push(Lir::ConstI32(out_pos as i32)); // [buf, pos]
                    out.push(Lir::LocalGet(acc_slot));
                    if shift > 0 {
                        out.push(Lir::ConstI64(shift as i64));
                        out.push(Lir::I64ShrU); // acc >> shift
                    }
                    out.push(Lir::I32WrapI64);
                    out.push(Lir::ConstI32(0xff));
                    out.push(Lir::I32And); // [buf, pos, byte:i32]
                    out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
                    out_pos += 1;
                    nbits -= 8;
                    // acc &= (1<<nbits)-1 — drop the just-flushed high bits (nbits==0 → acc := 0).
                    if nbits == 0 {
                        out.push(Lir::ConstI64(0));
                    } else {
                        out.push(Lir::LocalGet(acc_slot));
                        // `2^nbits - 1` in u128 for the same host-overflow safety as the field mask above
                        // (nbits < 8 here after the `-= 8` flushes, so this is always small + `as i64` exact;
                        // computed in u128 purely for uniformity with the ceil/mask hardening).
                        out.push(Lir::ConstI64(((1u128 << nbits) - 1) as i64));
                        out.push(Lir::I64And);
                    }
                    out.push(Lir::LocalSet(acc_slot)); // → [buf]
                }
            }
            debug_assert_eq!(
                nbits, 0,
                "a runtime bit-field run must be byte-aligned (CDZ0220)"
            );
            Ok(()) // leaves [buf] — the bytes handle
        }
        // Read a fixed-width int segment out of a runtime `Bytes` scrutinee (a `bin`-pattern binder). The
        // scrutinee handle is stashed in a slot, then the `w` bytes at `byte_offset` are `bytes-get`'d and
        // assembled into an i64 accumulator MSB-first (`le` reversed), and sign/zero-extended per `signed`.
        // The caller's length probe guarantees the read is in bounds. Result: [value:i64].
        Core::BinIntRead {
            bytes,
            byte_offset,
            off_plus,
            width,
            signed,
            little_endian,
        } => {
            let w = width as u32;
            // The `bytes` operand is the materialized scrutinee (a `LocalRef` — a cheap `local.get`), so
            // it is RE-EMITTED per `bytes-get` rather than stashed in a scratch slot. Claiming a scratch
            // slot here (typed i32) collided with an i64 slot in a nested-if match chain; re-emitting the
            // handle avoids any scratch of our own, so nothing this arm emits can re-type a shared slot.
            // §4a DYNAMIC OFFSET: when `off_plus` is `Some` (a `(bytes body n)` precedes this read), each byte
            // position is `byte_offset + p + off_plus`. `off_plus` is a PURE read (a `BinIntRead`, or a sum of
            // them), so — like `bytes` — it is RE-EMITTED inline per byte rather than stashed in a slot,
            // keeping this arm scratch-free (an off_plus read of a still-static size claims no slot of its
            // own; nesting one inside a nested-if chain is what the no-scratch rule above protects against).
            out.push(Lir::ConstI64(0)); // [acc:i64]
            for p in 0..w {
                let shift = (w - 1 - p) * 8; // MSB-first bit position
                let pos = if little_endian {
                    byte_offset + (w - 1 - p)
                } else {
                    byte_offset + p
                };
                emit(db, bytes, slots, base, high, scratch_ty, layout, out)?; // [acc, bytes]
                out.push(Lir::ConstI32(pos as i32)); // [acc, bytes, static-pos]
                if let Some(op) = off_plus {
                    emit(db, op, slots, base, high, scratch_ty, layout, out)?; // [acc, bytes, static-pos, off:i64]
                    out.push(Lir::I32WrapI64); // [acc, bytes, static-pos, off:i32]
                    out.push(Lir::I32Add); // [acc, bytes, pos]
                }
                out.push(Lir::CallImport(OP_BYTES_GET)); // [acc, byte:i32]
                out.push(Lir::I64ExtendI32U); // [acc, byte:i64] (0..=255)
                if shift > 0 {
                    out.push(Lir::ConstI64(shift as i64));
                    out.push(Lir::I64Shl); // [acc, byte << shift]
                }
                out.push(Lir::I64Or); // [acc']
            }
            // Sign-extend a SIGNED segment narrower than 64 bits from its top bit; an unsigned segment is
            // already zero-extended (each byte was zero-extended). Shift left then arithmetic-shift right.
            if signed && w < 8 {
                let sh = ((8 - w) * 8) as i64; // bits above the value
                out.push(Lir::ConstI64(sh));
                out.push(Lir::I64Shl);
                out.push(Lir::ConstI64(sh));
                out.push(Lir::I64ShrS); // arithmetic → sign-extended
            }
            Ok(()) // leaves [value:i64]
        }
        // A `BinRestRead` binds a FINAL unsized `(bytes rest)` segment: the tail of the scrutinee after
        // the fixed int prefix, as a fresh `Bytes` handle. Emit `bytes-slice(bytes, off, bytes-len - off)`.
        // `bytes` is the materialized scrutinee (a `LocalRef` — a borrow shared by every arm), but
        // `bytes-slice` CONSUMES its source handle, so DUP the scrutinee first (rc++) and slice the copy;
        // the original stays live for the enclosing `let`'s scope-end drop. `off` is a static u32 (the sum
        // of the preceding int widths); the length is `bytes-len - off`, computed at i32 width (both are
        // non-negative and `off <= bytes-len` since the arm's length probe already required `len >= off`).
        Core::BinRestRead {
            bytes,
            byte_offset,
            off_plus,
        } => {
            // dup(handle) pops the handle and rc++'s it, returning nothing — so `tee` it into a scratch
            // slot, dup that copy, then get it back as the slice source. The slot is typed i32 (a handle).
            let handle_slot = base;
            *high = (*high).max(handle_slot + 1);
            scratch_ty.insert(handle_slot, ValType::I32);
            // §4a DYNAMIC OFFSET: `start = byte_offset + off_plus`, so the tail is `bytes-len - start`.
            // Materialize `off_plus` (an i64 count) into an i32 scratch slot once, reused for start + length.
            let (inner_base, off_slot) = match off_plus {
                None => (base + 1, None),
                Some(op) => {
                    let off_slot = base + 1;
                    *high = (*high).max(off_slot + 1);
                    scratch_ty.insert(off_slot, ValType::I32);
                    emit(db, op, slots, base + 2, high, scratch_ty, layout, out)?; // [off:i64]
                    out.push(Lir::I32WrapI64); // [off:i32]
                    out.push(Lir::LocalSet(off_slot)); // off_slot = off
                    (base + 2, Some(off_slot))
                }
            };
            emit(db, bytes, slots, inner_base, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::LocalTee(handle_slot)); // [bytes], slot = bytes
            out.push(Lir::CallImport(OP_DUP)); // pops the copy, rc++ → []
            // Slice source (the retained, rc-incremented handle), then start, then len = bytes-len - start.
            out.push(Lir::LocalGet(handle_slot)); // [bytes] (owned copy for bytes-slice to consume)
            out.push(Lir::ConstI32(byte_offset as i32)); // [bytes, static-off]
            if let Some(off_slot) = off_slot {
                out.push(Lir::LocalGet(off_slot));
                out.push(Lir::I32Add); // [bytes, start]
            }
            out.push(Lir::LocalGet(handle_slot)); // [bytes, start, bytes]
            out.push(Lir::CallImport(OP_BYTES_LEN)); // [bytes, start, len:i32] (borrows)
            out.push(Lir::ConstI32(byte_offset as i32));
            if let Some(off_slot) = off_slot {
                out.push(Lir::LocalGet(off_slot));
                out.push(Lir::I32Add); // [bytes, start, len, start]
            }
            out.push(Lir::I32Sub); // [bytes, start, len - start]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [slice-handle] (consumes the copied bytes)
            Ok(()) // leaves [rest:bytes-handle]
        }
        // A `BinSizedRead` binds a DEPENDENT-SIZE `(bytes payload n)` segment: exactly `n` bytes at a static
        // `byte_offset`, as a fresh `Bytes` handle. Emit `bytes-slice(bytes, off, n)` — the same shape as
        // `BinRestRead` but the length is the RUNTIME `n` (a `BinIntRead` of the earlier size segment, an
        // i64) narrowed to i32, not `bytes-len - off`. `bytes-slice` CONSUMES its source, so DUP the shared
        // scrutinee (rc++) and slice the copy; the original survives the enclosing `let`'s scope-end drop.
        // The arm's length probe already required `bytes-len >= off + n`, so the slice is in bounds.
        Core::BinSizedRead {
            bytes,
            byte_offset,
            off_plus,
            len,
        } => {
            let handle_slot = base;
            *high = (*high).max(handle_slot + 1);
            scratch_ty.insert(handle_slot, ValType::I32);
            // §4a DYNAMIC OFFSET: `start = byte_offset + off_plus`. Materialize `off_plus` (an i64 count)
            // into an i32 scratch slot once (used at the single `start` push). `None` = the static case.
            let (inner_base, off_slot) = match off_plus {
                None => (base + 1, None),
                Some(op) => {
                    let off_slot = base + 1;
                    *high = (*high).max(off_slot + 1);
                    scratch_ty.insert(off_slot, ValType::I32);
                    emit(db, op, slots, base + 2, high, scratch_ty, layout, out)?; // [off:i64]
                    out.push(Lir::I32WrapI64); // [off:i32]
                    out.push(Lir::LocalSet(off_slot)); // off_slot = off
                    (base + 2, Some(off_slot))
                }
            };
            emit(db, bytes, slots, inner_base, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::LocalTee(handle_slot)); // [bytes], slot = bytes
            out.push(Lir::CallImport(OP_DUP)); // pops the copy, rc++ → []
            out.push(Lir::LocalGet(handle_slot)); // [bytes] (owned copy for bytes-slice to consume)
            out.push(Lir::ConstI32(byte_offset as i32)); // [bytes, static-off]
            if let Some(off_slot) = off_slot {
                out.push(Lir::LocalGet(off_slot));
                out.push(Lir::I32Add); // [bytes, start]
            }
            emit(db, len, slots, inner_base, high, scratch_ty, layout, out)?; // [bytes, start, n:i64]
            out.push(Lir::I32WrapI64); // [bytes, start, n:i32] (a byte count fits i32)
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [slice-handle] (consumes the copied bytes)
            Ok(()) // leaves [payload:bytes-handle]
        }
        // `Bytes.len` — emit the bytes handle, then `bytes-len` (→ u32, an i32 slot), then extend to i64
        // (a length is non-negative), since `Bytes.len : Int64`. Mirrors `List.len` exactly.
        Core::BytesLen { operand } => {
            // RECLAMATION (same as `Core::ListLen`): `bytes-len` BORROWS the bytes and returns a scalar
            // count, so an OWNED-TEMPORARY operand must be dropped after the borrow or it leaks a heap cell.
            // A BORROWED param/local is left to its owner (leak-safe: Owned only on a proven-fresh producer).
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            );
            if reclaim {
                let bytes_slot = base;
                *high = (*high).max(bytes_slot + 1);
                scratch_ty.insert(bytes_slot, ValType::I32);
                emit(db, operand, slots, base + 1, high, scratch_ty, layout, out)?; // [bytes]
                out.push(Lir::LocalTee(bytes_slot)); // [bytes], bytes_slot = the owned bytes
                out.push(Lir::CallImport(OP_BYTES_LEN)); // → [len:i32] (borrows the bytes)
                out.push(Lir::LocalGet(bytes_slot)); // [len, bytes]
                out.push(Lir::CallImport(OP_DROP)); // → [len] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [len:i64] — Bytes.len : Int64
                return Ok(());
            }
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::CallImport(OP_BYTES_LEN)); // → [len:i32]
            out.push(Lir::I64ExtendI32U); // → [len:i64] — Bytes.len : Int64
            Ok(())
        }
        // `String.scalar-len` on a RUNTIME string — the number of Unicode SCALARS (codepoints). A String is
        // a flat UTF-8 byte leaf, so WALK the buffer once counting LEAD bytes: a byte begins a new scalar iff
        // `(byte & 0xC0) != 0x80` (not a `10xxxxxx` continuation). The scalar count is exactly the number of
        // lead bytes. Uses only `bytes-len` (loop bound) + `bytes-get` (per-byte read) — the SAME borrowing
        // reads `Core::StrAt`'s scan uses (HASH-NEUTRAL, no new runtime op). A simplification of `StrAt`: no
        // index/skip/span, just one counting pass. Result is a plain `Int64` — no runtime Char involved.
        Core::StrScalarLen { operand } => {
            // RECLAMATION (same as `Core::BytesLen`): the walk BORROWS the string (`bytes-len`/`bytes-get`);
            // an OWNED-temporary operand is dropped after the last borrow, a borrowed param/local is left to
            // its owner. (2) SA1: ALSO drop when `operand` is a SumExpect char-view in the VIEW set — the
            // SumExpect emit dup'd it (rc1 owned) + freed the shell, and THIS (the sole scalar-read consumer)
            // is its liveness last-use, so the post-scan drop reclaims the view (the String twin of
            // reclaim_bytes; VIEW set only — a SHELL-set view is owned by its Call consumer, NOT dropped here).
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) || out.sumexpect_view_reclaim.contains(&operand);
            let str_slot = base;
            let pos_slot = base + 1;
            let bytelen_slot = base + 2;
            let count_slot = base + 3;
            *high = (*high).max(count_slot + 1);
            scratch_ty.insert(str_slot, ValType::I32);
            for s in [pos_slot, bytelen_slot, count_slot] {
                scratch_ty.insert(s, ValType::I64);
            }
            emit(db, operand, slots, base + 4, high, scratch_ty, layout, out)?; // [str]
            out.push(Lir::LocalSet(str_slot));
            // bytelen = bytes-len(str) (borrow), extended to i64.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN));
            out.push(Lir::I64ExtendI32U);
            out.push(Lir::LocalSet(bytelen_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(pos_slot)); // pos = 0
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(count_slot)); // count = 0
            // block { loop { br_out if pos>=bytelen; count += ((bytes-get(str,pos) & 0xC0) != 0x80); pos++;
            // br loop } } — one pass over the bytes, incrementing `count` on each lead byte.
            out.push(Lir::Block(BlockType::Empty)); // $done
            out.push(Lir::Loop(BlockType::Empty)); // $scan
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::I64GeS);
            out.push(Lir::BrIf(1)); // pos >= bytelen → $done
            // count += ((bytes-get(str, pos) & 0xC0) != 0x80) as i64 (a lead byte adds 1, a continuation 0).
            out.push(Lir::LocalGet(count_slot));
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::I32WrapI64);
            out.push(Lir::CallImport(OP_BYTES_GET)); // [count, byte:i32]
            out.push(Lir::ConstI32(0xC0));
            out.push(Lir::I32And);
            out.push(Lir::ConstI32(0x80));
            out.push(Lir::I32Ne); // [count, is_lead:i32]
            out.push(Lir::I64ExtendI32U); // [count, is_lead:i64]
            out.push(Lir::I64Add); // [count']
            out.push(Lir::LocalSet(count_slot));
            // pos++
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::ConstI64(1));
            out.push(Lir::I64Add);
            out.push(Lir::LocalSet(pos_slot));
            out.push(Lir::Br(0)); // → $scan
            out.push(Lir::End); // end $scan
            out.push(Lir::End); // end $done
            if reclaim {
                out.push(Lir::LocalGet(str_slot));
                out.push(Lir::CallImport(OP_DROP)); // reclaim the owned temporary
            }
            out.push(Lir::LocalGet(count_slot)); // [count:i64] — String.scalar-len : Int64
            Ok(())
        }
        // `List.push(l, x)` — emit the list handle, then the element boxed to a u32 handle by its type
        // (a narrow int extended i32→i64 first), then `vec-push` (RETURNS the new list handle). Nested
        // compound elements are already handles (`box_op` → None), pushed directly.
        Core::ListPush { list, elem } => {
            emit(db, list, slots, base, high, scratch_ty, layout, out)?; // [list]
            // The element emits its scratch ABOVE `list`'s high-water so the two operands never share a
            // scratch slot at conflicting valtypes — the disjoint-slot discipline `Core::Tuple`/`Core::
            // ListNew` apply. E.g. `(List.push (rec t) (+ v 1))`: `list` = a recursive-call handle stashed
            // in an i32 dup slot, `elem` = a checked `(+ v 1)` teeing its sum into an i64 overflow-guard
            // slot — a shared `base` would force one slot to two types (validate: "expected i32, found i64").
            emit(db, elem, slots, *high, high, scratch_ty, layout, out)?; // [list, elem]
            let boxed = box_op(db, elem)?;
            emit_heap_store_tail(db, elem, boxed, out); // [list, handle]
            out.push(Lir::CallImport(OP_VEC_PUSH)); // → [list']
            Ok(())
        }
        // `List.prepend(l, x)` — the FRONT-growth twin of `List.push`: identical emission (list handle, then
        // the element boxed by its type above the list's high-water — the same disjoint-slot discipline),
        // differing ONLY in the op called: `vec-prepend` (RETURNS the new list handle). Replaces the old
        // `concat(singleton, l)` lowering, which leaked the superseded front-spine (~17 cells/prepend).
        Core::ListPrepend { list, elem } => {
            emit(db, list, slots, base, high, scratch_ty, layout, out)?; // [list]
            emit(db, elem, slots, *high, high, scratch_ty, layout, out)?; // [list, elem]
            let boxed = box_op(db, elem)?;
            emit_heap_store_tail(db, elem, boxed, out); // [list, handle]
            out.push(Lir::CallImport(OP_VEC_PREPEND)); // → [list']
            Ok(())
        }
        // `List.concat(a, b)` — emit both list handles, then `vec-concat` (→ the joined list handle). The
        // second operand emits ABOVE the first's high-water (disjoint-slot discipline — a recursive-call
        // handle and a checked-arith guard temp must not share a scratch slot at conflicting valtypes).
        Core::ListConcat { lhs, rhs } => {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?; // [a]
            emit(db, rhs, slots, *high, high, scratch_ty, layout, out)?; // [a, b]
            out.push(Lir::CallImport(OP_VEC_CONCAT)); // → [a++b]
            Ok(())
        }
        // `List.update(l, i, x)` — emit the list handle, the index WRAPPED to the `u32` the op takes (the
        // language index is `Int64`, an i64 slot), then the element boxed to a u32 handle by its type (a
        // narrow int extended i32→i64 first, exactly as a push), then `vec-update` (RETURNS the new list
        // handle; an out-of-bounds index traps). Order matches `vec-update(v, index, elem)`.
        Core::ListUpdate { list, index, elem } => {
            emit(db, list, slots, base, high, scratch_ty, layout, out)?; // [list]
            // Each operand emits ABOVE the running high-water so the three never share a scratch slot at
            // conflicting valtypes (disjoint-slot discipline — see `Core::ListPush`). `list` may be a
            // recursive-call handle (i32 dup slot), `index`/`elem` may be checked-arith i64 guard temps.
            let idx_base = *high;
            emit(db, index, slots, idx_base, high, scratch_ty, layout, out)?; // [list, index:i64]
            // HIGH-BITS BOUNDS GUARD before the i64→i32 wrap. `vec-update` takes a u32 index and checks it
            // against the length, but `i32.wrap_i64` discards the high 32 bits FIRST — so a huge index
            // `>= 2^32` that truncates BELOW the length would silently update the wrong slot instead of
            // trapping (an OOB update aliasing a valid element — a safety hole). Trap if the i64 index does
            // not fit u32 (`(index as u64) >= 2^32`); a value in `[0, 2^32)` wraps losslessly and the
            // runtime's own length check catches a real OOB. A NEGATIVE index is a huge u64 (≥ 2^32) so it
            // is caught here too (and is ≥ length regardless). Mirrors the `br_if` wrap-alias guard the
            // scalar `br_table` dispatch emits for an i64 scrutinee, but traps (`IfUnreachableEnd`) rather
            // than routing to a default. The index sub-value is kept in a scratch local across the test —
            // claimed at `idx_base` (above `list`'s scratch) so it can't alias a scratch slot `list`'s emit
            // typed differently.
            // WIDTH-PARTITIONED CLAIM (breaker #23 RESIDUAL): the index sub-value is kept in a scratch local
            // across the bounds test. `idx_base = *high` is normally free, BUT a floor-reset across a fold's
            // handler arms can leave `*high` pointing back at a slot a LIVE i32 handle still holds — e.g. a
            // `SumExpect` shell (a fold-forced `Option.expect`, breaker pfxH func[9] slot 6) recorded that slot
            // I32 and raised `*high`, then a sibling-arm reset dropped `*high` back onto it. Blindly teeing the
            // i64 index there (`scratch_ty.insert(idx_slot, I64)`, last-writer-wins) re-declares that wasm
            // local i64 → the earlier i32 handle `tee` is invalid ("expected i32, found i64"), invalid
            // component. SKIP forward over any slot `scratch_ty` records at a NON-i64 width, the same
            // width-partition discipline `Core::ListAt`/`Core::Let`/`emit_checked_arith_to` already apply.
            let mut idx_slot = idx_base;
            while matches!(scratch_ty.get(&idx_slot), Some(&w) if w != ValType::I64) {
                idx_slot += 1;
            }
            *high = (*high).max(idx_slot + 1);
            scratch_ty.insert(idx_slot, ValType::I64);
            out.push(Lir::LocalTee(idx_slot)); // [list, index] — keep a copy in the slot
            out.push(Lir::ConstI64(0x1_0000_0000)); // 2^32
            out.push(Lir::I64GeU); // [list, (index as u64) >= 2^32]
            out.push(Lir::IfUnreachableEnd); // out of u32 range → trap (index out of bounds)
            out.push(Lir::LocalGet(idx_slot)); // [list, index:i64]
            out.push(Lir::I32WrapI64); // [list, index:i32] — now known to fit u32
            emit(db, elem, slots, *high, high, scratch_ty, layout, out)?; // [list, index, elem] — above index scratch
            let boxed = box_op(db, elem)?;
            emit_heap_store_tail(db, elem, boxed, out); // [list, index, handle]
            out.push(Lir::CallImport(OP_VEC_UPDATE)); // → [list']
            Ok(())
        }
        // A runtime SUM construction — `(Option.Some 5)` or a nullary `None`. Build the PAYLOAD handle,
        // then `sum-new(disc, payload)`. The payload is (`value-heap-runtime.md` §Sum):
        //  - NULLARY (no payloads) → the unit value, an empty array `arr-alloc(0)`;
        //  - SINGLE payload → the value boxed to a handle (`box-int`/`box-bool`; a compound payload is
        //    already a handle, no box) — a NARROW int extends i32→i64 first, as `box-int` takes an i64;
        //  - MULTIPLE payloads → a tuple handle built exactly as `Core::Tuple` (`arr-alloc(n)` + per-
        //    payload box + `arr-set`).
        // Leaves the sum's u32 handle on the stack.
        Core::SumNew { disc, payloads } => {
            // BUILD-ONCE-IMMORTAL a nullary MIXED-sum terminal (`(Z)`/`(Nil)`): if this node was collected as
            // a static compound (`is_markable_constant_sum_nullary`), read the immortal handle from its module
            // global instead of a fresh `sum-new` — the rsl1 leak-1 fix (the fresh per-construction node was
            // the leaked terminal on recursive-sum walks). Keyed by node id, so this only routes the collected
            // nullary-terminal nodes; every other `SumNew` falls through to the fresh build below.
            if try_emit_static_compound(db, id, layout, out) {
                return Ok(());
            }
            // The variant's DECLARED payload type(s) at this sum's instantiation — box each payload by THAT,
            // not the payload-value NODE's type. Same fix as `Core::Record`/`Core::Tuple`: an at-scale
            // empty-collection payload (`(Some Map.empty)`, or a sum whose payload is a Map/sum) can reach
            // here with a `Ty::Var` node type that `box_op_ty` defaults to `box-int` (i64), wrongly boxing a
            // live i32 handle → "expected i64 found i32 @func27". For a SINGLE payload the declared type IS
            // the payload type; for MULTIPLE it is a `Ty::Tuple` whose element `i` is payload `i`'s type.
            let sum_ty = crate::infer::type_of(db, id);
            let payload_decl = variant_payload_ty_at(db, &sum_ty, disc);
            // ENUM-DISCRIMINANT sum (every variant nullary, ≥2 variants): the value IS its discriminant,
            // so construction is JUST the constant — no `sum-new` box, no unit payload. The i32 slot holds
            // the discriminant directly; a match switches on it and equality is `i32.eq` (see those sites).
            if node_is_enum_disc(db, id) {
                out.push(Lir::ConstI32(disc as i32));
                return Ok(());
            }
            out.push(Lir::ConstI32(disc as i32)); // [disc]
            match payloads.len() {
                0 => {
                    // Unit payload: the inline-unit handle. `arr-alloc(0)` RETURNS exactly this constant
                    // (a compile-time-known handle, no heap node), so push it directly rather than
                    // emitting an `arr-alloc(0)` CALL — one const instead of `const 0 ; call arr-alloc`,
                    // and the nullary construction imports no runtime op for its payload. `IMM_UNIT` is
                    // DERIVED from the runtime's `cdz-abi` section by codegen (never hand-coded).
                    out.push(Lir::ConstI32(super::super::runtime_abi::IMM_UNIT as i32)); // [disc, unit]
                }
                1 => {
                    let p = payloads[0];
                    // A UNIT-TYPED single payload — a single-variant nullary sum (`(type E EX)`) erases to
                    // `Ty::Nominal { inner: Unit }`, so a `(Result A E)`'s `Err` carries an `E` payload whose
                    // value is `Core::Unit` (it EMITS NOTHING — `valtype_of(Unit) = None`). This is the SAME
                    // shape as the 0-payload case above (a genuinely nullary variant): the payload handle is
                    // the inline-unit sentinel `IMM_UNIT`, which `emit_heap_store_tail` substitutes when the
                    // value pushed nothing. A SCALAR boxes; a compound payload is already a handle. Without
                    // this the payload was absent and `sum-new` underflowed the stack (invalid wasm).
                    emit(db, p, slots, base, high, scratch_ty, layout, out)?; // [disc, value | nothing]
                    let boxed = match payload_decl.as_ref() {
                        Some(declared) => box_op_for(db, p, declared)?,
                        None => box_op(db, p)?,
                    };
                    emit_heap_store_tail(db, p, boxed, out); // [disc, payload-handle]
                    // Canonicalize a rope-capable String/Bytes payload to a flat leaf on construction (see
                    // the `Core::Tuple` arm) — a rope in a sum payload (e.g. `(Some (concat …))`) is the
                    // sum-payload face of the nested-rope value-eq/key miss.
                    if elem_needs_rope_compaction(db, p) {
                        out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [disc, flat-leaf]
                    }
                }
                n => {
                    // Multiple payloads: build a tuple `arr` and box each into its position. A UNIT payload
                    // occupies its slot with the inline-unit sentinel (`emit_heap_store_tail`), keeping the
                    // positional indices aligned — this is the `(A Int64 Unit)` shape whose Unit slot pushed
                    // NOTHING before, underflowing the per-payload `arr-set` into invalid wasm.
                    out.push(Lir::ConstI32(n as i32)); // [disc, n]
                    out.push(Lir::CallImport(OP_ARR_ALLOC)); // [disc, arr]
                    // A multi-payload variant's declared type is a `Ty::Tuple` — element `i` is payload `i`'s
                    // declared type (box each by that, falling back to the node when the tuple/element is
                    // unresolved), the multi-payload twin of the single-payload declared-type box above.
                    let payload_elem_tys = match payload_decl.as_ref().map(|t| t.strip_nominal()) {
                        Some(crate::ty::Ty::Tuple(ts)) => Some(ts.clone()),
                        _ => None,
                    };
                    // Each payload sub-emit starts its scratch ABOVE the running high-water
                    // (`payload_base = *high`), so sibling payloads never SHARE a scratch slot — the same
                    // discipline `emit_loop_iteration` applies to sibling tail-call args. A wasm local has ONE
                    // type: a later payload's i32 handle temp (e.g. a recursive-call payload's Perceus `dup`
                    // temp) reusing an earlier payload's i64 arith-overflow-guard slot (`(ICons (+ h 1) (rec
                    // t))` — the checked `(+ h 1)` tees its sum into an i64 guard slot; `(rec t)` tees its
                    // handle into a dup slot) would force one slot to two types and the module fails
                    // validation ("expected i32, found i64"). Advancing to `*high` hands each payload
                    // fresh, never-typed scratch slots. (Was a fixed `base` for all payloads — the sibling
                    // scratch-slot i32/i64 collision bug.)
                    let mut payload_base = base;
                    for (i, &p) in payloads.iter().enumerate() {
                        out.push(Lir::ConstI32(i as i32)); // [disc, arr, i]
                        emit(db, p, slots, payload_base, high, scratch_ty, layout, out)?; // [disc, arr, i, value]
                        payload_base = *high;
                        let boxed = match payload_elem_tys.as_ref().and_then(|ts| ts.get(i)) {
                            Some(declared) => box_op_for(db, p, declared)?,
                            None => box_op(db, p)?,
                        };
                        emit_heap_store_tail(db, p, boxed, out); // [disc, arr, i, handle]
                        // Canonicalize a rope-capable payload element on construction (see above).
                        if elem_needs_rope_compaction(db, p) {
                            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [disc, arr, i, flat-leaf]
                        }
                        out.push(Lir::CallImport(OP_ARR_SET)); // [disc, arr]
                    }
                }
            }
            out.push(Lir::CallImport(OP_SUM_NEW)); // → [sum-handle]
            Ok(())
        }
        // A RUNTIME `List.at(list, index)` — the bounds-checked fallible read. Evaluate the list handle
        // and the Int64 index ONCE into scratch slots (each is read more than once: the list for
        // `vec-len` AND `vec-get`; the index for the low/high bounds tests AND `vec-get`). Then test
        // `0 <= index < vec-len(list)` (an i64 compare — the u32 length is extended to i64, and the
        // signed index's low side catches a NEGATIVE index without it wrapping to a huge unsigned
        // offset), and emit `if in-bounds (Some <element>) else None`:
        //  - IN BOUNDS: `vec-get(list, wrap(index))` yields the element handle BORROWED (rc unchanged);
        //    `dup` it (the `Some` payload CONSUMES its handle, but the list still owns the element —
        //    value-heap-runtime.md §Constructors Consume And Accessors Borrow), then `sum-new(disc_some,
        //    handle)`. The element stays BOXED (a downstream match unboxes it), so no box/unbox here.
        //  - OUT OF BOUNDS: `sum-new(disc_none, arr-alloc(0))` — the nullary `None` (unit payload).
        // Leaves the Option's u32 handle on the stack.
        Core::ListAt {
            list,
            index,
            disc_some,
            disc_none,
        } => {
            // HANDLE SLOT REUSE: the list handle is read TWICE (the bounds-check `vec-len` and the in-bounds
            // `vec-get`), both BORROWING (no refcount change, never consume). When the list is a reusable
            // handle already resident in a stable slot (a param / kept `let`-local), read THAT slot directly
            // at each use instead of copying it into a fresh scratch slot — the heap analogue of the scalar
            // operand-slot reuse. A computed list is stashed in scratch once, as before.
            let reuse_list = reusable_handle_slot(db, list, slots);
            // RECLAMATION: `vec-len`/`vec-get` only BORROW the list; the read element is `dup`'d into the
            // `Some` payload (so it survives independently). If the list operand is a fresh OWNED TEMPORARY
            // (`List.at (build …) i` — not a reused param/kept-local slot), nothing else drops it → it LEAKS.
            // Drop `list_slot` after the if/else (the Option result is on top; `drop` pops only the list).
            // A reused-slot (param/kept-local) list is BORROWED — its owner reclaims; never drop it here.
            let reclaim_list = reuse_list.is_none()
                && matches!(heap_operand_ownership(db, list), Ok(HandleOwnership::Owned));
            // Scratch above `base`: the list handle (i32, only when NOT reusing an owner slot), the index
            // (i64), and — in the in-bounds arm — the borrowed element handle (i32). Reusing the list slot
            // frees one scratch slot (the index/elem shift down), shrinking the high-water. The operand
            // recursions float above all claimed scratch so they never clobber a live slot.
            let (list_slot, index_slot, elem_slot, floor) = match reuse_list {
                Some(s) => (s, base, base + 1, base + 2),
                None => (base, base + 1, base + 2, base + 3),
            };
            *high = (*high).max(elem_slot + 1);
            if reuse_list.is_none() {
                scratch_ty.insert(list_slot, ValType::I32);
            }
            scratch_ty.insert(index_slot, ValType::I64);
            scratch_ty.insert(elem_slot, ValType::I32);
            if reuse_list.is_none() {
                emit(db, list, slots, floor, high, scratch_ty, layout, out)?; // [list]
                out.push(Lir::LocalSet(list_slot));
            }
            // Float the index operand's scratch floor ABOVE the running high-water (breaker #23, the INVERSE
            // face of #18 — the SAME seam the sibling `Core::BytesAt` already fixes). When the `list` operand
            // is a LIVE handle threaded across handler dispatches (a `List.push`-grown next-state `pre`, an
            // i32 vec handle whose emit — or an enclosing dispatch's — raised `*high` and typed intervening
            // slots i32), a COMPUTED index (`(- (List.len pre) 1)` = i64 checked-arith) reset to the STALE
            // `floor` reuses one of those i32-typed slots → one wasm local declared at two widths → "expected
            // i32, found i64", invalid component (breaker #23 pfxmin5: 3 `add` dispatches through a
            // computed-index `List.at`+`List.push` arm, func[13] @0x4fd; the 3rd dispatch pushes allocation
            // onto the live handle slot). Advancing to `floor.max(*high)` hands the index fresh slots.
            // Harmless when nothing above `floor` was claimed (`*high == floor`).
            let index_floor = floor.max(*high);
            emit(db, index, slots, index_floor, high, scratch_ty, layout, out)?; // [index:i64]
            out.push(Lir::LocalSet(index_slot));
            // in_bounds = (index >= 0) & (index < len), all in i64. Each half is INDEPENDENTLY elidable when
            // provably true:
            //   • LOWER-BOUND ELISION (`index >= 0`): a NON-NEGATIVE index (a masked/length/unsigned/refined
            //     value) — a masked index (`(& i 15)`), a `List.len`, a loop counter refined `≥ 0`.
            //   • UPPER-BOUND ELISION (`index < len`, operator-greenlit BOUNDS facet): an index flow-known
            //     `< len(this list)` — an enclosing `(< i (List.len xs))` guard proved it, so `List.at xs i`
            //     inside that branch double-checks. The fact is KEYED ON COLLECTION IDENTITY: a guard on a
            //     DIFFERENT list does not elide (that would be OOB).
            // When BOTH halves are proven, `in_bounds` is a compile-time `true`, so the whole runtime test —
            // AND the `vec-len` call, the `if`, and the `None` arm — is dropped: the access is unconditional.
            let index_nonneg = crate::lower::value_provably_nonneg(db, index);
            let index_below_len = crate::lower::index_provably_below_len(db, index, list);
            // The Some(element) arm — shared by the conditional path and the both-proven unconditional path.
            // `vec-get` yields the element handle BORROWED; `dup` retains it (rc++) so the `Some` payload can
            // own a reference while the list keeps its own. `dup(handle)` RETURNS NOTHING (it pops the handle
            // and increments its count), so the handle is stashed in a scratch slot: `tee` (store + keep a
            // copy for `dup`), `dup` (consume that copy, rc++), then `get` it back as the payload under
            // `disc_some` for `sum-new`.
            let emit_some = |out: &mut Vec<Lir>| {
                out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
                out.push(Lir::LocalGet(list_slot));
                out.push(Lir::LocalGet(index_slot));
                out.push(Lir::I32WrapI64); // [disc_some, list, index:i32] — vec-get takes a u32
                out.push(Lir::CallImport(OP_VEC_GET)); // [disc_some, elem-handle] (borrowed)
                out.push(Lir::LocalTee(elem_slot)); // [disc_some, elem], elem_slot = elem
                out.push(Lir::CallImport(OP_DUP)); // pops elem, rc++ → [disc_some]
                out.push(Lir::LocalGet(elem_slot)); // [disc_some, elem] (the retained handle)
                out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            };
            if index_nonneg && index_below_len {
                // BOTH bounds proven — the index is unconditionally in range, so emit Some(element) directly
                // with no bounds test, no `vec-len`, no `if`, and no `None` arm. The list is still read twice
                // (get borrows it), so the owned-temporary reclaim below is unchanged.
                emit_some(out);
            } else {
                if !index_nonneg {
                    out.push(Lir::LocalGet(index_slot));
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GeS); // [index >= 0]
                }
                if !index_below_len {
                    out.push(Lir::LocalGet(index_slot));
                    out.push(Lir::LocalGet(list_slot));
                    out.push(Lir::CallImport(OP_VEC_LEN)); // [.., index, len:i32]
                    out.push(Lir::I64ExtendI32U); // [.., index, len:i64]
                    out.push(Lir::I64LtS); // [(index >= 0,) index < len]
                }
                if !index_nonneg && !index_below_len {
                    out.push(Lir::I32And); // [in_bounds] — both halves survived, combine them
                }
                out.push(Lir::If(BlockType::Val(ValType::I32)));
                emit_some(out);
                out.push(Lir::Else);
                // ELSE — None: the unit payload is an empty array.
                emit_none_option(disc_none, out); // [None-handle]
                out.push(Lir::End);
            }
            if reclaim_list {
                // [Option] — drop the owned-temporary list now that both borrows (len + get) are done.
                out.push(Lir::LocalGet(list_slot));
                out.push(Lir::CallImport(OP_DROP)); // → [Option] (list reclaimed)
            }
            Ok(())
        }
        // A MAP construction — `(map …)` or `Map.empty`. `map-empty` leaves a fresh empty map; then for
        // each entry, box the key and value by their types (a narrow int extended i32→i64 first) and
        // `map-insert(map, key, val)` — which CONSUMES the map handle + key + value and RETURNS the new
        // map, threading the handle through with no scratch local (like `bytes-set`). Entries insert in
        // SOURCE order, so a later duplicate key overwrites (keys compared by value). Leaves the map handle.
        Core::MapNew {
            entries,
            key_ty,
            val_ty,
        } => {
            // §2d STATIC: a markable constant map in the build-once table is built ONCE (immortal, deep-marked)
            // by the `start` init; read it here with a bare `global.get`. Else build inline below.
            if try_emit_static_compound(db, id, layout, out) {
                return Ok(());
            }
            out.push(Lir::CallImport(OP_MAP_EMPTY)); // → [map]
            for &(k, v) in entries.iter() {
                // Each key/value sub-expression starts its scratch ABOVE the running high-water, NOT at a
                // fixed `base` — the same disjoint-slot discipline `Core::Tuple`/`Core::ListNew` apply, so a
                // key/value that stashes an i32 handle in a scratch slot never collides with a sibling's i64
                // arith temp at that slot number (one wasm local at two widths = an invalid module).
                let key_base = base.max(*high);
                emit(db, k, slots, key_base, high, scratch_ty, layout, out)?; // [map, key]
                let key_boxed = box_op_for(db, k, &key_ty)?;
                emit_heap_store_tail(db, k, key_boxed, out); // [map, key-handle]
                if key_needs_compaction(db, k) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
                }
                if key_needs_canonicalize(db, k) {
                    // list-typed/-containing key → canonical RRB shape (champ slot exactness)
                    emit_key_canonicalize(db, k, &key_ty, high, scratch_ty, out)?; // [map, canon-key]
                }
                let val_base = base.max(*high);
                emit(db, v, slots, val_base, high, scratch_ty, layout, out)?; // [map, key, val]
                let val_boxed = box_op_for(db, v, &val_ty)?;
                emit_heap_store_tail(db, v, val_boxed, out); // [map, key, val-handle]
                out.push(Lir::CallImport(OP_MAP_INSERT)); // → [map'] (consumes map, key, val)
            }
            Ok(()) // leaves [map] — the map handle
        }
        // `Map.insert(m, k, v)` — emit the map handle, the key boxed by its type, the value boxed by its
        // type, then `map-insert` (RETURNS the new map handle; consumes all three). Mirrors `MapNew`'s
        // per-entry insert.
        Core::MapInsert {
            map,
            key,
            val,
            key_ty,
            val_ty,
        } => {
            // Each of map/key/val starts its scratch ABOVE the running high-water, NOT at a fixed `base`
            // — the disjoint-slot discipline `Core::Tuple`/`Core::ListNew` apply. `map`'s prior insert
            // may have stashed an i32 handle in a slot; `key`/`val` carrying guarded i64 arith
            // (`(BigInt.of (+ n 2))`) must not reuse that slot number at a different width (one wasm local
            // at two widths = an invalid module: `expected i64, found i32`).
            let map_base = base.max(*high);
            emit(db, map, slots, map_base, high, scratch_ty, layout, out)?; // [map]
            let key_base = base.max(*high);
            emit(db, key, slots, key_base, high, scratch_ty, layout, out)?; // [map, key]
            let key_boxed = box_op_for(db, key, &key_ty)?;
            emit_heap_store_tail(db, key, key_boxed, out); // [map, key-handle]
            if key_needs_compaction(db, key) {
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf (champ contract)
            }
            if key_needs_canonicalize(db, key) {
                // list-typed/-containing key → canonical RRB shape (champ slot exactness)
                emit_key_canonicalize(db, key, &key_ty, high, scratch_ty, out)?; // [map, canon-key]
            }
            let val_base = base.max(*high);
            emit(db, val, slots, val_base, high, scratch_ty, layout, out)?; // [map, key, val]
            let val_boxed = box_op_for(db, val, &val_ty)?;
            emit_heap_store_tail(db, val, val_boxed, out); // [map, key, val-handle]
            out.push(Lir::CallImport(OP_MAP_INSERT)); // → [map']
            Ok(())
        }
        // `Map.merge(a, b)` — emit both map handles, then `map-merge` (→ the unioned map handle; CONSUMES
        // both, LAST-WRITER/b-wins on an overlapping key). No key/val boxing — the operands' entries are
        // already boxed inside the maps. The second operand emits ABOVE the first's high-water (disjoint-
        // slot discipline), exactly as `Core::ListConcat` threads its two heap handles.
        Core::MapMerge { lhs, rhs } => {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?; // [a]
            emit(db, rhs, slots, *high, high, scratch_ty, layout, out)?; // [a, b]
            out.push(Lir::CallImport(OP_MAP_MERGE)); // → [a ∪ b, b-wins]
            Ok(())
        }
        // `Map.remove(m, k)` — emit the map handle, the key boxed by its type, then `map-remove` (RETURNS
        // the new map; consumes the map, BORROWS the key). Removing an absent key yields a map equal to the
        // operand (total). The op only reads the key (via hash/eq) and drops the map's OWN stored columns,
        // never the passed-in key — so an OWNED-TEMPORARY key (boxed scalar / compacted rope / const-String
        // leaf) must be `drop`ped by the emit after the borrow, exactly like `MapLookup`. Scratch: the key
        // handle (i32), teed before the op and dropped after when owned.
        Core::MapRemove { map, key, key_ty } => {
            let key_slot = base;
            *high = (*high).max(key_slot + 1);
            scratch_ty.insert(key_slot, ValType::I32);
            // `key` floats its scratch ABOVE `map`'s high-water (disjoint-slot discipline, as
            // `Core::MapInsert`/`Core::Tuple`): `map` may be a recursive-call handle stashed in an i32
            // `dup` slot, `key` a checked `(+ v 1)` teeing into an i64 overflow-guard slot — a shared
            // fixed `base + 1` would re-type one wasm local to two widths (invalid module,
            // `expected i32, found i64`). `key_slot` (= `base`, the owned-drop tee) is below both.
            emit(db, map, slots, base + 1, high, scratch_ty, layout, out)?; // [map]
            let key_base = (base + 1).max(*high);
            emit(db, key, slots, key_base, high, scratch_ty, layout, out)?; // [map, key]
            let key_boxed = box_op_for(db, key, &key_ty)?;
            emit_heap_store_tail(db, key, key_boxed, out); // [map, key-handle]
            if key_needs_compaction(db, key) {
                // Compact BEFORE the tee so key_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
            }
            if key_needs_canonicalize(db, key) {
                // Canonicalize BEFORE the tee so key_slot holds the fresh owned canonical key the drop reclaims.
                emit_key_canonicalize(db, key, &key_ty, high, scratch_ty, out)?; // [map, canon-key]
            }
            // OWNERSHIP GATE (mirrors `MapLookup`): `map-remove` BORROWS the key, so drop it AFTER only when
            // it is an OWNED TEMPORARY. A BORROWED String/compound key (param / kept-local / a live
            // sum-payload projection) is left to its owner — dropping it would free a live reference.
            let key_owned = key_handle_is_owned_temporary(db, key, &key_ty)?;
            out.push(Lir::LocalTee(key_slot)); // [map, key], key_slot = key (for the later drop)
            out.push(Lir::CallImport(OP_MAP_REMOVE)); // → [map'] (consumes map, borrows key)
            if key_owned {
                // Drop the owned key temporary now that the borrow-remove is done.
                out.push(Lir::LocalGet(key_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(())
        }
        // `Map.size(m)` — emit the map handle, `map-size` (→ u32, an i32 slot), then extend to i64 (a
        // count is non-negative), since `Map.size : Int64`. Mirrors `List.len`/`Bytes.len`.
        Core::MapSize { map } => {
            // RECLAMATION (same as `Core::ListLen`/`Core::BytesLen`): `map-size` BORROWS the map and returns
            // a scalar count, so an OWNED-TEMPORARY operand (`Map.len (build …)`) must be dropped after the
            // borrow or it leaks a heap cell. A BORROWED param/local is left to its owner (Owned only on a
            // proven-fresh producer, else Borrowed — leak-safe). ALSO reclaim an OWNED nested-compound
            // `Core::Proj` child (`Map.len (. (mk i) a)`): `heap_operand_ownership(Proj)` is (deliberately)
            // Borrowed, but the `Proj` emit DUP'd the extracted child into a standalone owned handle, which
            // this borrowing read must then drop (drop-iff-dup'd — see `owned_proj_child_dupd`). Fixes the
            // Map.len-over-a-projected-fresh-record disjoint-slot leak (corpus-05 #4547).
            let reclaim = matches!(heap_operand_ownership(db, map), Ok(HandleOwnership::Owned))
                || owned_proj_child_dupd(db, map, slots);
            if reclaim {
                let map_slot = base;
                *high = (*high).max(map_slot + 1);
                scratch_ty.insert(map_slot, ValType::I32);
                emit(db, map, slots, base + 1, high, scratch_ty, layout, out)?; // [map]
                out.push(Lir::LocalTee(map_slot)); // [map], map_slot = the owned map
                out.push(Lir::CallImport(OP_MAP_SIZE)); // → [size:i32] (borrows the map)
                out.push(Lir::LocalGet(map_slot)); // [size, map]
                out.push(Lir::CallImport(OP_DROP)); // → [size] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [size:i64] — Map.size : Int64
                return Ok(());
            }
            emit(db, map, slots, base, high, scratch_ty, layout, out)?; // [map]
            out.push(Lir::CallImport(OP_MAP_SIZE)); // → [size:i32]
            out.push(Lir::I64ExtendI32U); // → [size:i64] — Map.size : Int64
            Ok(())
        }
        // A SET construction — `(Set.of (list …))`. `set-empty` leaves a fresh empty set; then for each
        // element, box it by its type (a narrow int extended i32→i64 first) and `set-insert(set, elem)` —
        // which CONSUMES the set + element and RETURNS the new set, threading the handle through (like a
        // map insert). A duplicate element is a no-op at insert (the set dedups). Leaves the set handle.
        Core::SetOf { elems, elem_ty } => {
            // §2d STATIC: a markable constant set in the build-once table is built ONCE (immortal, deep-marked)
            // by the `start` init; read it here with a bare `global.get`. Else build inline below.
            if try_emit_static_compound(db, id, layout, out) {
                return Ok(());
            }
            out.push(Lir::CallImport(OP_SET_EMPTY)); // → [set]
            for &e in elems.iter() {
                // Each element starts its scratch ABOVE the running high-water, NOT at a fixed `base` —
                // the same disjoint-slot discipline `Core::Tuple`/`Core::Record`/`Core::ListNew` apply.
                // An element that stashes a transient in a scratch slot at a given TYPE (a BigInt arith's
                // i32 handle scratch) fixes that slot's declared type; a LATER element reusing the same
                // slot number at a DIFFERENT width (`(BigInt.of (+ n 2))`'s i64 overflow-guard temp) would
                // re-type one wasm local to two widths → an invalid module (`expected i64, found i32`).
                // Advancing `elem_base` past each element's high-water keeps siblings on disjoint slots.
                let elem_base = base.max(*high);
                emit(db, e, slots, elem_base, high, scratch_ty, layout, out)?; // [set, elem]
                let elem_boxed = box_op_for(db, e, &elem_ty)?;
                emit_heap_store_tail(db, e, elem_boxed, out); // [set, elem-handle]
                if key_needs_compaction(db, e) {
                    out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
                }
                if key_needs_canonicalize(db, e) {
                    // list-typed/-containing element → canonical RRB shape (champ slot exactness)
                    emit_key_canonicalize(db, e, &elem_ty, high, scratch_ty, out)?; // [set, canon-elem]
                }
                out.push(Lir::CallImport(OP_SET_INSERT)); // → [set'] (consumes set, elem)
            }
            Ok(()) // leaves [set] — the set handle
        }
        // `Set.insert(s, e)` — emit the set handle, the element boxed by its type, then `set-insert`
        // (RETURNS the new set; consumes both). Mirrors `MapInsert` without the value column.
        Core::SetInsert { set, elem, elem_ty } => {
            // `set` and `elem` start their scratch ABOVE the running high-water (disjoint-slot discipline,
            // as `Core::Tuple`/`Core::ListNew`): `set`'s prior inserts may have typed a scratch slot i32,
            // and `elem`'s guarded i64 arith must not reuse it at a different width (invalid module).
            let set_base = base.max(*high);
            emit(db, set, slots, set_base, high, scratch_ty, layout, out)?; // [set]
            let elem_base = base.max(*high);
            emit(db, elem, slots, elem_base, high, scratch_ty, layout, out)?; // [set, elem]
            let elem_boxed = box_op_for(db, elem, &elem_ty)?;
            emit_heap_store_tail(db, elem, elem_boxed, out); // [set, elem-handle]
            if key_needs_compaction(db, elem) {
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
            }
            if key_needs_canonicalize(db, elem) {
                emit_key_canonicalize(db, elem, &elem_ty, high, scratch_ty, out)?; // [set, canon-elem]
            }
            out.push(Lir::CallImport(OP_SET_INSERT)); // → [set']
            Ok(())
        }
        // `Set.remove(s, e)` — emit the set handle, the element boxed by its type, then `set-remove`
        // (RETURNS the new set; consumes the set, BORROWS the element). Removing an absent element yields an
        // equal set (total). Like `map-remove`, the op only reads the element and drops the set's OWN stored
        // columns, never the passed-in element — so an OWNED-TEMPORARY element must be `drop`ped by the emit
        // after the borrow, exactly like `SetContains`. Scratch: the element handle (i32).
        Core::SetRemove { set, elem, elem_ty } => {
            let elem_slot = base;
            *high = (*high).max(elem_slot + 1);
            scratch_ty.insert(elem_slot, ValType::I32);
            // `elem` floats its scratch ABOVE `set`'s high-water (disjoint-slot discipline, as
            // `Core::SetInsert`/`Core::Tuple`): `set` may be a recursive-call handle in an i32 `dup`
            // slot, `elem` a checked `(+ v 1)` teeing into an i64 overflow-guard slot — a shared fixed
            // `base + 1` would re-type one wasm local to two widths (invalid module, `expected i32,
            // found i64`). `elem_slot` (= `base`, the owned-drop tee) is below both.
            emit(db, set, slots, base + 1, high, scratch_ty, layout, out)?; // [set]
            let elem_base = (base + 1).max(*high);
            emit(db, elem, slots, elem_base, high, scratch_ty, layout, out)?; // [set, elem]
            let elem_boxed = box_op_for(db, elem, &elem_ty)?;
            emit_heap_store_tail(db, elem, elem_boxed, out); // [set, elem-handle]
            if key_needs_compaction(db, elem) {
                // Compact BEFORE the tee so elem_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
            }
            if key_needs_canonicalize(db, elem) {
                // Canonicalize BEFORE the tee so elem_slot holds the fresh owned canonical elem the drop reclaims.
                emit_key_canonicalize(db, elem, &elem_ty, high, scratch_ty, out)?; // [set, canon-elem]
            }
            // OWNERSHIP GATE (mirrors `SetContains`): `set-remove` BORROWS the element, so drop it AFTER only
            // when it is an OWNED TEMPORARY. A BORROWED element (param / kept-local / a live sum-payload
            // projection) is left to its owner — dropping it would free a live reference.
            let elem_owned = key_handle_is_owned_temporary(db, elem, &elem_ty)?;
            out.push(Lir::LocalTee(elem_slot)); // [set, elem], elem_slot = elem (for the later drop)
            out.push(Lir::CallImport(OP_SET_REMOVE)); // → [set'] (consumes set, borrows elem)
            if elem_owned {
                // Drop the owned element temporary now that the borrow-remove is done.
                out.push(Lir::LocalGet(elem_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(())
        }
        // `Set.len(s)` — emit the set handle, `set-size` (→ u32), extend to i64 (`Set.len : Int64`).
        Core::SetLen { set } => {
            // RECLAMATION (same as `Core::MapSize`/`Core::ListLen`): `set-size` BORROWS the set, so an
            // OWNED-TEMPORARY operand must be dropped after the borrow or it leaks a heap cell. A borrowed
            // param/local is left to its owner.
            let reclaim = matches!(heap_operand_ownership(db, set), Ok(HandleOwnership::Owned));
            if reclaim {
                let set_slot = base;
                *high = (*high).max(set_slot + 1);
                scratch_ty.insert(set_slot, ValType::I32);
                emit(db, set, slots, base + 1, high, scratch_ty, layout, out)?; // [set]
                out.push(Lir::LocalTee(set_slot)); // [set], set_slot = the owned set
                out.push(Lir::CallImport(OP_SET_SIZE)); // → [size:i32] (borrows the set)
                out.push(Lir::LocalGet(set_slot)); // [size, set]
                out.push(Lir::CallImport(OP_DROP)); // → [size] (reclaim the owned temporary)
                out.push(Lir::I64ExtendI32U); // → [size:i64]
                return Ok(());
            }
            emit(db, set, slots, base, high, scratch_ty, layout, out)?; // [set]
            out.push(Lir::CallImport(OP_SET_SIZE)); // → [size:i32]
            out.push(Lir::I64ExtendI32U); // → [size:i64]
            Ok(())
        }
        // `Set.to-list(s)` — enumerate the set's elements as a `List` in canonical element order. Emit the
        // set handle, then build the compiler-baked element-shape descriptor as a constant `Bytes` handle
        // inline (`bytes-alloc(n)` then a `bytes-set` per descriptor byte — `bytes-set` returns the buffer,
        // so the desc handle threads on the stack), then `set-to-list(s, desc)` (BORROWS both; the runtime
        // reads the desc to order by element value and returns a fresh owned `List`). The descriptor is a
        // `Set`-rooted shape (NOT the value-form `Framed` wrapper), which is what `op_set_to_list` resolves.
        Core::SetToList { set, elem_ty } => {
            let Some(desc) = crate::lower::set_shape_descriptor(db, &elem_ty) else {
                // Float-scope the no-descriptor reject: a FLOAT leaf makes the element un-orderable by a
                // PERMANENT no-total-order carve-out (a floating-point type offers only the IEEE partial
                // order — a NaN is unordered), so it is a coded CDZ0203 that points at the relational ops —
                // the Set.to-list face of the float three-way-compare reject. A Set/Map leaf (no blessed
                // order at all) stays a codeless decline (a separate not-yet/carve-out class).
                if crate::lower::type_has_float_leaf(db, &elem_ty, &mut Vec::new()) {
                    return Err(Reject::coded(
                        crate::diag::Code::TypeMismatch,
                        "Set.to-list enumerates elements in a total order, but this element type has a \
                         floating-point leaf, which offers only the IEEE partial order (a not-a-number is \
                         unordered) — it has no total order; compare its orderable components with the \
                         relational operators `<`, `<=`, `>`, `>=` instead",
                    ));
                }
                // An UNDETERMINED element type (a free `Var`) is a DETERMINACY fault, not an unorderable-
                // feature gap — code it CDZ0203 ("annotate the type") like the Map.to-list / key-
                // canonicalize twins, rather than the codeless backstop (the Set face of the same
                // reachable-codeless class, v-cdz-smith seed 902902902). A determined-but-unorderable
                // shape (no free var) keeps the codeless decline.
                if elem_ty.has_free_var() {
                    return Err(Reject::coded(
                        crate::diag::Code::TypeMismatch,
                        format!(
                            "a Set element's type `{}` is not fully determined — annotate it \
                             (e.g. `(: (list) (List Int64))`) so its elements have a canonical form for \
                             comparison",
                            crate::ty::Ty::Set(Box::new(elem_ty.clone()))
                                .render_name(&db.name_ctx())
                        ),
                    ));
                }
                return Err(Reject::decline(
                    "Set.to-list element shape has no orderable descriptor",
                ));
            };
            // The baked descriptor `Bytes` is an OWNED TEMPORARY that `set-to-list` only BORROWS (the
            // runtime reads it as an inspector; see `op_set_to_list` — "BORROWS `s` and `desc`"). So it must
            // be dropped after the op, or every `Set.to-list` call LEAKS the descriptor heap cell. Stash its
            // handle in a scratch slot across the (set, desc)-consuming op call, then drop it.
            // The SOURCE `set` is ALSO only BORROWED by `set-to-list` — so when it is an OWNED TEMPORARY (a
            // fresh computed value with no owner slot, e.g. `Set.to-list (Set.of …)`), nothing else reclaims
            // it and the source set LEAKS one handle per call. Stash it too and drop it after the op when
            // Owned (a Param/kept-local set is `Borrowed` → left to its owner, never dropped here — mirrors
            // `SetContains`/`SetLen`). Slots: desc at `base`, the owned source at `base+1`; operands float
            // above both. The op leaves `[list]` on the stack; the two drops pop only the stashed handles.
            let set_owned = matches!(heap_operand_ownership(db, set), Ok(HandleOwnership::Owned));
            let desc_slot = base;
            let set_slot = base + 1;
            *high = (*high).max(set_slot + 1);
            scratch_ty.insert(desc_slot, ValType::I32);
            if set_owned {
                scratch_ty.insert(set_slot, ValType::I32);
            }
            emit(db, set, slots, base + 2, high, scratch_ty, layout, out)?; // [set]
            if set_owned {
                out.push(Lir::LocalTee(set_slot)); // [set], set_slot = the owned source (for the later drop)
            }
            out.push(Lir::ConstI32(desc.len() as i32)); // [set, len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [set, desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32)); // [set, buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [set, buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [set, buf] (bytes-set returns the buffer)
            }
            out.push(Lir::LocalTee(desc_slot)); // [set, desc], desc_slot = desc (for the later drop)
            out.push(Lir::CallImport(OP_SET_TO_LIST)); // [set, desc] → [list] (borrows both)
            out.push(Lir::LocalGet(desc_slot)); // [list, desc]
            out.push(Lir::CallImport(OP_DROP)); // → [list] (drop the borrowed-only descriptor Bytes)
            if set_owned {
                out.push(Lir::LocalGet(set_slot)); // [list, set]
                out.push(Lir::CallImport(OP_DROP)); // → [list] (drop the borrowed owned-temporary source set)
            }
            Ok(()) // leaves [list]
        }
        // `Map.to-list(m)` — the map companion: emit the map, bake a MAP-rooted key/value shape descriptor
        // inline, then `map-to-list(m, desc)` → a `List (Tuple k v)` in canonical KEY order.
        Core::MapToList {
            map,
            key_ty,
            val_ty,
        } => {
            let Some(desc) = crate::lower::map_shape_descriptor(db, &key_ty, &val_ty) else {
                // An UNDETERMINED key/value type (a free `Var` — an unconstrained `Result` Err arm, an
                // empty-collection element, …) has no shape, so `map_shape_descriptor` returns `None`.
                // That is a DETERMINACY fault, not an unorderable-feature gap: code it CDZ0203
                // ("annotate the type"), the twin of the `emit_key_canonicalize` key-determinacy reject
                // and the Int-key path's CDZ0203 — so a Float32-key map (whose descriptor requires the
                // value shape, unlike an Int-key map, which tolerates an undetermined value) no longer
                // slips to the CODELESS backstop for the SAME undetermined-`Result`-Err cause the Int
                // path already codes (v-cdz-smith seed 902902902). A genuinely-unorderable DETERMINED
                // shape (no free var) keeps the codeless decline — the not-yet / carve-out class.
                if key_ty.has_free_var() || val_ty.has_free_var() {
                    return Err(Reject::coded(
                        crate::diag::Code::TypeMismatch,
                        format!(
                            "a Set/Map key's type `{}` is not fully determined — annotate it \
                             (e.g. `(: (list) (List Int64))`) so its keys have a canonical form for \
                             comparison",
                            crate::ty::Ty::Map(Box::new(key_ty.clone()), Box::new(val_ty.clone()))
                                .render_name(&db.name_ctx())
                        ),
                    ));
                }
                return Err(Reject::decline(
                    "Map.to-list key/value shape has no orderable descriptor",
                ));
            };
            // As in `Set.to-list`: the baked descriptor `Bytes` is an owned temporary `map-to-list` only
            // BORROWS (`op_map_to_list` — "BORROWS `m` and `desc`"), so it must be dropped after the op or
            // every `Map.to-list` call leaks the descriptor heap cell. Stash + drop across the op call.
            // The SOURCE `map` is ALSO only BORROWED — so when it is an OWNED TEMPORARY (`Map.to-list
            // (Map.insert …)`, the enumerate→transform→fold idiom with no binding keeping the map live),
            // nothing reclaims it and the source map LEAKS per call. Stash + drop it after the op when Owned
            // (a Param/kept-local map is `Borrowed` → left to its owner). Slots: desc at `base`, owned source
            // at `base+1`; operands float above both.
            let map_owned = matches!(heap_operand_ownership(db, map), Ok(HandleOwnership::Owned));
            let desc_slot = base;
            let map_slot = base + 1;
            *high = (*high).max(map_slot + 1);
            scratch_ty.insert(desc_slot, ValType::I32);
            if map_owned {
                scratch_ty.insert(map_slot, ValType::I32);
            }
            emit(db, map, slots, base + 2, high, scratch_ty, layout, out)?; // [map]
            if map_owned {
                out.push(Lir::LocalTee(map_slot)); // [map], map_slot = the owned source (for the later drop)
            }
            out.push(Lir::ConstI32(desc.len() as i32)); // [map, len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [map, desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32)); // [map, buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [map, buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [map, buf]
            }
            out.push(Lir::LocalTee(desc_slot)); // [map, desc], desc_slot = desc (for the later drop)
            out.push(Lir::CallImport(OP_MAP_TO_LIST)); // [map, desc] → [list] (borrows both)
            out.push(Lir::LocalGet(desc_slot)); // [list, desc]
            out.push(Lir::CallImport(OP_DROP)); // → [list] (drop the borrowed-only descriptor Bytes)
            if map_owned {
                out.push(Lir::LocalGet(map_slot)); // [list, map]
                out.push(Lir::CallImport(OP_DROP)); // → [list] (drop the borrowed owned-temporary source map)
            }
            Ok(()) // leaves [list]
        }
        // A runtime `Set.contains(s, e)` — the TOTAL membership predicate. Box the element, `set-contains(s,
        // key)` (BORROWS both; returns a `bool` directly — UNLIKE `Map.lookup`'s NULL-or-handle → Option).
        // The boxed element is an owned temporary the emit must `drop` after the borrow — stash it in a
        // scratch slot, box, contains, then drop the stashed element. Leaves the bool on the stack.
        Core::SetContains { set, elem, elem_ty } => {
            let set_slot = base;
            let elem_slot = base + 1;
            *high = (*high).max(elem_slot + 1);
            scratch_ty.insert(set_slot, ValType::I32);
            scratch_ty.insert(elem_slot, ValType::I32);
            // `set-contains` BORROWS BOTH the set and the element and returns a bool (nothing borrows out of
            // the set). Reclaim EACH iff an OWNED TEMPORARY: the SET when it is a fresh computed value
            // (`Set.contains (build …) x` — else it leaks, the collection twin of the Set.len owned-temp
            // reclaim), the ELEMENT via the existing `key_handle_is_owned_temporary` gate. A borrowed
            // param/kept-local set/element is left to its owner.
            let set_owned = matches!(heap_operand_ownership(db, set), Ok(HandleOwnership::Owned));
            emit(db, set, slots, base + 2, high, scratch_ty, layout, out)?; // [set]
            if set_owned {
                out.push(Lir::LocalTee(set_slot)); // [set], set_slot = the owned set (for the later drop)
            }
            emit(db, elem, slots, base + 2, high, scratch_ty, layout, out)?; // [set, elem]
            let elem_boxed = box_op_for(db, elem, &elem_ty)?;
            emit_heap_store_tail(db, elem, elem_boxed, out); // [set, elem-handle]
            if key_needs_compaction(db, elem) {
                // Compact BEFORE the tee so elem_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope element → canonical flat leaf
            }
            if key_needs_canonicalize(db, elem) {
                // Canonicalize BEFORE the tee so elem_slot holds the fresh owned canonical elem the drop reclaims.
                emit_key_canonicalize(db, elem, &elem_ty, high, scratch_ty, out)?; // [set, canon-elem]
            }
            // OWNERSHIP GATE (mirrors `MapLookup`): `set-contains` BORROWS the element, so drop it AFTER
            // only when it is an OWNED TEMPORARY (a boxed scalar, a compacted rope, or a fresh owned
            // compound). A BORROWED String/compound element — a param / kept-local / a live sum-payload or
            // element projection — is used as-is; dropping it would free a reference its owner still holds.
            let elem_owned = key_handle_is_owned_temporary(db, elem, &elem_ty)?;
            out.push(Lir::LocalTee(elem_slot)); // [set, elem], elem_slot = elem (for the later drop)
            out.push(Lir::CallImport(OP_SET_CONTAINS)); // [bool] (borrows set + elem)
            if elem_owned {
                // Drop the owned element temporary now that the borrow-contains is done.
                out.push(Lir::LocalGet(elem_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            if set_owned {
                // Drop the owned-temporary SET (set-contains only borrowed it; the bool result is a scalar).
                out.push(Lir::LocalGet(set_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(()) // leaves [bool]
        }
        // A set-algebra op — emit both operand set handles, then the matching runtime op (consumes both,
        // returns the result set). `Set.union`/`intersection`/`difference` share this shape.
        Core::SetAlgebra { op, lhs, rhs } => {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?; // [a]
            emit(db, rhs, slots, base, high, scratch_ty, layout, out)?; // [a, b]
            out.push(Lir::CallImport(match op {
                crate::core::SetAlgebraOp::Union => OP_SET_UNION,
                crate::core::SetAlgebraOp::Intersection => OP_SET_INTERSECTION,
                crate::core::SetAlgebraOp::Difference => OP_SET_DIFFERENCE,
            })); // → [result]
            Ok(())
        }
        // A runtime `Map.lookup(m, k)` — the fallible keyed read. Box the key, `map-lookup(m, key)` (BORROWS
        // both; returns the STORED VALUE HANDLE, or NULL when the key is absent). If the returned handle is
        // non-null build `Some(value)` — the value is a boxed handle used DIRECTLY as the `Some` payload,
        // `dup`'d so the map keeps its own reference (mirrors `ListAt`'s borrowed `vec-get`) — else `None`.
        // The boxed key is an owned temporary `drop`ped after the borrow. Scratch: the key handle (i32,
        // dropped after lookup) and the looked-up value handle (i32).
        Core::MapLookup {
            map,
            key,
            key_ty,
            disc_some,
            disc_none,
            ..
        } => {
            let key_slot = base;
            let val_slot = base + 1;
            let map_slot = base + 2;
            *high = (*high).max(map_slot + 1);
            scratch_ty.insert(key_slot, ValType::I32);
            scratch_ty.insert(val_slot, ValType::I32);
            scratch_ty.insert(map_slot, ValType::I32);
            // `map-lookup` BORROWS the map; if the map is an OWNED TEMPORARY (`Map.lookup (build …) k` — not
            // a reused param/kept-local) it must be reclaimed or it LEAKS. WARNING: DELICATE ORDERING: the looked-up
            // VALUE is stored borrowed in `val_slot` and `dup`'d in the Some arm — so the map's drop must come
            // AFTER that dup (THEN), and in the None arm (val is NULL, nothing to preserve). Dropping right
            // after `map-lookup` would free the value `val_slot` still borrows → a UAF. Stash the map now.
            let map_owned = matches!(heap_operand_ownership(db, map), Ok(HandleOwnership::Owned));
            emit(db, map, slots, base + 3, high, scratch_ty, layout, out)?; // [map]
            if map_owned {
                out.push(Lir::LocalTee(map_slot)); // [map], map_slot = the owned map (for the arm drops)
            }
            emit(db, key, slots, base + 3, high, scratch_ty, layout, out)?; // [map, key]
            let key_boxed = box_op_for(db, key, &key_ty)?;
            emit_heap_store_tail(db, key, key_boxed, out); // [map, key-handle]
            if key_needs_compaction(db, key) {
                // Compact BEFORE the tee so key_slot holds the owned flat leaf the later drop reclaims.
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope key → canonical flat leaf
            }
            if key_needs_canonicalize(db, key) {
                // Canonicalize BEFORE the tee so key_slot holds the fresh owned canonical key the drop reclaims.
                emit_key_canonicalize(db, key, &key_ty, high, scratch_ty, out)?; // [map, canon-key]
            }
            // OWNERSHIP GATE: `map-lookup` BORROWS the key (never consumes it), so we drop it AFTER only
            // when it is an OWNED TEMPORARY (a boxed scalar, a compacted rope, or a fresh owned compound).
            // A BORROWED String/compound key — a param / kept-local / a `sum-payload`/`arr-get` projection
            // of a still-live value — is used as-is; dropping it would free a reference its owner still
            // holds (a use-after-free). This is the two-live-matched-String-payloads miscompile: a
            // tree-walker looking up a node's OWN key AND its child's key (both live sum-payload String
            // projections) had the second borrowed key freed, flipping its comparison (a silent wrong
            // count). See `key_handle_is_owned_temporary`.
            let key_owned = key_handle_is_owned_temporary(db, key, &key_ty)?;
            out.push(Lir::LocalTee(key_slot)); // [map, key], key_slot = key (for the later drop)
            out.push(Lir::CallImport(OP_MAP_LOOKUP)); // [value-or-null] (borrows map + key)
            out.push(Lir::LocalSet(val_slot)); // val_slot = value-or-null, stack empty
            if key_owned {
                // Drop the owned key temporary now that the borrow-lookup is done (map-lookup borrows it).
                out.push(Lir::LocalGet(key_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            // present = (value != NULL).
            out.push(Lir::LocalGet(val_slot));
            out.push(Lir::ConstI32(NULL_HANDLE));
            out.push(Lir::I32Ne); // [present]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(value). The stored value handle is BORROWED (the map still owns it); `dup` it so
            // the `Some` payload owns its own reference, then use it as the payload under `disc_some`.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(val_slot)); // [disc_some, value]
            out.push(Lir::CallImport(OP_DUP)); // pops value, rc++ → [disc_some]
            out.push(Lir::LocalGet(val_slot)); // [disc_some, value] (retained)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            if map_owned {
                // The value is now independently retained (dup'd above), so it is SAFE to drop the
                // owned-temporary map here — AFTER the dup, not right after `map-lookup` (that would free
                // the value the `val_slot` borrow points at → UAF). `drop` pops only the map. [Some-handle]
                out.push(Lir::LocalGet(map_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            out.push(Lir::Else);
            // ELSE — None: the unit payload is an empty array.
            emit_none_option(disc_none, out); // [None-handle]
            if map_owned {
                // None arm: `val_slot` is NULL (no value borrowed out), so the owned-temporary map is dropped
                // here with nothing to preserve. [None-handle]
                out.push(Lir::LocalGet(map_slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            out.push(Lir::End);
            Ok(())
        }
        // A runtime `Bytes.at` — the fallible byte read. Bounds-check `0 <= index < bytes-len`, then in
        // bounds build `Some(box-int(bytes-get))`, else `None`. Unlike `ListAt`, `bytes-get` returns a
        // RAW byte VALUE (i32 `0..=255`), so there is no borrowed handle to `dup`: zero-extend it to i64
        // and `box-int` it into an `Int64` for the `Some` payload. Two scratch slots (bytes handle i32,
        // index i64) above `base`; operand recursions float above them.
        Core::BytesAt {
            bytes,
            index,
            disc_some,
            disc_none,
        } => {
            // HANDLE SLOT REUSE (see `ListAt`): the bytes handle is read twice (bounds-check `bytes-len` +
            // the in-bounds `bytes-get`), both BORROWING. A reusable handle already in a stable slot is read
            // directly; a computed one is stashed in scratch once.
            let reuse_bytes = reusable_handle_slot(db, bytes, slots);
            // RECLAMATION (see `ListAt`): an OWNED-temporary bytes operand (`Bytes.at (build …) i`, not a
            // reused param/kept-local) is dropped after the borrowing len/get — the read byte is a COPIED
            // i32 value (nothing borrows from the bytes), so the sequence can be freed. A borrowed
            // param/kept-local is left to its owner.
            // (2) rope/slice-view: ALSO drop when `bytes` is a SumExpect-extracted view marked for reclaim
            // (`sumexpect_view_reclaim`) — the SumExpect emit dup'd it (rc1, owned) + freed the shell, and
            // THIS (the sole consuming Bytes.at, single-consumer-gated) is its liveness last-use, so the
            // post-borrow drop reclaims the view. DEDICATED set (not dup_sites) → this fires the view-drop
            // ONLY for the (2) reclaim, never conflated with a mark_binder_dups/B1 dup on the same node.
            let reclaim_bytes = reuse_bytes.is_none()
                && (matches!(
                    heap_operand_ownership(db, bytes),
                    Ok(HandleOwnership::Owned)
                ) || out.sumexpect_view_reclaim.contains(&bytes));
            let (bytes_slot, index_slot, floor) = match reuse_bytes {
                Some(s) => (s, base, base + 1),
                None => (base, base + 1, base + 2),
            };
            *high = (*high).max(index_slot + 1);
            if reuse_bytes.is_none() {
                scratch_ty.insert(bytes_slot, ValType::I32);
            }
            scratch_ty.insert(index_slot, ValType::I64);
            if reuse_bytes.is_none() {
                emit(db, bytes, slots, floor, high, scratch_ty, layout, out)?; // [bytes]
                out.push(Lir::LocalSet(bytes_slot));
            }
            // Float the index operand's scratch floor ABOVE the running high-water (breaker #18, WIDENED
            // face): when the bytes operand is a `String.to-bytes` rope VIEW over a multi-value-upgraded
            // effect-state thread, its emit spends transient scratch at `floor` and types those slots i32
            // (the rope handle); a computed INDEX (i64 arith) reset to the SAME `floor` then reuses an
            // i32-typed slot → "expected i64, found i32", invalid component — identical seam to the
            // `Core::StrAt` fix. Advancing to `floor.max(*high)` hands the index fresh slots. Harmless when
            // the bytes operand was reused (no bytes emit, `*high` unchanged) or spent no scratch.
            let index_floor = floor.max(*high);
            emit(db, index, slots, index_floor, high, scratch_ty, layout, out)?; // [index:i64]
            out.push(Lir::LocalSet(index_slot));
            // in_bounds = (index >= 0) & (index < len), all in i64. LOWER-BOUND ELISION (see `ListAt`): a
            // provably NON-NEGATIVE index (a masked/length/unsigned/refined value) makes `index >= 0` a
            // compile-time `true`, so drop it and test only `index < len`.
            let index_nonneg = crate::lower::value_provably_nonneg(db, index);
            if !index_nonneg {
                out.push(Lir::LocalGet(index_slot));
                out.push(Lir::ConstI64(0));
                out.push(Lir::I64GeS); // [index >= 0]
            }
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN)); // [.., index, len:i32]
            out.push(Lir::I64ExtendI32U); // [.., index, len:i64]
            out.push(Lir::I64LtS); // [(index >= 0,) index < len]
            if !index_nonneg {
                out.push(Lir::I32And); // [in_bounds]
            }
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(box-int(bytes-get(bytes, index))). `bytes-get` yields the byte as an i32 VALUE
            // (0..=255); zero-extend to i64 and box it into an `Int64` handle for the `Some` payload.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I32WrapI64); // [disc_some, bytes, index:i32] — bytes-get takes a u32
            out.push(Lir::CallImport(OP_BYTES_GET)); // [disc_some, byte:i32] (raw value 0..=255)
            out.push(Lir::I64ExtendI32U); // [disc_some, byte:i64] (a byte is non-negative)
            out.push(Lir::CallImport(OP_BOX_INT)); // [disc_some, Int64-handle]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None: the unit payload is an empty array.
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            if reclaim_bytes {
                // [Option] — drop the owned-temporary bytes now that both borrows (len + get) are done.
                out.push(Lir::LocalGet(bytes_slot));
                out.push(Lir::CallImport(OP_DROP)); // → [Option] (bytes reclaimed)
            }
            Ok(())
        }
        // `String.at(str, index)` on a RUNTIME string — read the i-th UNICODE SCALAR as a one-scalar
        // String, fallibly. A String is a flat UTF-8 byte leaf, so WALK the byte buffer: a byte is a
        // scalar START iff `(byte & 0xC0) != 0x80` (not a `10xxxxxx` continuation byte). Phase 1 skips
        // `index` scalar starts, leaving `pos` at the target scalar's first byte; phase 2 measures that
        // scalar's byte span (lead byte + its continuation bytes) and `bytes-slice`s it into `Some`. A
        // negative index or one at/beyond the scalar count → `None`. The string handle is BORROWED for the
        // scan (`bytes-len`/`bytes-get`) and CONSUMED by the final `bytes-slice`; the None branch drops it.
        // `String.scalar-at` on a RUNTIME string → the `index`-th Unicode scalar as `(Option Char)`. The
        // runtime op `bytes-scalar-at(buf, scalar-index) -> u32` does the UTF-8 walk (unlike `StrAt`, which
        // walks the buffer in wasm), returning the codepoint or `u32::MAX` for out-of-range / ill-formed. Box
        // the codepoint into a Char scalar cell for `Some` (#5252: zero-extend i32→i64, `box-int`); `u32::MAX
        // → None`. The string is BORROWED (its owner reclaims — like `StrAt`), so it is not dropped here.
        Core::StrScalarAt {
            operand,
            index,
            disc_some,
            disc_none,
        } => {
            let str_slot = base;
            let index_slot = base + 1;
            let cp_slot = base + 2;
            *high = (*high).max(cp_slot + 1);
            scratch_ty.insert(str_slot, ValType::I32);
            scratch_ty.insert(index_slot, ValType::I64);
            scratch_ty.insert(cp_slot, ValType::I32);
            // `bytes-scalar-at` only BORROWS the string (reads the buffer), so if the operand is a fresh
            // OWNED TEMPORARY (`String.scalar-at (String.concat …) i`, or an inline constant), nothing else
            // reclaims it → it LEAKS; drop it after the read. A BORROWED operand (a param / kept-local) is
            // reclaimed by its owner — never dropped here (mirrors `Core::ListAt`'s `reclaim_list`).
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            );
            emit(db, operand, slots, base + 3, high, scratch_ty, layout, out)?; // [str]
            out.push(Lir::LocalSet(str_slot));
            // Float the index operand's scratch floor above the running high-water — the i32/i64 slot-width
            // collision guard `StrAt`/`ListAt`/`BytesAt` apply (a computed i64 index must not reuse a slot the
            // string emit typed i32; harmless when nothing above `base + 3` was claimed).
            let index_floor = (base + 3).max(*high);
            emit(db, index, slots, index_floor, high, scratch_ty, layout, out)?; // [index:i64]
            out.push(Lir::LocalSet(index_slot));
            // codepoint = bytes-scalar-at(str, wrap(index)) — the runtime does the OOR/ill-formed check.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I32WrapI64); // scalar-index crosses as a u32
            out.push(Lir::CallImport(OP_BYTES_SCALAR_AT)); // [codepoint:i32]
            out.push(Lir::LocalSet(cp_slot));
            // in_range = codepoint != u32::MAX (0xFFFFFFFF).
            out.push(Lir::LocalGet(cp_slot));
            out.push(Lir::ConstI32(-1)); // 0xFFFFFFFF
            out.push(Lir::I32Ne); // [codepoint != u32::MAX]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // Some(char): box the codepoint into a Char scalar cell (zero-extend into the box-int i64 cell).
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(cp_slot));
            out.push(Lir::I64ExtendI32U);
            out.push(Lir::CallImport(OP_BOX_INT)); // [disc_some, boxed-char]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            if reclaim {
                // [Option] — drop the owned-temporary string now the borrow-read is done (recursively frees
                // its cells → the census balances). `drop` pops the string; the Option result stays on top.
                out.push(Lir::LocalGet(str_slot));
                out.push(Lir::CallImport(OP_DROP)); // → [Option] (string reclaimed)
            }
            Ok(())
        }
        Core::StrAt {
            string,
            index,
            disc_some,
            disc_none,
        } => {
            let str_slot = base;
            let index_slot = base + 1;
            let pos_slot = base + 2;
            let scalar_slot = base + 3;
            let bytelen_slot = base + 4;
            let spanstart_slot = base + 5;
            *high = (*high).max(spanstart_slot + 1);
            scratch_ty.insert(str_slot, ValType::I32);
            for s in [
                index_slot,
                pos_slot,
                scalar_slot,
                bytelen_slot,
                spanstart_slot,
            ] {
                scratch_ty.insert(s, ValType::I64);
            }
            emit(db, string, slots, base + 6, high, scratch_ty, layout, out)?; // [str]
            out.push(Lir::LocalSet(str_slot));
            // Float the index operand's scratch floor ABOVE the running high-water (not a fixed `base + 6`),
            // exactly as `String.slice` does for its start/end operands. A wasm local's type is fixed
            // function-wide, so if the STRING emit spent transient scratch at `base + 6` (e.g. its handle
            // traces to a multi-value-upgraded effect-state thread, whose owned-reclaim tees an i32 handle
            // into a scratch slot) and the computed INDEX (`(- n 1)`, i64 arith) then reset to the SAME
            // `base + 6`, the later i64 store reuses a slot already typed i32 → "expected i32, found i64",
            // module invalid (the `function[18]` invalid-wasm, breaker #18; sibling of the `String.slice`
            // width-disjoint-slot fix). A literal / bare-param index spends no scratch so never collided,
            // which is why only a COMPUTED index over an effect-grown string tripped it.
            let index_base = (base + 6).max(*high);
            emit(db, index, slots, index_base, high, scratch_ty, layout, out)?; // [index:i64]
            out.push(Lir::LocalSet(index_slot));
            // byte-count (i64), read repeatedly below.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN));
            out.push(Lir::I64ExtendI32U);
            out.push(Lir::LocalSet(bytelen_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(pos_slot)); // pos = 0
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(scalar_slot)); // scalar = 0
            // A byte at `pos` (already known `pos < bytelen`) begins a NEW scalar iff it is NOT a `10xxxxxx`
            // continuation byte: `(bytes-get(str, pos) & 0xC0) != 0x80`. Helper closure emitting that test
            // as an i32 bool onto the stack (borrows `str`).
            let push_is_lead = |out: &mut Emit| {
                out.push(Lir::LocalGet(str_slot));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::I32WrapI64);
                out.push(Lir::CallImport(OP_BYTES_GET)); // [byte:i32]
                out.push(Lir::ConstI32(0xC0));
                out.push(Lir::I32And);
                out.push(Lir::ConstI32(0x80));
                out.push(Lir::I32Ne); // [(byte & 0xC0) != 0x80]
            };
            // "Advance `pos` past ONE whole scalar": `pos++`, then while `pos < bytelen` and the byte at
            // `pos` is a CONTINUATION byte, `pos++`. Emitted as `block { pos++; loop { br_out if pos>=len;
            // br_out if is_lead(pos); pos++; br loop } }`. Precondition: `pos < bytelen` (a scalar starts
            // here). Used in both phases.
            let emit_skip_one_scalar = |out: &mut Emit, push_is_lead: &dyn Fn(&mut Emit)| {
                // pos++ past the lead byte.
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Block(BlockType::Empty)); // $cont_done
                out.push(Lir::Loop(BlockType::Empty)); // $cont
                // pos >= bytelen → done.
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(bytelen_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // → $cont_done
                // byte at pos is a LEAD byte (new scalar) → done.
                push_is_lead(out);
                out.push(Lir::BrIf(1)); // → $cont_done
                // else a continuation byte: pos++, loop.
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Br(0)); // → $cont
                out.push(Lir::End); // end $cont
                out.push(Lir::End); // end $cont_done
            };
            // PHASE 1 — skip `index` scalar starts (only if index >= 0; a negative index leaves scalar=0 <
            // index false, so found is computed false below). `block { loop { br_out if scalar>=index;
            // br_out if pos>=bytelen; skip_one_scalar; scalar++; br loop } }`.
            out.push(Lir::Block(BlockType::Empty)); // $skip_done
            out.push(Lir::Loop(BlockType::Empty)); // $skip
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I64GeS);
            out.push(Lir::BrIf(1)); // scalar >= index → $skip_done
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::I64GeS);
            out.push(Lir::BrIf(1)); // pos >= bytelen → $skip_done (index out of range)
            emit_skip_one_scalar(out, &push_is_lead);
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::ConstI64(1));
            out.push(Lir::I64Add);
            out.push(Lir::LocalSet(scalar_slot));
            out.push(Lir::Br(0)); // → $skip
            out.push(Lir::End); // end $skip
            out.push(Lir::End); // end $skip_done
            // found = (index >= 0) & (scalar == index) & (pos < bytelen). scalar only reaches `index` if it
            // did not hit end first, so `scalar == index && pos < bytelen` is the in-range condition; the
            // explicit `index >= 0` guards a negative index (where scalar=0 stayed below index).
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [index >= 0]
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::LocalGet(index_slot));
            out.push(Lir::I64Eq); // [.., scalar == index]
            out.push(Lir::I32And);
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::I64LtS); // [.., pos < bytelen]
            out.push(Lir::I32And); // [found]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — measure the scalar's byte span and slice it. spanstart = pos; advance pos past this
            // scalar; span_len = pos - spanstart. `bytes-slice(str, spanstart, span_len)` CONSUMES str.
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalSet(spanstart_slot));
            emit_skip_one_scalar(out, &push_is_lead);
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            // `bytes-slice` CONSUMES its buffer, but `str` is a BORROWED operand of `String.at` (the
            // `str_slot` handle is only borrowed — a param/local the caller owns, or an owned temporary
            // this arm's own end reclaims uniformly). So RETAIN it (`dup`, rc++) before the consuming slice,
            // exactly as `List.at` `dup`s the borrowed `vec-get` element before the `Some` consumes it — the
            // slice then owns an INDEPENDENT reference, and `str` is left untouched for its other uses (the
            // recursive `(cnt s …)` that threads the same string). Without this, slicing consumes the sole
            // reference to `str`; the pre-fix code masked it by LEAKING the un-compacted `Some(slice)` (the
            // leak pinned `str` alive but compared by rope offset — wrong), and compacting the slice
            // (below) then `op_drop`s `str`'s node → a use-after-free the recursive scan hit as a trap.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_DUP)); // rc++ str: the slice takes an independent reference
            out.push(Lir::LocalGet(str_slot)); // [disc_some, str] (retained)
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I32WrapI64); // [.., spanstart:i32]
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I64Sub);
            out.push(Lir::I32WrapI64); // [.., span_len:i32]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [disc_some, slice-handle] (consumes the dup'd str)
            // COMPACT the fresh slice to an INDEPENDENT flat leaf before wrapping it in `Some`. A
            // `bytes-slice` result is a ROPE node — a `[off, len]` offset INTO the source string — whose
            // PHYSICAL bytes are the source's, not a flat `[byte…]` leaf. A String's content-equality
            // (`Core::ValueEq` → `champ_eq`) and its map/set-key hashing compare PHYSICAL bytes, so a rope
            // slice compares by its offset and never matches a flat twin of identical content (the
            // `String.at` content-equality miscompile: `(= (String.at s i) "a")` silently false; a lexer
            // over a runtime string cannot classify a char). `bytes-compact` here is REFCOUNT-NEUTRAL — it
            // CONSUMES the owned slice we just produced and returns a content-equal owned leaf (flattening
            // releases the source-string reference the slice pinned, exactly the reference `bytes-slice`
            // transferred in), so the ownership accounting is unchanged (owned in, owned out) and no
            // downstream borrow/drop shifts. Doing it at the PRODUCER (not the `=` site) fixes ALL uses of
            // the result — equality, a map/set key, a re-concat — and is cheap: a `String.at` result is a
            // SINGLE Unicode scalar (≤4 bytes), so the flatten copies at most 4 bytes. This is why the
            // `Core::ValueEq` owned-String compaction did NOT catch it: the extracted payload reaches `=` as
            // a BORROW from the `Some` wrapper (owned by it), so the value-eq site cannot own-and-compact it
            // without a double-free — the slice must be flattened while it is still the fresh owned producer
            // result, here.
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // slice rope → independent flat leaf (owned→owned)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None. `str` was BORROWED (the Some branch `dup`'d it for the slice; this branch takes
            // no reference), so it is NOT dropped here — its owner reclaims it (an enclosing `let`/param),
            // exactly as `List.at`'s None branch leaves its list untouched. `String.at` is now a clean
            // BORROW of its string, like `List.at`/`Bytes.at`.
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // `String.slice string start end` — the fallible half-open SCALAR sub-range `[start, end)`. Walk the
        // flat UTF-8 buffer scalar-by-scalar (identical machinery to `String.at`, generalized to TWO scalar
        // positions): PHASE 1 skips `start` scalars, recording the byte position `spanstart`; PHASE 2 skips
        // to `end`, leaving the byte position `pos`. The slice is the byte span `[spanstart, pos)`. In range
        // (`start >= 0 && start <= end && scalar == end` — reaching `scalar == end` proves exactly `end`
        // scalars existed, so `end <= scalar-len`) → `Some(bytes-slice(str, spanstart, pos - spanstart))`
        // COMPACTED to an independent flat leaf; else `None`. `pos == bytelen` is a VALID end (slicing to the
        // very end), so the found test does NOT require `pos < bytelen` (unlike `String.at`, which reads a
        // scalar AT pos). `str` is BORROWED (Some branch `dup`s it before the consuming `bytes-slice`; None
        // branch takes no reference) — the owner reclaims it, exactly like `String.at`.
        Core::StrSlice {
            string,
            start,
            end,
            disc_some,
            disc_none,
        } => {
            let str_slot = base;
            let start_slot = base + 1;
            let end_slot = base + 2;
            let pos_slot = base + 3;
            let scalar_slot = base + 4;
            let bytelen_slot = base + 5;
            let spanstart_slot = base + 6;
            *high = (*high).max(spanstart_slot + 1);
            scratch_ty.insert(str_slot, ValType::I32);
            for s in [
                start_slot,
                end_slot,
                pos_slot,
                scalar_slot,
                bytelen_slot,
                spanstart_slot,
            ] {
                scratch_ty.insert(s, ValType::I64);
            }
            // Each operand's TRANSIENT scratch floats ABOVE the running high-water, not at a fixed
            // `base + 7`. A wasm local's type is fixed function-wide, so if `start` (e.g. a checked-arith
            // `(+ i 1)`, i64 `$r`) and `end` (e.g. `String.scalar-len`, whose owned-reclaim tees the i32
            // string handle into a scratch slot) both reset their floor to `base + 7`, the LATER operand
            // reuses a slot the earlier one already typed at a DIFFERENT width → "expected i32, found i64",
            // module invalid (the recursive-param-rebound-sliceconcat class; sibling of the br_table fix,
            // width-disjoint-slot family). Advancing each operand's floor to `*high` hands it fresh,
            // never-typed slots. The operands are each stored into their fixed slot immediately, so the
            // transient scratch is dead after each store — floating the floor costs nothing but disjointness.
            emit(db, string, slots, base + 7, high, scratch_ty, layout, out)?; // [str]
            out.push(Lir::LocalSet(str_slot));
            let start_base = (base + 7).max(*high);
            emit(db, start, slots, start_base, high, scratch_ty, layout, out)?; // [start:i64]
            out.push(Lir::LocalSet(start_slot));
            let end_base = (base + 7).max(*high);
            emit(db, end, slots, end_base, high, scratch_ty, layout, out)?; // [end:i64]
            out.push(Lir::LocalSet(end_slot));
            // byte-count (i64), read repeatedly below.
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN));
            out.push(Lir::I64ExtendI32U);
            out.push(Lir::LocalSet(bytelen_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(pos_slot)); // pos = 0
            out.push(Lir::ConstI64(0));
            out.push(Lir::LocalSet(scalar_slot)); // scalar = 0
            // A byte at `pos` (`pos < bytelen`) begins a NEW scalar iff it is NOT a `10xxxxxx` continuation
            // byte: `(bytes-get(str, pos) & 0xC0) != 0x80`. (Same helper as `String.at`.)
            let push_is_lead = |out: &mut Emit| {
                out.push(Lir::LocalGet(str_slot));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::I32WrapI64);
                out.push(Lir::CallImport(OP_BYTES_GET)); // [byte:i32]
                out.push(Lir::ConstI32(0xC0));
                out.push(Lir::I32And);
                out.push(Lir::ConstI32(0x80));
                out.push(Lir::I32Ne); // [(byte & 0xC0) != 0x80]
            };
            // "Advance `pos` past ONE whole scalar" — `pos++`, then skip continuation bytes. Precondition:
            // `pos < bytelen`. (Same helper as `String.at`.)
            let emit_skip_one_scalar = |out: &mut Emit, push_is_lead: &dyn Fn(&mut Emit)| {
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Block(BlockType::Empty)); // $cont_done
                out.push(Lir::Loop(BlockType::Empty)); // $cont
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(bytelen_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // pos >= bytelen → $cont_done
                push_is_lead(out);
                out.push(Lir::BrIf(1)); // lead byte → $cont_done
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Br(0)); // → $cont
                out.push(Lir::End); // end $cont
                out.push(Lir::End); // end $cont_done
            };
            // A "skip scalars until `scalar >= <limit_slot>` or `pos >= bytelen`" loop — used for both
            // phases (limit = start, then end). Advances pos/scalar together. Captures `push_is_lead` /
            // `emit_skip_one_scalar` from the enclosing scope (rather than taking them as params) so its
            // signature stays a simple `(&mut Emit, u32)`.
            let emit_skip_until = |out: &mut Emit, limit_slot: u32| {
                out.push(Lir::Block(BlockType::Empty)); // $done
                out.push(Lir::Loop(BlockType::Empty)); // $skip
                out.push(Lir::LocalGet(scalar_slot));
                out.push(Lir::LocalGet(limit_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // scalar >= limit → $done
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(bytelen_slot));
                out.push(Lir::I64GeS);
                out.push(Lir::BrIf(1)); // pos >= bytelen → $done (limit beyond scalar-len)
                emit_skip_one_scalar(out, &push_is_lead);
                out.push(Lir::LocalGet(scalar_slot));
                out.push(Lir::ConstI64(1));
                out.push(Lir::I64Add);
                out.push(Lir::LocalSet(scalar_slot));
                out.push(Lir::Br(0)); // → $skip
                out.push(Lir::End); // end $skip
                out.push(Lir::End); // end $done
            };
            // PHASE 1 — skip `start` scalars, then record the slice's byte start.
            emit_skip_until(out, start_slot);
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalSet(spanstart_slot)); // spanstart = byte pos of the start-th scalar
            // PHASE 2 — skip on to `end`; `pos` is then the byte position of the end-th scalar boundary.
            emit_skip_until(out, end_slot);
            // found = (start >= 0) & (start <= end) & (scalar == end). `scalar == end` proves exactly `end`
            // scalars were consumed (so `end <= scalar-len`); `start <= end` rejects a reversed range (which
            // could otherwise reach `scalar == end` when phase 1 ran out at exactly `end` scalars); `start >=
            // 0` rejects a negative start (which skips no scalars, leaving `scalar` free to reach `end`).
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [start >= 0]
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::LocalGet(end_slot));
            out.push(Lir::I64LeS); // [.., start <= end]
            out.push(Lir::I32And);
            out.push(Lir::LocalGet(scalar_slot));
            out.push(Lir::LocalGet(end_slot));
            out.push(Lir::I64Eq); // [.., scalar == end]
            out.push(Lir::I32And); // [found]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — `Some(bytes-slice(str, spanstart, pos - spanstart))`. `bytes-slice` CONSUMES its buffer,
            // but `str` is BORROWED, so `dup` (rc++) before the slice takes an independent reference (exactly
            // as `String.at` does). COMPACT the fresh slice rope to a flat leaf so its content-equality /
            // key-hashing compares by physical bytes, not rope offset (the `String.at` content-eq fix,
            // refcount-neutral). `spanstart == pos` (an empty slice, `start == end`) yields `Some ""`.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(str_slot));
            out.push(Lir::CallImport(OP_DUP)); // rc++ str: the slice takes an independent reference
            out.push(Lir::LocalGet(str_slot)); // [disc_some, str] (retained)
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I32WrapI64); // [.., spanstart:i32]
            out.push(Lir::LocalGet(pos_slot));
            out.push(Lir::LocalGet(spanstart_slot));
            out.push(Lir::I64Sub);
            out.push(Lir::I32WrapI64); // [.., span_len:i32]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [disc_some, slice-handle] (consumes the dup'd str)
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // slice rope → independent flat leaf (owned→owned)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None. `str` was BORROWED (this branch took no reference), so it is NOT dropped here; its
            // owner reclaims it (exactly like `String.at`'s None branch).
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // `Bytes.concat(a, b)` — emit both handles, `bytes-concat` (consumes both, returns the new one).
        Core::BytesConcat { lhs, rhs } => {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?; // [a]
            // `rhs` emits ABOVE the running high-water, NOT at the same `base` as `lhs` — the disjoint-slot
            // discipline the sibling `Core::ListConcat` already follows. When `lhs` reserves a PERSISTENT
            // scratch slot at `base` (a `SumPayload` rope-child Perceus RETAIN tees the borrowed handle into
            // `base` as an i32 and raises `*high`), `rhs` emitted at the stale `base` would reuse that slot at
            // a DIFFERENT width — a checked `(+ s 1)` counter step in the branch-picked suffix claims `base`
            // as an i64 guard temp — so the one wasm local is declared at two widths and the module fails to
            // validate ("expected i64, found i32"). This is the `(String.concat r (if … "x" "yz"))` next-state
            // rebuild in a tuple-(scalar,string)-state tail-resumptive fold (breaker slmin11). Floating `rhs`
            // past `lhs`'s high-water keeps the two operands on disjoint slots.
            emit(db, rhs, slots, *high, high, scratch_ty, layout, out)?; // [a, b]
            out.push(Lir::CallImport(OP_BYTES_CONCAT)); // → [a++b]
            Ok(())
        }
        // `BigInt.of x` on a runtime `Int a` — widen to a BigInt heap leaf (an i32 handle). `x` is an i64
        // SCALAR (no heap ref), so nothing to drop — a fresh owned handle is left on the stack.
        //
        // SIGNEDNESS: `bigint-of-i64`'s operand is a SIGNED i64, so it reads the high bit as a sign. That is
        // correct for a SIGNED source width (Int8..Int64), but a runtime `UInt64` value ≥ 2^63 has its high
        // bit SET as MAGNITUDE, not sign — passing it through `bigint-of-i64` yields the WRONG NEGATIVE
        // BigInt (`BigInt.of` is `∀a.(Int a)->BigInt`, and a big UInt64 is a positive value). So for an
        // UNSIGNED source, build the BigInt from its canonical sign-magnitude BYTES instead: a non-negative
        // sign byte (0) + the 8 little-endian magnitude bytes of the u64, then `bigint-of-bytes` (the runtime
        // analogue of the beyond-i64 CONSTANT path `emit_const_bigint_leaf`). `from_sign_magnitude_bytes`
        // re-normalizes, so trailing-zero magnitude bytes are fine — a fixed 9-byte leaf needs no runtime
        // stripping. (A signed source keeps the one-instruction `bigint-of-i64`; only unsigned pays the
        // ~9-op byte materialization, and only Int64/UInt64's top bit can even differ — a narrower unsigned
        // width's value is always < 2^63 so both paths agree, but keying on signedness is uniform + correct.)
        Core::BigIntOfI64 { value } => {
            if int_ty_of(db, value).ground_signed() {
                emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [x]
                // A NARROW signed source (Int8/16/32) lives in an i32 SLOT, but `bigint-of-i64` takes an i64
                // — sign-extend the i32 to i64 first, or the module fails wasm validation ("expected i64,
                // found i32"). A full-width Int64 is already i64 and this is a no-op. (`emit_box_i32_to_i64_
                // extend` is the same widen every `box-int` payload site uses; it extends by the source sign.)
                emit_box_i32_to_i64_extend(db, value, out); // [x : i64]
                out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // → [bigint handle : i32]
                return Ok(());
            }
            // UNSIGNED source: materialize `[sign=0][8 LE magnitude bytes]` then `bigint-of-bytes`. Stash the
            // u64 value in a scratch i64 local so each byte-extraction re-reads it (the operand emits once).
            let val_slot = *high;
            *high = val_slot + 1;
            scratch_ty.insert(val_slot, ValType::I64);
            emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [x]
            // A NARROW unsigned source (UInt8/16/32) is in an i32 slot — ZERO-extend to i64 before stashing
            // in the i64 scratch (else the `local.set` type-mismatches, and the high magnitude bytes would be
            // garbage). `emit_box_i32_to_i64_extend` zero-extends an unsigned narrow int; a full-width UInt64
            // is already i64 (no-op). This keeps the byte-extraction loop reading a correct 64-bit magnitude.
            emit_box_i32_to_i64_extend(db, value, out); // [x : i64]
            out.push(Lir::LocalSet(val_slot)); // [] — x stashed
            out.push(Lir::ConstI32(9)); // [9] — 1 sign byte + 8 magnitude bytes
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            out.push(Lir::ConstI32(0)); // [buf, index=0]
            out.push(Lir::ConstI32(0)); // [buf, 0, sign=0 (non-negative)]
            out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
            for i in 0..8u32 {
                out.push(Lir::ConstI32((i + 1) as i32)); // [buf, index=i+1]
                out.push(Lir::LocalGet(val_slot)); // [buf, i+1, x]
                out.push(Lir::ConstI64((i * 8) as i64)); // [buf, i+1, x, shift]
                out.push(Lir::I64ShrU); // [buf, i+1, x >>u (8*i)] — LOGICAL shift (magnitude, not sign)
                out.push(Lir::I32WrapI64); // [buf, i+1, low32] — wrap keeps the low byte in the low bits
                out.push(Lir::ConstI32(0xFF)); // [buf, i+1, low32, 0xFF]
                out.push(Lir::I32And); // [buf, i+1, byte_i]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
            }
            out.push(Lir::CallImport(OP_BIGINT_OF_BYTES)); // consumes buf → [fresh owned BigInt handle : i32]
            Ok(())
        }
        // `Int64.of b` on a runtime BigInt — checked narrow back to i64 (traps out of range at run time).
        // `bigint-to-i64-checked` BORROWS its operand (`unbox_bigint` reads without consuming) and returns
        // an i64 scalar, so an OWNED-temporary operand must be dropped after the read (a borrowed param/
        // local is left to its owner) — the `value-eq` reclamation discipline for a borrowing op.
        Core::BigIntToI64 { operand } => emit_bigint_borrow_unary(
            db,
            operand,
            OP_BIGINT_TO_I64_CHECKED,
            high,
            slots,
            scratch_ty,
            layout,
            out,
        ),
        // `Char.to-int c` on a runtime char (Char-rep 1/N) — the `Char` operand is an i32 slot holding its
        // Unicode code point (`valtype_of(Ty::Char) == I32`), and the result is `Int64` (an i64 slot). Emit
        // the operand, then ZERO-EXTEND i32→i64 (`i64.extend_i32_u` — a code point is non-negative, so the
        // unsigned extend gives the exact scalar value). No runtime op, no handle, nothing to reclaim (the
        // operand is a scalar). This is the whole op: a `Char` IS its code point in the slot.
        Core::CharToInt { operand } => {
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [code:i32]
            out.push(Lir::I64ExtendI32U); // → [code:i64]
            Ok(())
        }
        // `Char.from-int n` on a runtime Int64 (Char-rep 4/N follow-on) — the FALLIBLE, TOTAL conversion
        // `Int64 -> (Option Char)`. Evaluate `n` once into a scratch (read up to 3× for the range test), test
        // the Unicode-scalar domain — `n u<= 0x10FFFF` (unsigned, so a NEGATIVE n wraps huge → false) AND NOT
        // a surrogate (`n u< 0xD800 || n u> 0xDFFF`) — then `if valid (Some #\<n>) else None`. The `Some`
        // payload is the code point BOXED as a char leaf: `n` is already the validated i64 code point, so
        // `box-int` stores it in the i64 heap cell exactly as a Char payload does (Char boxes like a narrow
        // int, Char-rep 4/N; read back with get-int + the i64→i32 narrow). Mirrors the `List.at` fallible-
        // Option shape; the exact scalar test the `lower` fold + the rust `char::from_u32` path use.
        Core::IntToCharChecked {
            operand,
            disc_some,
            disc_none,
        } => {
            let n_slot = base.max(*high);
            *high = (*high).max(n_slot + 1);
            scratch_ty.insert(n_slot, ValType::I64);
            let op_floor = n_slot + 1;
            emit(db, operand, slots, op_floor, high, scratch_ty, layout, out)?; // [n:i64]
            out.push(Lir::LocalSet(n_slot));
            // valid = (n u<= 0x10FFFF) & (n u< 0xD800 | n u> 0xDFFF)
            out.push(Lir::LocalGet(n_slot));
            out.push(Lir::ConstI64(0x10_FFFF));
            out.push(Lir::I64LeU); // [n<=maxcp]
            out.push(Lir::LocalGet(n_slot));
            out.push(Lir::ConstI64(0xD800));
            out.push(Lir::I64LtU); // [.., n<0xD800]
            out.push(Lir::LocalGet(n_slot));
            out.push(Lir::ConstI64(0xDFFF));
            out.push(Lir::I64GtU); // [.., n<0xD800, n>0xDFFF]
            out.push(Lir::I32Or); // [n<=maxcp, not-surrogate]
            out.push(Lir::I32And); // [valid]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // Some(char): [disc_some] ; box the validated code point as a char leaf (box-int on the i64) ; sum-new
            out.push(Lir::ConstI32(disc_some as i32));
            out.push(Lir::LocalGet(n_slot)); // [disc_some, n:i64]
            out.push(Lir::CallImport(OP_BOX_INT)); // [disc_some, char-leaf]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // `Rational.numerator`/`denominator` on a runtime Rational — `rational-num`/`rational-den` BORROW
        // the Rational handle (`unbox` reads without consuming) and return a FRESH owned BigInt handle. Same
        // borrow-and-reclaim shape as `BigIntToI64` (drop an owned-temporary operand after the read); the
        // returned BigInt handle stays on the stack. (`emit_bigint_borrow_unary` is result-type-agnostic —
        // the operand is an i32 heap handle either way; here it's a Rational rather than a BigInt.)
        Core::RationalNum { operand } => emit_bigint_borrow_unary(
            db,
            operand,
            OP_RATIONAL_NUM,
            high,
            slots,
            scratch_ty,
            layout,
            out,
        ),
        Core::RationalDen { operand } => emit_bigint_borrow_unary(
            db,
            operand,
            OP_RATIONAL_DEN,
            high,
            slots,
            scratch_ty,
            layout,
            out,
        ),
        // A runtime BigInt `+`/`-`/`*`/`/` — the runtime op BORROWS both operand handles (`unbox_bigint`)
        // and returns a FRESH owned result handle, so each OWNED-temporary operand is dropped after the
        // call while the new result is kept. A borrowed param/local operand is NOT dropped (its owner
        // reclaims it). Same borrow-and-reclaim shape as `value-eq`, but the result is a handle to keep.
        Core::BigIntBinOp { op, lhs, rhs } => {
            let import = match op {
                crate::core::BigIntOp::Add => OP_BIGINT_ADD,
                crate::core::BigIntOp::Sub => OP_BIGINT_SUB,
                crate::core::BigIntOp::Mul => OP_BIGINT_MUL,
                crate::core::BigIntOp::Div => OP_BIGINT_DIV,
                crate::core::BigIntOp::Rem => OP_BIGINT_REM,
            };
            emit_bigint_borrow_binary(db, lhs, rhs, import, high, slots, scratch_ty, layout, out)
        }
        // A runtime BigInt COMPARISON `<`/`>`/`<=`/`>=`/`=` — `bigint-cmp` BORROWS both operands and
        // returns the three-way `-1`/`0`/`1` (`a<b`/`a=b`/`a>b`) as an i64; the operator is then the
        // SIGNED i64 compare of that against `0`: `< → cmp <ₛ 0`, `> → cmp >ₛ 0`, `<= → cmp <=ₛ 0`,
        // `>= → cmp >=ₛ 0`, `= → cmp == 0` (`i64.eqz`). The borrowing binary helper emits the operands,
        // drops each OWNED temporary, and leaves the i64 `cmp` result; then push the compare-with-zero.
        Core::BigIntCmp { op, lhs, rhs } => {
            emit_bigint_borrow_binary(
                db,
                lhs,
                rhs,
                OP_BIGINT_CMP,
                high,
                slots,
                scratch_ty,
                layout,
                out,
            )?; // → [cmp : i64]
            match op {
                Prim::Eq => out.push(Lir::I64Eqz), // cmp == 0
                Prim::Lt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LtS);
                }
                Prim::Gt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GtS);
                }
                Prim::Le => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LeS);
                }
                Prim::Ge => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GeS);
                }
                // `lower_bigint_cmp` only builds `BigIntCmp` for a comparison prim; anything else is a
                // compiler invariant violation.
                _ => {
                    return Err(Reject::decline("BigIntCmp carries a non-comparison prim"));
                }
            }
            Ok(())
        }
        // `Rational.of n d` on runtime ints — widen EACH int operand to a BigInt leaf (`bigint-of-i64`),
        // then `rational-of` (which CONSUMES both BigInt handles, normalizes, and builds the 2-handle
        // rational node; traps on a zero denominator at run time). Both `num`/`den` are i64 scalars (no
        // heap ref), so nothing to drop — the two fresh BigInt handles are consumed by `rational-of`.
        Core::RationalOfInts { num, den } => {
            emit(db, num, slots, base, high, scratch_ty, layout, out)?; // [num : i64]
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big : i32]
            let den_base = base.max(*high);
            emit(db, den, slots, den_base, high, scratch_ty, layout, out)?; // [num-big, den : i64]
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [num-big, den-big]
            out.push(Lir::CallImport(OP_RATIONAL_OF)); // → [rational handle : i32]
            Ok(())
        }
        // `Rational.of-int n` — the whole rational `n/1`: widen `n` and the constant `1` to BigInt, then
        // `rational-of`. `n` is an i64 scalar.
        Core::RationalOfIntWiden { value } => {
            emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [n : i64]
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [n-big : i32]
            out.push(Lir::ConstI64(1));
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // [n-big, 1-big]
            out.push(Lir::CallImport(OP_RATIONAL_OF)); // → [rational handle]
            Ok(())
        }
        // A runtime Rational `+`/`-`/`*`/`/` — the runtime op BORROWS both operand handles and returns a
        // FRESH normalized rational; each OWNED-temporary operand is dropped after the call. Same
        // borrow-and-reclaim shape as the BigInt arithmetic (reuses `emit_bigint_borrow_binary`, which is
        // op-generic — it just threads two handle operands, drops owned temporaries, and leaves the result).
        Core::RationalBinOp { op, lhs, rhs } => {
            let import = match op {
                crate::core::RationalOp::Add => OP_RATIONAL_ADD,
                crate::core::RationalOp::Sub => OP_RATIONAL_SUB,
                crate::core::RationalOp::Mul => OP_RATIONAL_MUL,
                crate::core::RationalOp::Div => OP_RATIONAL_DIV,
            };
            emit_bigint_borrow_binary(db, lhs, rhs, import, high, slots, scratch_ty, layout, out)
        }
        // A runtime Rational COMPARISON — `rational-cmp` (three-way `-1`/`0`/`1`) BORROWS both operands,
        // then the operator's signed i64 compare-with-zero (`=`→`i64.eqz`), exactly like `BigIntCmp`.
        Core::RationalCmp { op, lhs, rhs } => {
            emit_bigint_borrow_binary(
                db,
                lhs,
                rhs,
                OP_RATIONAL_CMP,
                high,
                slots,
                scratch_ty,
                layout,
                out,
            )?; // → [cmp : i64]
            match op {
                Prim::Eq => out.push(Lir::I64Eqz),
                Prim::Lt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LtS);
                }
                Prim::Gt => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GtS);
                }
                Prim::Le => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64LeS);
                }
                Prim::Ge => {
                    out.push(Lir::ConstI64(0));
                    out.push(Lir::I64GeS);
                }
                _ => {
                    return Err(Reject::decline("RationalCmp carries a non-comparison prim"));
                }
            }
            Ok(())
        }
        // `Bytes.compact(b)` — emit the handle, `bytes-compact` (consumes it, returns a content-equal one).
        // `Bytes.compact` realizes the memory model's storage-independence guarantee: it derives from a
        // value (a `bytes-slice`/`bytes-concat` rope that may RETAIN a large backing buffer through a small
        // window) another value EQUAL to it whose storage is independent (a fresh flat leaf holding only the
        // window's bytes), so the larger buffer can be released — changing storage use without changing the
        // value.
        //= spec/capabilities/memory-and-resource-model.md#retained-storage-is-what-a-value-s-representation-holds-live
        //# A program MUST be able to derive from a value another value equal to it whose storage is independent of the storage the value was derived from, so that a value retaining a small part of a larger value's storage can release the larger value's storage, changing storage use without changing the value.
        Core::BytesCompact { operand } => {
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [b]
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // → [compacted]
            Ok(())
        }
        // A runtime `String.to-bytes(string)` — the TOTAL UTF-8 encoding `String → Bytes`. A String IS a
        // UTF-8 Bytes leaf (byte-identical), so the encoding is a no-op re-view — but the string may be a
        // `String.concat`/`.slice` ROPE whose node `raw` holds header bytes, not content; the result must be
        // a well-formed Bytes value (a nested rope compares/keys WRONG under the tagless heap walk unless
        // flattened AT CONSTRUCTION — the canonicalize-at-construction invariant). `bytes-compact` does
        // exactly that (flatten the rope to a canonical flat leaf, CONSUMES the handle, transfers it out), so
        // it is the whole op — the exact inverse of `str-from-bytes` on well-formed input. No `sum-new`
        // (total, unlike the fallible decode), no `dup` (the handle is owned out of `bytes-compact`).
        Core::StrToBytes { string } => {
            emit(db, string, slots, base, high, scratch_ty, layout, out)?; // [string]
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // → [flat Bytes leaf] (consumes string)
            Ok(())
        }
        // A runtime `str-nfc-normalize(string)` (FINDING #23) — canonicalize a String value to NFC. Emit the
        // string handle, then the one runtime op (`str-nfc-normalize` flattens + NFC-normalizes, CONSUMES the
        // handle, transfers it out — the same handle when already NFC, else a fresh normalized leaf). No
        // `dup` (owned out of the op); the collect twin inserts exactly this one `OP_STR_NFC_NORMALIZE`.
        Core::NfcNormalize { string } => {
            emit(db, string, slots, base, high, scratch_ty, layout, out)?; // [string]
            out.push(Lir::CallImport(OP_STR_NFC_NORMALIZE)); // → [NFC leaf] (consumes string)
            Ok(())
        }
        // `Blake3.of` — the blake3 content hash. Emit the Bytes operand (leaves its handle), then the
        // `hash-blake3` heap op (op 91): it BORROWS the handle and returns a FRESH 32-byte Bytes leaf.
        Core::Blake3Of { operand } => {
            // RECLAMATION (mirror `ListLen`): `hash-blake3` BORROWS its Bytes operand (`op_bytes_len`/
            // `op_bytes_get` reads, no drop) and returns a FRESH digest. A fresh OWNED-TEMPORARY operand
            // (a `Bytes.of` / concat / another op's result) is otherwise never dropped → leaks one Bytes
            // per call (`(= (Blake3.of x) (Blake3.of y))` leaked its two owned inputs). Stash it, drop after
            // the borrowing hash. A BORROWED operand (param / kept-local) is left to its owner.
            let reclaim = matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            );
            if reclaim {
                let op_slot = base;
                *high = (*high).max(op_slot + 1);
                scratch_ty.insert(op_slot, ValType::I32);
                emit(db, operand, slots, base + 1, high, scratch_ty, layout, out)?; // [bytes]
                out.push(Lir::LocalTee(op_slot)); // [bytes], op_slot = the owned operand
                out.push(Lir::CallImport(OP_HASH_BLAKE3)); // → [digest] (borrows bytes)
                out.push(Lir::LocalGet(op_slot)); // [digest, bytes]
                out.push(Lir::CallImport(OP_DROP)); // → [digest] (reclaim the owned temporary)
                return Ok(());
            }
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::CallImport(OP_HASH_BLAKE3)); // → [digest] (borrows bytes)
            Ok(())
        }
        // `Ast.print` (runtime) — render the heap `Ast` to its canonical s-expr TEXT. Emit the `Ast` operand
        // (leaves its handle), bake the disc descriptor into a FRESH `Bytes` buffer on top (the operand emit
        // already ran on a clean stack, so no scratch slot is needed — the per-byte `bytes-set` touches only
        // the top three, leaving the `Ast` handle beneath), then the `ast-print` heap op (op 92): it BORROWS
        // the `Ast` handle + `discs` and returns a FRESH `String` leaf. `ast-print(handle, discs)` — the stack
        // `[ast, discs-buf]` feeds param0=ast, param1=discs. Byte-identical to the compile-time fold.
        Core::AstPrint { operand, discs } => emit_ast_op_with_discs(
            db,
            operand,
            &discs,
            OP_AST_PRINT,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
        ),
        // `Ast.encode` (runtime) — serialize the heap `Ast` to its canonical `cdzast` BYTES. Identical shape
        // to `AstPrint`: emit the `Ast` operand (leaves its handle), bake the 9-disc descriptor into a FRESH
        // `Bytes` buffer on top (the operand emit ran on a clean stack, so per-byte `bytes-set` touches only
        // the top three and leaves the `Ast` handle beneath), then the `ast-encode` heap op (op 93): it
        // BORROWS the `Ast` handle + `discs` and returns a FRESH OWNED `Bytes` leaf. `ast-encode(handle,
        // discs)` — stack `[ast, discs-buf]` feeds param0=ast, param1=discs. Byte-identical to the
        // compile-time `codec::encode` fold (the op runs the SAME shared codec).
        Core::AstEncode { operand, discs } => emit_ast_op_with_discs(
            db,
            operand,
            &discs,
            OP_AST_ENCODE,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
        ),
        // `Ast.decode` (runtime) — parse the canonical `cdzast` BYTES back to a heap `Ast`. The op-call shape
        // is identical to `AstEncode` (emit the bytes operand, bake the 9-disc descriptor, `ast-decode` op 94
        // BORROWS both and drops the discs buffer + owned operand) — but op 94 returns a heap Ast HANDLE, or
        // `0` (`NULL_HANDLE`) on a parse failure, so the result must be WRAPPED as `(Result Ast e)`: a nonzero
        // handle → `(Ok <ast>)` (the handle is owned, used directly as the payload), `0` → `(Err unit)` (the
        // inline-unit constant — the runtime returned no handle, nothing to drop). Mirrors `StrFromBytes`'s
        // `Some`/`None` null-wrap. `codec::decode`-identical to the compile-time fold (shared codec + discs).
        Core::AstDecode {
            operand,
            discs,
            disc_ok,
            disc_err,
        } => {
            emit_ast_op_with_discs(
                db,
                operand,
                &discs,
                OP_AST_DECODE,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
            )?; // [handle-or-0]
            // `emit_ast_op_with_discs` used `base`/`base+1` (ast + discs slots) and floated the operand emit
            // above them; both are dead now, so reuse `base` for the i32 result handle.
            let result_slot = base;
            *high = (*high).max(result_slot + 1);
            scratch_ty.insert(result_slot, ValType::I32);
            out.push(Lir::LocalSet(result_slot)); // result_slot = handle-or-0, stack empty
            out.push(Lir::LocalGet(result_slot));
            out.push(Lir::ConstI32(NULL_HANDLE));
            out.push(Lir::I32Ne); // [ok?]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Ok(handle): the decoded Ast handle is OWNED (op 94 returns a fresh handle); use it
            // directly as the payload under `disc_ok`, no `dup`.
            out.push(Lir::ConstI32(disc_ok as i32)); // [disc_ok]
            out.push(Lir::LocalGet(result_slot)); // [disc_ok, handle]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Ok-handle]
            out.push(Lir::Else);
            // ELSE — Err(unit): the unit payload is the inline-unit constant. Op 94 returned 0 (no handle),
            // so there is nothing to release here (mirrors `StrFromBytes`'s `None`).
            emit_none_option(disc_err, out); // [Err-handle] — sum-new(disc_err, IMM_UNIT)
            out.push(Lir::End);
            Ok(())
        }
        // A runtime `String.from-bytes(bytes)` — the TOTAL UTF-8 decode. Emit the bytes handle,
        // `str-from-bytes` (CONSUMES it; strict UTF-8 validate → the buffer AS a String handle, or NULL when
        // invalid), then build `Some(handle)` / `None`. The returned handle is already OWNED (str-from-bytes
        // transfers the buffer out on success), so it is used DIRECTLY as the `Some` payload — no `dup`. On
        // failure the runtime already dropped the buffer, so the `None` branch has nothing to drop. One
        // scratch slot (the result handle i32) above `base`; the operand recursion floats above it.
        Core::StrFromBytes {
            bytes,
            disc_some,
            disc_none,
        } => {
            let result_slot = base;
            *high = (*high).max(result_slot + 1);
            scratch_ty.insert(result_slot, ValType::I32);
            emit(db, bytes, slots, base + 1, high, scratch_ty, layout, out)?; // [buf]
            out.push(Lir::CallImport(OP_STR_FROM_BYTES)); // [handle-or-NULL] (consumes buf)
            out.push(Lir::LocalSet(result_slot)); // result_slot = handle-or-NULL, stack empty
            // valid = (handle != NULL).
            out.push(Lir::LocalGet(result_slot));
            out.push(Lir::ConstI32(NULL_HANDLE));
            out.push(Lir::I32Ne); // [valid]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(handle). The handle is OWNED (str-from-bytes transferred the buffer out); use it
            // directly as the payload under `disc_some`, no `dup`.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(result_slot)); // [disc_some, handle]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None: the unit payload is the inline-unit constant (as `Map.lookup`/`Bytes.at` do). The
            // buffer was consumed+dropped by str-from-bytes on failure, so there is nothing to release here.
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // A runtime `Bytes.slice(bytes, start, len)` — the fallible sub-range read. Bounds-check `start >=
        // 0 && len >= 0 && start <= bytes-len && len <= bytes-len - start` (all i64), then in bounds build
        // `Some(bytes-slice(bytes, start, len))` — `bytes-slice` CONSUMES its buffer, but `bytes-len` above
        // only BORROWED it, so the one owned ref is consumed exactly by the slice — else `None`. The slice
        // result is a Bytes HANDLE, the `Some` payload directly (no box, unlike `at`'s byte value). Four
        // scratch slots (bytes i32, start i64, len i64, byte-count i64) above `base`; operand recursions
        // float above them.
        //
        // WARNING: The predicate is OVERFLOW-SAFE. The naive `start + len <= bytes-len` OVERFLOWS: for
        // attacker-chosen `start`/`len` near i64::MAX the i64 sum wraps to a negative value that trivially
        // passes the signed `<=`, wrongly taking the in-range path (a wrong `Some`, or a trap when the
        // i32-wrapped index exceeds the runtime's u32 range) — a soundness hole in a FALLIBLE op that
        // promises `None`, never a trap. Instead, once `start >= 0` holds, `bytes-len - start` (with the
        // `start <= bytes-len` guard, so the difference is in `[0, bytes-len]`) cannot underflow — a small
        // u32-extended non-negative i64 — and `len <= bytes-len - start` is the same range test with no
        // add, so no sum ever overflows. Mirrors the const-fold path's i128 check (`lower_bytes_slice`).
        Core::BytesSlice {
            bytes,
            start,
            len,
            disc_some,
            disc_none,
        } => {
            let bytes_slot = base;
            let start_slot = base + 1;
            let len_slot = base + 2;
            let bytelen_slot = base + 3;
            *high = (*high).max(bytelen_slot + 1);
            scratch_ty.insert(bytes_slot, ValType::I32);
            scratch_ty.insert(start_slot, ValType::I64);
            scratch_ty.insert(len_slot, ValType::I64);
            scratch_ty.insert(bytelen_slot, ValType::I64);
            // Each operand emits at a base ABOVE both `base + 4` (the four reserved slots) AND every scratch
            // slot a PRIOR operand's own emit consumed — `(*high).max(base + 4)`, NOT a bare `base + 4`. A
            // `bytes` operand that is a dup-site `Core::SumPayload` (the collection-lookup shape: `(match
            // (Map.lookup …) ((Some bs) (Bytes.slice bs …)))`) floats its Perceus retain child into a slot at
            // `*high` typed I32; a bare `base + 4` base then lets a following perform-threaded i64 `start`/`len`
            // materialize into that SAME index, and a wasm local has ONE type function-wide → i32/i64 collision
            // → an invalid module (the #2311 CallClosure scratch-alias, here at `bytes-slice`). Threading the
            // base past `*high` keeps each operand's scratch DISJOINT.
            let opnd_base = (*high).max(base + 4);
            emit(db, bytes, slots, opnd_base, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::LocalSet(bytes_slot));
            let opnd_base = (*high).max(base + 4);
            emit(db, start, slots, opnd_base, high, scratch_ty, layout, out)?; // [start:i64]
            out.push(Lir::LocalSet(start_slot));
            let opnd_base = (*high).max(base + 4);
            emit(db, len, slots, opnd_base, high, scratch_ty, layout, out)?; // [len:i64]
            out.push(Lir::LocalSet(len_slot));
            // Materialize the byte-count ONCE (i64, u32-extended — read three times below).
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::CallImport(OP_BYTES_LEN)); // [len:i32]
            out.push(Lir::I64ExtendI32U); // [byte-count:i64]
            out.push(Lir::LocalSet(bytelen_slot));
            // in_bounds = (start >= 0) & (len >= 0) & (start <= byte-count) & (len <= byte-count - start),
            // all i64 — OVERFLOW-SAFE (no `start + len`; `byte-count - start` cannot underflow once
            // `start >= 0 & start <= byte-count`).
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [start >= 0]
            out.push(Lir::LocalGet(len_slot));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64GeS); // [start>=0, len>=0]
            out.push(Lir::I32And); // [start>=0 & len>=0]
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::I64LeS); // [.., start <= byte-count]
            out.push(Lir::I32And); // [start>=0 & len>=0 & start<=byte-count]
            out.push(Lir::LocalGet(len_slot));
            out.push(Lir::LocalGet(bytelen_slot));
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::I64Sub); // [.., len, byte-count - start]  (no underflow: start in [0, byte-count])
            out.push(Lir::I64LeS); // [.., len <= byte-count - start]
            out.push(Lir::I32And); // [in_bounds]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(bytes-slice(bytes, start, len)). The operand emit produced ONE owned ref;
            // `bytes-len` above only BORROWED it (rc unchanged), so that one ref is still live and
            // `bytes-slice` CONSUMES it exactly — net zero, no dup, no leak.
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(bytes_slot)); // [disc_some, bytes]
            out.push(Lir::LocalGet(start_slot));
            out.push(Lir::I32WrapI64); // [disc_some, bytes, start:i32]
            out.push(Lir::LocalGet(len_slot));
            out.push(Lir::I32WrapI64); // [disc_some, bytes, start, len:i32]
            out.push(Lir::CallImport(OP_BYTES_SLICE)); // [disc_some, slice-handle] (consumes bytes)
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None. `bytes-slice` was NOT called, so the operand's one owned ref is still live —
            // DROP it (the None path does not consume the bytes) to avoid a leak.
            out.push(Lir::LocalGet(bytes_slot));
            out.push(Lir::CallImport(OP_DROP)); // release the un-consumed bytes reference
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // A runtime PROJECTION `(. t i)` — read element `i` off the operand's array handle and UNBOX it
        // to its scalar: `<operand handle> ; i32.const i ; arr-get ; get-<T>`. The result type (this
        // node's solved type) chooses the unbox op.
        Core::Proj { operand, index } => {
            // RECLAMATION (U13/U14): if the projected `operand` is a fresh OWNED temporary (a peer/host call
            // result, a constructor — `heap_operand_ownership` == Owned) rather than a BORROW of a live
            // binding (a `Param`/`LocalRef`, reclaimed by its owner), the aggregate would otherwise LEAK —
            // `arr-get` borrows it to read the element but nothing releases it. Two element cases:
            //   • SCALAR element (`get_op` Some): `get-int`/`get-bool` COPY the value out, so after the read
            //     the parent can be dropped directly (U13).
            //   • NESTED-COMPOUND element (`get_op` None): the `arr-get` result IS the child handle, a BORROW
            //     into the parent. `dup` the child (rc++) so it survives the parent's drop, THEN drop the
            //     parent — the parent's storage + every OTHER child is reclaimed, and the returned child
            //     stays live under its own retained reference (U14).
            // Stash the aggregate in a scratch i32 slot so it survives the read for the post-read drop.
            // `heap_operand_ownership` declines an operand whose ownership it cannot prove, so an unhandled
            // shape rejects (Owned only on a proven-fresh producer), never leaks wrongly or double-frees.
            let scalar_elem = get_op(db, id)?;
            // A UNIT element: `arr-get` yields the inline-unit sentinel, but a `Unit` projection leaves NO
            // machine value (`valtype_of(Unit) = None`), so the sentinel must be DROPPED. Distinguished
            // from a nested-compound element (both classify as `None`) by this node's solved type — and a
            // Unit is never a live-compound FBIP target, so it skips the child-dup/retain logic below.
            let unit_elem =
                scalar_elem.is_none() && matches!(type_of(db, id).strip_nominal(), Ty::Unit);
            // WARNING: A MATERIALIZED-SLOT operand is BORROWED, not owned-by-this-Proj. When `operand` is a node
            // stashed in `slots` (a `local.get` here — a sum-match scrutinee materialized ONCE into a slot,
            // re-read by every arm), the ENCLOSING construct owns that slot and reclaims it; this Proj only
            // reads it. Reclaiming (drop) here frees-early: the guarded-record UAF
            // (`(match r ((guard (record (x a)(y b)) (> a 0)) (+ a b)) …)` — the guard-cond's field Proj over
            // the materialized record scrutinee dropped it before the body's field Proj re-read it →
            // freed-then-read). `heap_operand_ownership` verdicts the operand's NODE (a `Call`/ctor → Owned)
            // and can't see it was materialized into a shared slot, so gate on slot-membership: a slotted
            // operand is Borrowed (its owner drops it), never reclaimed by a borrowing Proj. A NON-slotted
            // fresh producer (`(. (mk) 0)` inline, no enclosing materialization) still reclaims (a dead temp).
            let reclaim = !slots.contains_key(&operand)
                && matches!(
                    heap_operand_ownership(db, operand),
                    Ok(HandleOwnership::Owned)
                );
            if reclaim {
                let agg_slot = base;
                *high = (*high).max(agg_slot + 1);
                scratch_ty.insert(agg_slot, ValType::I32);
                emit(db, operand, slots, base + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalTee(agg_slot)); // [handle], agg_slot = the owned aggregate
                out.push(Lir::ConstI32(index as i32)); // [handle, i]
                out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle] (BORROWS the aggregate)
                match scalar_elem {
                    _ if unit_elem => {
                        // Unit element: discard the sentinel `arr-get` yielded, then release the owned
                        // aggregate — the projection leaves NOTHING on the stack.
                        out.push(Lir::Drop); // [] (drop the inline-unit sentinel)
                        out.push(Lir::LocalGet(agg_slot));
                        out.push(Lir::CallImport(OP_DROP)); // [] (aggregate reclaimed)
                    }
                    Some(op) => {
                        out.push(Lir::CallImport(op)); // → [scalar (i64 for an int, i32 for a bool)]
                        if needs_get_int_narrow(db, id) {
                            out.push(Lir::I32WrapI64);
                        }
                        // The scalar is now a COPY on the stack; release the owned aggregate (rc--, cascades).
                        out.push(Lir::LocalGet(agg_slot));
                        out.push(Lir::CallImport(OP_DROP)); // [scalar] (unchanged; aggregate reclaimed)
                    }
                    None => {
                        // A NESTED-COMPOUND element: retain the returned child (rc++) so it outlives the
                        // parent, then drop the parent. `dup` POPS its handle, so re-read the child from a
                        // scratch slot for the dup and leave the original copy on the stack as the result.
                        // WARNING: The child slot floats ABOVE `*high` — NOT `base + 1` — because the `operand`
                        // emit above may have spent scratch ABOVE `base + 1` and, crucially, may have bound
                        // a `let` there at a DIFFERENT width (a wasm local has ONE type function-wide). At
                        // MODULE SCALE this is a real miscompile: `(Map.size (. r 1))` where the record `r`'s
                        // producer contains `let x: Int64 = f(...)` binding slot `base + 1` at i64 → this
                        // i32 child tee at `base + 1` re-typed it → `expected i32, found i64`, the whole
                        // component invalid (the shape compiles fine standalone — nothing binds that slot).
                        // `*high` reflects the operand's scratch, so it hands a fresh, never-typed slot.
                        let child_slot = *high;
                        *high = (*high).max(child_slot + 1);
                        scratch_ty.insert(child_slot, ValType::I32);
                        out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                        out.push(Lir::LocalGet(child_slot)); // [child, child]
                        out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
                        out.push(Lir::LocalGet(agg_slot));
                        out.push(Lir::CallImport(OP_DROP)); // drop the parent; the dup'd child survives → [child]
                    }
                }
                return Ok(());
            }
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [handle]
            out.push(Lir::ConstI32(index as i32)); // [handle, i]
            out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle]
            // A scalar element unboxes (`get-int`/`get-bool`, then a NARROW int narrows i64→i32); a
            // nested compound: the handle `arr-get` yields IS the nested compound — use it as-is. (A
            // nested-compound projection of a BORROWED aggregate is kept alive by the aggregate's owner not
            // being dropped while the projection escapes — see `binding_escapes`'s nested-compound Proj arm,
            // which treats such a projection as an ESCAPE of the operand so its owner is not reclaimed.)
            if let Some(op) = scalar_elem {
                out.push(Lir::CallImport(op)); // → [scalar (i64 for an int, i32 for a bool)]
                if needs_get_int_narrow(db, id) {
                    out.push(Lir::I32WrapI64);
                }
            } else if unit_elem {
                // A Unit element: drop the inline-unit sentinel `arr-get` yielded — the projection leaves
                // NO machine value. (Never an FBIP-retained child, so it never reaches the dup branch.)
                out.push(Lir::Drop);
            } else if out.dup_sites.contains(&id) {
                // PERCEUS RETAIN of the projected CHILD (`collect_dup_sites`/`mark_binder_dups` marked this
                // consuming nested-compound projection of a still-live binder): the `arr-get` returned the
                // child as a BORROW (no rc++), so the child's rc is 1 (only the parent's array cell). A
                // consuming op (`vec-push`/…) would FBIP-mutate it in place, corrupting a LATER re-projection
                // that reads the same child. `dup` the child (rc++ → 2) so the consumer takes the persistent
                // copy path and the parent's array stays intact for the later read; the consumer's own drop
                // (or the persistent path's drop of its taken ref) reclaims the extra reference. `dup` POPS
                // its arg and returns nothing, so re-materialize the child from a scratch slot: tee it, dup
                // the copy, leave the original on the stack for the consumer. Float the slot ABOVE `*high`
                // (NOT `base`) — the `operand` emit above may have spent scratch at/above `base` (a `let`
                // binding of a different width), and a wasm local has ONE type function-wide, so reusing
                // `base` here re-types that slot → invalid module at module scale (see the reclaim arm).
                let child_slot = *high;
                *high = (*high).max(child_slot + 1);
                scratch_ty.insert(child_slot, ValType::I32);
                out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                out.push(Lir::LocalGet(child_slot)); // [child, child]
                out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
            }
            Ok(())
        }
        // A sum-variant pattern's payload binder — WALK the access `path` from the scrutinee handle
        // (`sum-payload` per `Payload` step, `arr-get i` per `Elem` step), then unbox the leaf by THIS
        // node's solved type. A single `[Payload]` path is the flat `(Some x)` case; `[Payload, Payload]`
        // is the nested `(Some (Some y))` binder.
        Core::SumPayload { scrutinee, path } => {
            // SHARED-PREFIX FAST PATH: if a proper prefix of this path (ending in `Payload`) was
            // materialized into a slot for this arm body (`collect_sum_payload_prefixes` +
            // `emit_sum_cont`), start from that slot's payload handle and walk only the SUFFIX — instead of
            // re-emitting `<scrutinee> …prefix`. The prefix ends in `Payload`, so the slotted value is a
            // payload handle (`cur = Ty::Any`). Take the LONGEST matching slotted prefix. Only prefixes
            // whose shared extensions are borrowing `Elem` reads are ever recorded (see the collector), so
            // the slot holds a borrowed handle this suffix walk only reads.
            let prefix_hit = (0..path.len()).rev().find_map(|k| {
                out.payload_prefix_slots
                    .get(&(scrutinee, path[..k].to_vec()))
                    .map(|&s| (k, s))
            });
            // A step's array read depends on the CURRENT sub-value's kind: a tuple/record/sum-payload is a
            // flat `arr` (`arr-get`), but a `List` is an RRB `vec` (`vec-get`), and a list REST binder
            // slices the tail with `vec-split`. Track the sub-value type as the walk descends.
            let walk_from;
            let mut cur;
            // The absolute path PREFIX walked so far — used to consult `sum_path_types` (the enclosing
            // switch's recorded entered-variant payload types) so a `Payload` step resolves to the ACTUAL
            // entered variant, not variant 0. When starting from a shared-prefix slot, seed the prefix with
            // the skipped steps so the key stays absolute-from-scrutinee.
            let mut walked_prefix: Vec<crate::core::PathStep>;
            if let Some((k, slot)) = prefix_hit {
                out.push(Lir::LocalGet(slot)); // [payload-handle] — the shared prefix, computed once
                walk_from = k;
                walked_prefix = path[..k].to_vec();
                // The slotted prefix ends in `Payload`; recover its recorded type so a following `Elem`
                // picks the right accessor (else a bare payload handle, `Any`).
                cur = out
                    .sum_path_types
                    .get(&(scrutinee, walked_prefix.clone()))
                    .cloned()
                    .unwrap_or(Ty::Any);
            } else {
                emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle]
                walk_from = 0;
                walked_prefix = Vec::new();
                cur = type_of(db, scrutinee);
            }
            for step in &path[walk_from..] {
                walked_prefix.push(*step);
                match step {
                    crate::core::PathStep::Payload => {
                        out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // → [payload-handle]
                        // The payload's TYPE — a variant carrying a `List` needs a following `Elem` to read
                        // it with `vec-get`, not `arr-get`. Resolve via the ENTERED variant recorded by the
                        // enclosing switch (`sum_path_types`), falling back to variant 0 at the root. This
                        // is what makes a nested list-in-payload element binder `(Ast.List (list x .. r)) →
                        // x` — where `List` is a NON-variant-0 variant — read with `vec-get`; without the
                        // recorded type, variant 0's payload (`Int64`) mis-picked `arr-get` on the RRB vec.
                        cur = match cur.strip_nominal() {
                            Ty::Sum { .. } => payload_step_ty_of(
                                db,
                                scrutinee,
                                Some(scrutinee),
                                &cur,
                                &walked_prefix,
                                &out.sum_path_types,
                            ),
                            // A nominal newtype's `Payload` step is a static unwrap to its inner type.
                            inner => inner.clone(),
                        };
                    }
                    crate::core::PathStep::Elem(i) => {
                        // STRIP nominal before the List check: for an ERASED newtype whose `Payload` step is
                        // elided at resolve time (the path is `[Elem(0)]`, no leading `Payload`), `cur` is
                        // the raw `Ty::Nominal` wrapping the list — an unstripped `matches!(cur, List)` missed
                        // it and mis-emitted `arr-get` on a vec handle (reads garbage/0). `(type Box (Bx
                        // (List Int64)))` matched `(Bx (list x .. r)) → x` is exactly this shape.
                        if matches!(cur.strip_nominal(), Ty::List(_)) {
                            out.push(Lir::ConstI32(*i as i32));
                            out.push(Lir::CallImport(OP_VEC_GET)); // list element → vec-get
                            cur = match cur.strip_nominal() {
                                Ty::List(e) => (**e).clone(),
                                _ => Ty::Any,
                            };
                        } else {
                            out.push(Lir::ConstI32(*i as i32));
                            out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle]
                            // Resolve `cur` to the ELEMENT's type (not `Any`), so a SUBSEQUENT `Elem` into a
                            // nested LIST field picks `vec-get` (else it defaults to `arr-get` on the vec — the
                            // multi-payload variant-with-a-list-field miscompile: `(Both (list a b) c)` walks
                            // `[Payload, Elem(0)→list, Elem(0)→elem]`; without this the list-field's element read
                            // mis-picked arr-get on the RRB vec → unreachable trap).
                            cur = elem_field_ty(&cur, *i);
                        }
                    }
                    crate::core::PathStep::TupleRestFrom(_) => {
                        // A runtime tuple-rest read (a trailing sub-tuple gather from the arr) is not yet
                        // lowered to wasm — decline (slice 1: a CONST tuple-rest folds to a `Core::Tuple`
                        // before emit and never reaches here; the wasm runtime gather is a follow-up slice).
                        // A graceful not-yet decline, never a miscompile.
                        return Err(Reject::unsupported(
                            "a runtime tuple rest binder is not supported on the wasm backend (a constant tuple-rest is)",
                        ));
                    }
                    crate::core::PathStep::RestFrom(k) => {
                        // Tail sublist from `k`: `vec-drop(list, k)` returns the `[k, len)` tail as ONE
                        // handle (dropping the `[0, k)` prefix internally). WARNING: `vec-drop` CONSUMES its
                        // argument (rc--). But the handle on the stack here is a BORROW of the shared match-
                        // arm scrutinee slot (from `emit(scrutinee)` = a plain `local.get`), and a SIBLING
                        // binder in the SAME arm may still read it — e.g. `((list x .. rest) (f rest (+ acc
                        // x)))`, where `x` is a `vec-get` (BORROW) off the same handle. If `vec-drop` runs
                        // first and drops the arm handle's last reference, the vector is reclaimed and the
                        // sibling `x` read returns garbage (0) — a MISCOMPILE (the head element reads 0).
                        // So RETAIN the handle before consuming: rc++ it (`dup`) so `vec-drop` decrements a
                        // fresh reference and the arm handle's own count is unchanged (every co-binder + the
                        // arm's end-of-scope drop still sees a live handle). `dup` POPS a handle (rc++,
                        // returns nothing), so RE-READ the scrutinee's slot for the extra reference rather
                        // than teeing into a new scratch slot — the scrutinee is a `Core::Param`/`LocalRef`
                        // (the materialized arm handle) with a stable slot, so `emit(scrutinee)` is a pure
                        // `local.get`. This needs NO fresh scratch slot (which could alias a sibling
                        // element binder's i64 slot in a multi-binder pattern `(list a b .. rest)`).
                        // Stack here: [handle] (the copy vec-drop will consume). Push another read, dup it
                        // (rc++, pops it), leaving the original copy for vec-drop.
                        // Site A: skip the preservation dup when the scrutinee is a loop param reassigned-
                        // without-drop this iteration whose vec-drop is its LAST emitted use (PART 2). Then
                        // vec-drop consumes the SOLE handle already on the stack (rc1→0, FBIP-reuse into t),
                        // no path-copy, no orphaned preserved ref. Else default: dup so co-binders / the arm
                        // end-of-scope drop see a live handle.
                        let skip_restfrom_dup = matches!(
                            core_of(db, scrutinee),
                            Core::Param { binder } | Core::LocalRef { binder }
                                if slots
                                    .get(&binder)
                                    .is_some_and(|sl| out.loop_reassign_no_dup.contains(sl))
                        );
                        if !skip_restfrom_dup {
                            emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle, handle]
                            out.push(Lir::CallImport(OP_DUP)); // pops the 2nd read, rc++ → [handle]
                        }
                        out.push(Lir::ConstI32(*k as i32));
                        out.push(Lir::CallImport(OP_VEC_DROP)); // → [tail-handle]
                    }
                }
            }
            // A scalar leaf unboxes; a compound handle is used as-is; a UNIT payload binder drops the
            // inline-unit sentinel the walk landed on (a `Unit` binder holds no machine value).
            let unboxed = get_op(db, id)?;
            // PERCEUS RETAIN of the extracted COMPOUND CHILD (`collect_dup_sites`/`mark_binder_dups` marked
            // this consuming payload extraction of a still-live scrutinee — the `SumPayload` arm there): the
            // `sum-payload`/`arr-get` walk returned the child as a BORROW (no rc++), so its rc is 1 (only the
            // scrutinee's cell refs it). A consuming op (`List.push`/…) would FBIP-mutate it in place,
            // corrupting the still-live scrutinee (matched again / threaded to a self-call). `dup` the child
            // (rc++ → 2) so the consumer takes the persistent copy path and the scrutinee's payload stays
            // intact; the consumer's own drop reclaims the extra reference. Only a COMPOUND leaf (`unboxed`
            // None AND not Unit — a real handle) aliases; a scalar `unboxed` COPIES out and is never a marked
            // site. WARNING: `get_op` returns `None` for BOTH a compound handle AND a `Unit` payload (Unit has no
            // machine value — the walk lands on the `IMM_UNIT` sentinel that `emit_heap_read_tail` DROPS). A
            // Unit has no heap cell to alias, so it must NOT take the dup fast path (which would `dup` +
            // return, leaving the sentinel un-dropped → an extra stack value → INVALID WASM). Route Unit
            // through `emit_heap_read_tail` as usual. `dup` POPS its arg and returns nothing, so tee the child,
            // dup the copy, leave the original for the consumer.
            let unit_leaf = matches!(type_of(db, id).strip_nominal(), Ty::Unit);
            // A path ending in `RestFrom` is a list-tail slice lowered to `vec-drop` (`op_vec_drop_tail`),
            // which CONSUMES the scrutinee (`op_drop(v)`) and returns the tail ALREADY OWNED (rc1 — the kept
            // subtrees were dup'd internally by `vec_take_tail`). So the child-retain dup below MUST NOT fire
            // for it: dup'ing an already-owned tail over-retains it (rc2) with only ONE consumer (the next
            // loop iteration's `vec-drop`, which decrements once → rc1), leaking one spine node per iteration
            // — the self-loop-tail fold leak (v-runtime rc-traced: `sum-l` over an N-list leaks the N-1
            // intermediate tails; foldonly 4→0). The retain is only for a BORROWING leaf (`sum-payload`/
            // `arr-get`, rc1 aliased by the still-live scrutinee); a `vec-drop` result is owned + the
            // scrutinee is consumed (no alias). Mirrors `mark_binder_dups`' `ends_in_rest` exclusion.
            // Co-verified: v-memory-safety (invariant: reclaim-once-across-the-back-edge; the fix is
            // dup-suppression, NOT an added drop) + v-runtime (rc-trace: op_vec_drop_tail returns owned).
            let ends_in_rest = matches!(
                core_of(db, id),
                Core::SumPayload { ref path, .. }
                    if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_)))
            );
            if unboxed.is_none() && !unit_leaf && !ends_in_rest && out.dup_sites.contains(&id) {
                // Float the retain slot ABOVE `*high` (NOT `base`) — the scrutinee walk above may have spent
                // scratch at/above `base` (a `let`/materialize of a different width), and a wasm local has
                // ONE type function-wide, so reusing `base` re-types that slot → invalid module at module
                // scale. This is the v-music recursive-list-map bug: `match h with Note(p,..) => List.push(
                // out, Note(p, ..))` reuses the destructured boxed payload `p` (this dup'd child) in a large
                // body where slot `base` already held an i64 binding → `expected i32, found i64`.
                let child_slot = *high;
                *high = (*high).max(child_slot + 1);
                scratch_ty.insert(child_slot, ValType::I32);
                out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                out.push(Lir::LocalGet(child_slot)); // [child, child]
                out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
                return Ok(());
            }
            emit_heap_read_tail(db, id, unboxed, out); // → [scalar | handle | nothing]
            Ok(())
        }
        // `Option.expect` / `Result.expect` on a RUNTIME sum — probe the discriminant; on the PRESENT
        // variant (`disc_present`) read the payload + unbox by the result type, else TRAP (`unreachable`).
        // Materialize the scrutinee ONCE into a fresh i32 slot (a computed scrutinee — a `checked-add` —
        // must not be recomputed for the disc probe and the payload read; a reusable param/local is read
        // from its own slot). Then: `sum-disc == disc_present` selects an `if` whose THEN reads
        // `sum-payload` (+ `get-*` unbox, narrowing an i32 int) and whose ELSE is `unreachable` (the
        // absent-variant trap — textless; core-semantics.md §Requiring The Value Of An Optional Traps On
        // Absence). Both `sum-disc`/`sum-payload` BORROW the handle (no rc change), like a match probe.
        Core::Trap => {
            // `trap` — an unconditional divergence. `unreachable` HALTS and leaves the stack polymorphic,
            // so it validates in ANY result position (the runtime counterpart of `trap`'s `Never` type,
            // exactly as `SumExpect`'s absent branch emits below). The `String` message is already dropped
            // at lowering (the wasm trap carries no text).
            out.push(Lir::Unreachable);
            Ok(())
        }
        // EFFECT NON-LOCAL EXIT — BASELINE PLACEHOLDER (v-wasm-opt fills the CASE-1 emit: `emit(value)` then a
        // bare wasm `return`, since the fold only produces `HandleAbort` when the reduced handle is the whole
        // function body so the function result IS the handle result — see the `Core::HandleAbort` doc). The
        // fold does not yet PRODUCE `HandleAbort`, so this arm is unreachable in the baseline; decline
        // gracefully rather than `unreachable!()` so an unexpected route declines instead of panicking.
        // EFFECT NON-LOCAL EXIT (recursive effect-abort): the fold produces `HandleAbort` ONLY when the
        // reduced abortive handle is the WHOLE function body (handle-result == function-result; a
        // mid-function or cross-recursive-frame abort DECLINES in the fold until a future vertical). So the
        // abort is a plain non-local RETURN of `value` from the enclosing function: `value` is already the
        // handle result type (the E4 abortive-arm type-consistency check guarantees it) = the function
        // result type, and `Lir::Return` pops it as the function result, abandoning the pending
        // continuation AND the self-loop's remaining iterations. `Return` leaves the stack POLYMORPHIC (like
        // `Core::Trap`'s `unreachable`), so this validates in ANY result position. `handle_id` is unused
        // here (it keys the block-depth machinery of the deferred mid-function CASE-2 emit).
        Core::HandleAbort { value, .. } => {
            emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [abort-value]
            out.push(Lir::Return); // non-local exit: value becomes the function/handle result
            Ok(())
        }
        // A KIND-PRESERVING divide-by-zero trap (demoted from a const `(/ 1 0)` in a conditional branch —
        // `lower::demote_conditional_trap`). Emit a guaranteed-trapping division so the runtime surfaces its
        // NATIVE reason ("integer divide by zero", which `trap_kind` canonicalizes to `div-by-zero`) rather
        // than the bare "unreachable" `Core::Trap` reports: `i64.const 0; i64.const 0; i64.div_s` always traps
        // ÷0 regardless of the dividend. The `unreachable` after it keeps the stack POLYMORPHIC (the div_s
        // leaves an i64 that never survives the trap), so this validates in ANY result position exactly like
        // `Core::Trap` — the branch's own type (an integer division result, but possibly a heap Rational, so a
        // bare `Core::Arith` would mis-type the slot) is satisfied by the polymorphic `unreachable`.
        Core::TrapDivZero => {
            out.push(Lir::ConstI64(0));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64DivS);
            out.push(Lir::Unreachable);
            Ok(())
        }
        // A KIND-PRESERVING integer-overflow trap (demoted from a const arithmetic overflow in a conditional
        // branch — `lower::demote_conditional_trap`). `i32.const i32::MIN; i32.const -1; i32.div_s` is the one
        // arithmetic op wasm traps as "integer overflow" (the same trick `Lir::IfIntegerOverflowEnd` uses),
        // so the runtime surfaces the "overflow" kind rather than the bare "unreachable" `Core::Trap` reports.
        // The `unreachable` after it keeps the stack POLYMORPHIC (the div_s result never survives the trap),
        // valid in ANY result position exactly like `Core::Trap`.
        Core::TrapOverflow => {
            out.push(Lir::ConstI32(i32::MIN));
            out.push(Lir::ConstI32(-1));
            out.push(Lir::I32DivS);
            out.push(Lir::Unreachable);
            Ok(())
        }
        Core::SumExpect {
            scrutinee,
            disc_present,
        } => {
            // The sum handle is read TWICE — the disc probe (`sum-disc`) and the present-payload read
            // (`sum-payload`), BOTH BORROWING (no rc change, never consume). HANDLE SLOT REUSE (mirrors
            // `MatchSum`/`List.at`/`MatchList`): a REUSABLE handle — a `Param`/kept `let`-`LocalRef` already
            // resident in a stable slot — is read from its OWN slot directly; no copy. A COMPUTED scrutinee
            // (a call/`if`/fresh construction) is stashed ONCE into a fresh i32 slot reserved ABOVE the
            // running high-water (`*high`), NOT at `base`: when this `SumExpect` is a SUB-EXPRESSION whose
            // SIBLING uses `base` for a different width — `(tuple (AInt (Option.expect …)) (+ i 1))`, where
            // the i64 `(+ i 1)` sibling also starts scratch at `base` — reusing `base` for the i32 handle
            // re-types a slot the sibling `local.set`s at i64, an invalid module. A slot at `*high` is
            // guaranteed never pre-typed. Either way, reading the slot twice evaluates the scrutinee EXACTLY
            // ONCE.
            let reuse = reusable_handle_slot(db, scrutinee, slots);
            let handle_slot = match reuse {
                Some(owner) => owner,
                None => {
                    let handle_slot = *high;
                    *high = handle_slot + 1;
                    scratch_ty.insert(handle_slot, ValType::I32);
                    emit(
                        db,
                        scrutinee,
                        slots,
                        handle_slot + 1,
                        high,
                        scratch_ty,
                        layout,
                        out,
                    )?;
                    out.push(Lir::LocalSet(handle_slot));
                    handle_slot
                }
            };
            // RECLAMATION: the sum SHELL (`sum-new` node) is a FRESH OWNED TEMPORARY when the scrutinee is an
            // owned producer (a `List.at`/`Map.lookup` `Some`, a constructor, a call) rather than a borrowed
            // param/kept-local — `sum-disc`/`sum-payload` only BORROW it, so nothing else drops it and the
            // shell LEAKS one heap cell per call (a `(Option.expect (List.at (build …) i))` in a loop leaks N
            // shells — value-correct, so the value + drop-import tests miss it; the live-objects gate caught
            // it). Drop the shell in the present arm AFTER the payload is read, but ONLY when freeing the shell
            // cannot free the extracted payload the caller keeps: (a) a SCALAR payload (`unboxed.is_some()`)
            // was copied off the boxed cell, so dropping the shell (which cascades into that boxed cell) is
            // safe; (b) a COMPOUND payload at a DUP site was `dup`'d (rc++) before the drop, so the cascade
            // decrements it back to a live rc. A COMPOUND payload that is NOT a dup site is returned AS-IS
            // (borrowed from the shell) — dropping the shell would free it → UAF, so leave it (that shape is
            // the rarer non-live-after compound expect; a residual leak there, never a double-free). Only a
            // FRESHLY-STASHED owned scrutinee is dropped: a reused param/kept-local slot (`reuse.is_some()`)
            // is borrowed and left to its owner (mirrors the `List.len`/`List.at` owned-operand reclaim gate).
            // (2) SumExpect-LOCAL owned treatment: a `String.at`/`Bytes.slice` view in the view/shell reclaim
            // sets is owned-single-payload BY CONSTRUCTION, so its shell is reclaimable HERE even though
            // `heap_operand_ownership(StrAt)` is (deliberately) not globally Owned — this local check keeps the
            // MatchSum Stage-B / value-eq consumers seeing StrAt UNCHANGED (the local>global discipline).
            let reclaim_shell = reuse.is_none()
                && (matches!(
                    heap_operand_ownership(db, scrutinee),
                    Ok(HandleOwnership::Owned)
                ) || out.sumexpect_view_reclaim.contains(&id)
                    || out.sumexpect_shell_reclaim.contains(&id));
            // The result block type is this node's solved type (the payload type).
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "expect result type has no machine representation",
                        ));
                    }
                },
            };
            // disc(handle) == disc_present ?  (`disc_present == 0` → `i32.eqz`, one instruction — the
            // sum-disc eqz special case; a `Some`/`Ok` present variant is discriminant 0.)
            out.push(Lir::LocalGet(handle_slot));
            out.push(Lir::CallImport(OP_SUM_DISC)); // [disc]
            if disc_present == 0 {
                out.push(Lir::I32Eqz); // [present?]
            } else {
                out.push(Lir::ConstI32(disc_present as i32));
                out.push(Lir::I32Eq); // [present?]
            }
            out.push(Lir::If(block_ty));
            // THEN — the present payload: sum-payload + unbox by result type. A scalar unboxes; a compound
            // is used as-is; a UNIT result drops the inline-unit sentinel so the THEN leaves NOTHING —
            // matching the `BlockType::Empty` a Unit result selects above (else a stray handle would defy
            // the block's declared type).
            out.push(Lir::LocalGet(handle_slot));
            out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // [payload-handle]
            let unboxed = get_op(db, id)?;
            // PERCEUS RETAIN of the extracted COMPOUND child (mirrors the `SumPayload` emit): a marked site
            // (`mark_binder_dups`' `SumExpect` arm) is a compound payload consumed while its scrutinee stays
            // live, so `dup` the child (rc++) before it flows into the consuming op — else the shared payload
            // is FBIP-mutated at rc==1 and the still-live scrutinee drifts. Only a COMPOUND leaf (`unboxed`
            // None AND not Unit — a real handle) aliases; a scalar unboxes/copies and is never a site. WARNING:
            // `get_op` returns `None` for BOTH a compound handle AND a `Unit` payload; a Unit has no heap cell
            // to alias and its `IMM_UNIT` sentinel must be DROPPED by `emit_heap_read_tail` — taking the dup
            // fast path would leave the sentinel un-dropped → an extra stack value in the block → INVALID
            // WASM (Copilot PR#441). Route Unit through the `else`. `dup` POPS + returns nothing, so tee the
            // child, dup the copy, leave the original for the consumer. A fresh scratch slot at `*high`
            // (never `base`, which a width-different sibling may claim).
            let unit_leaf = matches!(type_of(db, id).strip_nominal(), Ty::Unit);
            // Also fire for a (2) rope/slice-view SumExpect reclaim (dup the extracted view + drop the Some
            // shell below): the VIEW set (scalar-read-dead — its paired view-drop is the sole Bytes.at's
            // reclaim_bytes / String.scalar-len's reclaim) OR the SHELL set (consumed-onward — the dup here is
            // the NET-0 compensation for the shell-drop cascade; NO view-drop, the consumer owns the view).
            let compound_dupd = unboxed.is_none()
                && !unit_leaf
                && (out.dup_sites.contains(&id)
                    || out.sumexpect_view_reclaim.contains(&id)
                    || out.sumexpect_shell_reclaim.contains(&id));
            if compound_dupd {
                let child_slot = *high;
                *high = child_slot + 1;
                scratch_ty.insert(child_slot, ValType::I32);
                out.push(Lir::LocalTee(child_slot)); // [child], child_slot = child
                out.push(Lir::LocalGet(child_slot)); // [child, child]
                out.push(Lir::CallImport(OP_DUP)); // pops the 2nd copy, rc++ → [child]
            } else {
                emit_heap_read_tail(db, id, unboxed, out); // [scalar | handle | nothing]
            }
            // Drop the owned SHELL now that the payload is off it — SAFE only for a scalar payload (copied
            // off) or a dup'd compound (rc++'d above); a non-dup'd compound is returned borrowed from the
            // shell, so dropping it would UAF (left un-dropped — a residual leak, never a double-free). The
            // shell handle is still in `handle_slot`; `drop` pops it and returns nothing, leaving the payload
            // value on the stack as the block's result.
            if reclaim_shell && (unboxed.is_some() || compound_dupd) {
                out.push(Lir::LocalGet(handle_slot)); // [payload, shell]
                out.push(Lir::CallImport(OP_DROP)); // → [payload] (reclaim the owned Some shell)
            }
            out.push(Lir::Else);
            // ELSE — absent variant: trap. `unreachable` leaves the stack polymorphic, so the block's
            // declared result type validates without a produced value.
            out.push(Lir::Unreachable);
            out.push(Lir::End);
            Ok(())
        }
        Core::If { cond, then_, else_ } => {
            let result = type_of(db, id);
            // FLOW-SENSITIVE DEAD-BRANCH ELIMINATION: when the active branch refinement DECIDES this `if`'s
            // condition (`(if (> n 0) (if (> n 0) …) …)` — the inner cond is known true; `(if (>= n 5) (if
            // (> n 0) …))` — implied), emit ONLY the taken branch and drop the other. The condition is a
            // comparison of a refined (trap-free) variable against a constant, so evaluating it has no
            // effect to preserve. Fires before the select/if lowering so a fully-decided nested conditional
            // costs nothing (not even a `select` on a constant). The taken branch keeps this `if`'s tail
            // context via `emit` (non-tail arm).
            if let Core::Compare { op, lhs, rhs } = core_of(db, cond)
                && let Some(taken) = crate::lower::refined_comparison_const(db, op, lhs, rhs)
            {
                let branch = if taken { then_ } else { else_ };
                trace!(target: "rcdzc::select", node = id.0, taken, "if condition decided by branch refinement — emit only the taken branch");
                return emit_branch(
                    db, branch, &result, slots, base, high, scratch_ty, layout, out,
                );
            }
            // FLOW-SENSITIVE EQUAL-BRANCH COLLAPSE: when both branches reduce to the SAME constant UNDER
            // their respective branch refinements — `(if (> x 10) (if (> x 5) 7 8) 7)`: the then-branch's
            // inner `(> x 5)` is decided true by the `x > 10` refinement, so it is `7`, matching the else —
            // the whole `if` is that constant, provided the condition is trap-free (it is still evaluated,
            // so a trapping cond must stay). This is the emit-time analogue of `lower`'s `core_equiv(then,
            // else)` fold, which only sees branches equal WITHOUT flow facts; `refined_const_value` applies
            // each branch's refinement to expose the equality `lower` could not. Each branch's refined
            // constant is computed under its own pushed frame (as the real emit below would).
            if crate::lower::is_trap_free(db, cond) {
                let base_frame = db.current_refinements();
                let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
                db.push_range_refinements(then_frame);
                let tc = refined_const_value(db, then_);
                db.pop_range_refinements();
                if let Some(tc) = tc {
                    let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
                    db.push_range_refinements(else_frame);
                    let ec = refined_const_value(db, else_);
                    db.pop_range_refinements();
                    if ec.as_ref() == Some(&tc) {
                        // Both branches are the same constant — emit it, dropping the (trap-free) branch.
                        trace!(target: "rcdzc::select", node = id.0, "if with equal refined-constant branches → the constant (trap-free cond dropped)");
                        let cid = crate::lower::synth_core(db, tc, result.clone());
                        return emit_branch(
                            db, cid, &result, slots, base, high, scratch_ty, layout, out,
                        );
                    }
                }
            }
            // IF-CHAIN → INTEGER MATCH: a nested `(if (= X k0) … (if (= X k1) … default))` on one
            // reusable integer scrutinee with ≥3 distinct constants is an integer dispatch a user wrote as
            // chained `if`s. Route it through the match backend so a DENSE range gets an O(1) `br_table`
            // instead of the O(n) `if (== k)` cascade this arm would otherwise emit. (Rust gets the jump
            // table from LLVM; wasm does not.) Fires AFTER the flow-sensitive dead-branch/equal-branch
            // folds above (a decided cond should collapse, not dispatch) and BEFORE the branchless-select
            // below (a 3+-way dispatch is not a 2-arm select).
            if let Some((scrut, arms)) = if_chain_as_int_match(db, cond, then_, else_) {
                let it = int_ty_of(db, scrut);
                let result_it = match &result {
                    Ty::Int(rit) => Some(*rit),
                    _ => None,
                };
                let block_ty = match &result {
                    Ty::Unit => BlockType::Empty,
                    other => match valtype_of(other) {
                        Some(vt) => BlockType::Val(vt),
                        None => {
                            return Err(Reject::decline(
                                "if-chain match result type has no machine representation",
                            ));
                        }
                    },
                };
                return emit_match_arms(
                    db, scrut, &arms, it, result_it, block_ty, slots, base, high, scratch_ty,
                    layout, out,
                );
            }
            // BRANCHLESS SELECT: when both branches are cheap trap-free scalar computations (a
            // param/local/constant, or a small trap-free op like `(& x 7)` — see `is_select_arm`) and the
            // result is a SCALAR (not unit, not a heap handle), emit wasm's `select` instead of an
            // `if`/`else`/`end` block — no branch. `select` pops `[a, b, cond]` and pushes `a` if `cond` is
            // nonzero else `b`, evaluating BOTH unconditionally; that is sound here precisely because each
            // arm is trap-free, allocation-free, effect-free, and cheap (so little is wasted vs the branch
            // it replaces). A HEAP result is excluded: `select` would evaluate both handles and discard one
            // WITHOUT the Perceus `drop` that the owning branch would run, leaking its cell — the `if`
            // (which evaluates only the taken branch) stays for those. This is the classic `min`/`max`/
            // conditional-value idiom `(if (< a b) a b)` and the masked/bitwise conditional
            // `(if c (& x 7) (| x 8))`.
            // BOOLEAN MATERIALIZATION first: `(if c 1 0)`/`(if c 0 1)` is the condition itself (coerced to
            // the result width), cheaper than the `const;const;select` a leaf select would emit.
            if let Some(r) = try_bool_materialization(
                db, cond, then_, else_, &result, slots, base, high, scratch_ty, layout, out,
            ) {
                return r;
            }
            if !matches!(result, Ty::Unit)
                && (!is_heap_type(&result) || ty_is_enum_disc(db, &result))
                && valtype_of(&result).is_some()
                && is_select_arm(db, then_)
                && is_select_arm(db, else_)
            {
                // An ENUM-DISC result is admitted alongside a scalar (see the tail `Core::If` arm): its
                // runtime rep is an i32 discriminant and each enum-disc arm emits just that constant (no
                // allocation, no drop), so a `select` between two discriminants is sound.
                // Each arm is emitted UNDER its branch-refinement frame, exactly as the structured `if`
                // below — a `select` arm computes the same value the `if` arm would, so a refinement that
                // simplifies the arm (elides a redundant mask `(& x 255)` under `x∈[0,255]`, folds a
                // now-constant subexpression) must still apply. Sound: a trap-free arm carries no guard to
                // wrongly elide, the TAKEN arm's refinement always holds (so its refined value is its true
                // value), and the untaken arm's value is discarded by `select` regardless of whether its
                // refinement held. So refining both arms is strictly better (branchless AND still elided).
                let base_frame = db.current_refinements();
                let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
                db.push_range_refinements(then_frame);
                let then_res = emit_branch(
                    db, then_, &result, slots, base, high, scratch_ty, layout, out,
                );
                db.pop_range_refinements();
                then_res?;
                let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
                db.push_range_refinements(else_frame);
                let else_res = emit_branch(
                    db, else_, &result, slots, base, high, scratch_ty, layout, out,
                );
                db.pop_range_refinements();
                else_res?;
                emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
                out.push(Lir::Select);
                return Ok(());
            }
            // Selection order matches wasm's structured `if`: push the condition, open the block with
            // the RESULT type (read off the node's solved type), then the two arms.
            emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
            // The branches start their scratch ABOVE the high-water the COND reached, NOT at `base`. A
            // cond may stash an i32 HEAP HANDLE in a scratch slot (a runtime `value-eq`/`MatchSum` on
            // constructed sums) that stays TYPED for the whole function; a branch reusing that slot at a
            // different width (an i64 arith temp — `(if (= (mk n) (mk 3)) n (find (+ n 1)))`) would force
            // one wasm local to two types and fail validation (`expected i64, found i32`). Advancing to
            // `*high` hands each branch fresh, never-typed slots — the same discipline `MatchSum`'s arms
            // and `emit_call_args` already apply. (A scalar cond leaves `*high == base`, so this is a
            // no-op for the common case and the emitted bytes are unchanged.)
            let branch_base = *high;
            // A `Never` result (BOTH branches diverge) has no valtype but yields no value on any path —
            // both arms end in `unreachable`. Emit an EMPTY (0-result) `if` block, then a trailing
            // `unreachable` AFTER it: the block produces nothing, and the stack-polymorphic `unreachable`
            // satisfies whatever machine slot the ENCLOSING context expects (this `if` may be a value
            // subexpression — `(if b 1 (if c (trap) (trap)))` — where the outer arm wants an i64). The
            // trailing `unreachable` is dead (both arms already trapped) but keeps the module valid in any
            // position. Mirrors `Core::Trap`'s bare `unreachable`. A genuinely unrepresentable result (a
            // non-diverging compound with no machine rep) still DECLINES.
            let mut never_diverges = false;
            let block_ty = match &result {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
                    None => {
                        return Err(Reject::decline(
                            "if result type has no machine representation",
                        ));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            // IF-JOIN PER-ARM DROP (v-memory-safety co-design): reclaim a DIVERGENT heap let-binding (escapes
            // one arm, dead on the other) on its DEAD arm ONLY — the post-body scope-drop suppressed it
            // (whole-body escape), so this is its SOLE reclaim (no double-drop). Capture the plan for THIS
            // `Core::If` node (`remove` consumes it so it fires once); the actual rc-aware DEEP `op_drop`
            // (`LocalGet slot; CallImport OP_DROP` — cascades to dead extracted children like tree's l,r) is
            // emitted AFTER each arm's `emit_branch` (below), NOT at the arm top: the D arm's body may still
            // READ the binder (a projection into its result, e.g. effects-tuple's `(. ab 1)`), so a top-of-arm
            // drop would free it BEFORE the read → UAF. The drop must FOLLOW the arm's reads (v-mem P0 fix).
            let ifjoin_plan = out.ifjoin_arm_drops.remove(&id).unwrap_or_default();
            // IF-JOIN OWNERSHIP-EQUALIZE (FIX A): when THIS `if` is a divergent-ownership let-value, emit an
            // rc-aware `dup(b)` on the ALIAS arm (the arm the result move-aliases the earlier binder `b`), so
            // the let result is UNIFORMLY OWNED. Stack-neutral (`LocalGet slot; OP_DUP` reads a fresh copy and
            // rc++s, leaving the arm's join value untouched). Placed AFTER `emit_branch`, mirroring the arm-drop.
            let ifjoin_dups = out.ifjoin_arm_dups.remove(&id).unwrap_or_default();
            // FLOW-SENSITIVE RANGE REFINEMENT: while emitting a branch, push a refinement frame recording
            // any one-sided bound this branch's condition establishes on a variable (`(< n 2)` → `n ≤ 1`
            // in `then`, `n ≥ 2` in `else`). A guard-elision check inside the branch (`value_range` →
            // `arith_provably_in_range`) then sees the narrowed range and drops a dead overflow guard
            // (`(- n 1)` under `n ≥ 2` cannot underflow). Frame merges the parent's (nested `if`s
            // accumulate). Pushed/popped around EACH branch so the refinement never leaks past it — and
            // popped even on an early `?` return (capture the result, pop, then `?`).
            let base_frame = db.current_refinements();
            // Both branches must produce the `if`'s RESULT machine slot; a bare-literal branch (default
            // Int64) opposite a NARROW branch would otherwise push a mismatched i64 into a narrow-i32
            // block. Ground a bare-`ConstInt` branch to the result's integer width via `emit_operand`,
            // exactly as an operator operand (`@1a4528f`) and a match arm (`@10f7bdb`) are grounded.
            let then_frame = refined_frame_for_branch(db, cond, true, base_frame.clone());
            db.push_range_refinements(then_frame);
            let then_res = emit_branch(
                db,
                then_,
                &result,
                slots,
                branch_base,
                high,
                scratch_ty,
                layout,
                out,
            );
            db.pop_range_refinements();
            then_res?;
            // IF-JOIN PER-ARM DROP (then-arm D-drop): AFTER the then body's reads, before the arm exits — a
            // D-then arm that reads the binder does so BEFORE it is reclaimed. Stack-safe: emit_branch left
            // the arm result in `&result`'s slot; a trailing `LocalGet(slot); OP_DROP` pops only the binder.
            for &(slot, d_is_then) in &ifjoin_plan {
                if d_is_then {
                    out.push(Lir::LocalGet(slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // IF-JOIN OWNERSHIP-EQUALIZE (then-arm dup): if the then-arm is the ALIAS arm, dup `b` so the
            // result owns an independent ref. Stack-neutral: a fresh `LocalGet` copy that OP_DUP rc++s + pops.
            for &(slot, dup_is_then) in &ifjoin_dups {
                if dup_is_then {
                    out.push(Lir::LocalGet(slot));
                    out.push(Lir::CallImport(OP_DUP));
                }
            }
            out.push(Lir::Else);
            // The else branch starts its scratch ABOVE the then branch's high-water (see the TAIL `Core::If`
            // arm for the full rationale): the two mutually-exclusive branches may want the same slot index
            // at DIFFERENT widths (a base arm's i32 Option handle vs a recursive arm's i64 temp), and a
            // slot's type is recorded ONCE — sharing `branch_base` sets one local at both types (invalid
            // module). Advancing past `*high` gives the else branch fresh slots; byte-identical when the
            // then branch used no scratch (`*high == branch_base`).
            let else_base = branch_base.max(*high);
            let else_frame = refined_frame_for_branch(db, cond, false, base_frame);
            db.push_range_refinements(else_frame);
            let else_res = emit_branch(
                db, else_, &result, slots, else_base, high, scratch_ty, layout, out,
            );
            db.pop_range_refinements();
            else_res?;
            // IF-JOIN PER-ARM DROP (else-arm D-drop): AFTER the else body's reads, before End (mirror of the
            // then-arm drop — the binder is reclaimed on the D path once its last read in the arm is done).
            for &(slot, d_is_then) in &ifjoin_plan {
                if !d_is_then {
                    out.push(Lir::LocalGet(slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // IF-JOIN OWNERSHIP-EQUALIZE (else-arm dup): mirror of the then-arm dup for an alias-on-else result.
            for &(slot, dup_is_then) in &ifjoin_dups {
                if !dup_is_then {
                    out.push(Lir::LocalGet(slot));
                    out.push(Lir::CallImport(OP_DUP));
                }
            }
            out.push(Lir::End);
            // A both-diverge (`Never`) `if` yields nothing from its empty block; a trailing `unreachable`
            // supplies the stack-polymorphic value the enclosing value position expects (dead — both arms
            // trapped).
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // A scalar MATCH → a chain of `if`s. The match's solved type is each arm's block-result type.
        // Each non-wildcard arm probes `scrutinee == literal` (push scrutinee, push the literal, compare)
        // and takes its body on a match, else falls through to the next arm; the wildcard arm is the
        // unconditional tail (`else`). The scrutinee is a scalar, so re-pushing it per probe is a cheap
        // local reload — no naming needed.
        Core::Match { scrutinee, arms } => {
            // A `Never` match (EVERY arm body diverges) has no valtype but yields no value on any path —
            // emit an empty block, then a trailing `unreachable` for the enclosing (possibly value)
            // position (same treatment as the both-diverge `Core::If`). A genuinely unrepresentable
            // non-diverging result still DECLINES.
            let mut never_diverges = false;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
                    None => {
                        return Err(Reject::decline(
                            "match result type has no machine representation",
                        ));
                    }
                },
            };
            let it = int_ty_of(db, scrutinee);
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            emit_match_arms(
                db, scrutinee, &arms, it, result_it, block_ty, slots, base, high, scratch_ty,
                layout, out,
            )?;
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // A sum MATCH → a chain of `if`s over `sum-disc(scrutinee)`. Each variant arm probes
        // `sum-disc(scrutinee) == disc` and takes its body on a match; a wildcard/binder arm (`disc:
        // None`) is the unconditional `else` tail. The scrutinee is a heap handle (an i32 local reload
        // per probe, cheap). A payload binder in a body reads `sum-payload(scrutinee)` on its own
        // (`Core::SumPayload`), so the arm dispatch needs only the disc.
        Core::MatchSum { scrutinee, root } => {
            // A `Never` sum match (all decision-tree leaves diverge): empty block + trailing
            // `unreachable`. A genuinely unrepresentable non-diverging result still DECLINES.
            let mut never_diverges = false;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
                    None => {
                        return Err(Reject::decline(
                            "sum match result type has no machine representation",
                        ));
                    }
                },
            };
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            // A REUSABLE scrutinee (a param/local already in a slot) is re-read cheaply per probe. A
            // NON-reusable one (a `List.at`, a call, an `if` — anything computed) must be evaluated ONCE:
            // re-emitting it per probe/payload would recompute the value (rebuilding a list, re-running a
            // call) AND its own scratch would clash with the arm bodies' scratch at the shared `base`,
            // producing an invalid module. Materialize it into a fresh i32 slot (a sum is a heap handle)
            // and record `(scrutinee → slot)` so every re-reference reads the slot (top of `emit`).
            let (arms_slots, arms_base, stashed_slot) = if reusable_handle_src(db, scrutinee, slots)
            {
                (slots.clone(), base, None)
            } else {
                // Reserve a FRESH slot for the scrutinee (an i32 heap handle) ABOVE the running
                // high-water — NOT `base` itself. When this `MatchSum` is an OPERAND of an enclosing op
                // (`(* (match (Bytes.at b i) …) …)`), `base` is that op's operand-scratch floor, which the
                // enclosing arith already TYPED (i64 for its operand/result slot); reusing `base` for the
                // i32 handle would re-type a slot the enclosing op `local.set`s at i64 — an invalid module
                // (`expected i64, found i32`). A slot at `*high` is guaranteed never pre-typed. Emit the
                // scrutinee's value above THAT (its own transient scratch — a `List.at`/`Bytes.at` types
                // slots for the buffer/index/element), then start the arm scratch above the high-water the
                // scrutinee emit reached (`*high`), so an i32 scrutinee-scratch slot never clashes with an
                // i64 arm temp.
                let slot = *high;
                *high = slot + 1;
                // The spill slot's width is the SCRUTINEE'S machine type, NOT always i32: a real boxed sum is
                // an i32 handle, but an ERASED single-variant newtype over a SCALAR (`(type W (Wrap Int64))`)
                // is a bare i64 (no box) — spilling that i64 into a hardcoded-i32 slot re-types one wasm local
                // to two widths → `type mismatch: expected i32, found i64`, an invalid module (a literal-
                // payload arm `(match (mk d) ((Wrap 5) …))` over a runtime-built erased newtype). Default to
                // i32 for a rep-less type (a handle-shaped scrutinee).
                let scrut_vt = valtype_of(&type_of(db, scrutinee)).unwrap_or(ValType::I32);
                scratch_ty.insert(slot, scrut_vt);
                emit(
                    db,
                    scrutinee,
                    slots,
                    slot + 1,
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(Lir::LocalSet(slot));
                let mut m = slots.clone();
                m.insert(scrutinee, slot);
                (m, (*high).max(slot + 1), Some((slot, scrut_vt)))
            };
            // RECLAMATION (the SumExpect owned-shell twin, narrowed after v-patterns caught a UAF): the sum
            // SHELL is a FRESH OWNED TEMPORARY when the scrutinee is an owned producer (a `List.at`/`Map.
            // lookup` `Some`/`None`, a constructor, a call) not a borrowed param/kept-local — the arms only
            // BORROW it (`sum-disc`/`sum-payload`), so nothing else drops it and the shell LEAKS one heap
            // cell per call (a `(match (List.at (build …) i) …)` in a loop leaks N — value-correct, caught by
            // the live-objects gate). Drop the shell after the match — but ONLY when NO arm can borrow a HEAP
            // PAYLOAD HANDLE out of the shell that OUTLIVES the block: `sum_has_only_scalar_payloads` (every
            // variant carries a scalar or nothing). WARNING: A SCALAR-RESULT gate is NOT sufficient (the bug
            // v-patterns caught): the HOL-kernel `term-eq (Comb x y)` returns a scalar Bool but its arms bind
            // `x`/`y` = `sum-payload` HANDLES borrowed from the shell and thread them into a recursive walk
            // that reads them AFTER the match — freeing the shell there frees `x`/`y` mid-use (OOB/UAF). With
            // all-scalar payloads no handle aliases the shell, so the drop is safe. A compound-payload sum is
            // left un-dropped (a residual leak, never a double-free — the conservative floor). Also require: a
            // freshly-stashed owned scrutinee (a reused param/local is borrowed, left to its owner), a REAL
            // BOXED SUM (`is_heap_type && !ty_is_enum_disc` — an all-nullary enum is a bare i32 disc with NO
            // shell + imports no value-heap runtime; its i32 slot width alone doesn't exclude it), and a
            // non-diverging match.
            let scrut_ty = type_of(db, scrutinee);
            // Deep-`drop` the owned freshly-stashed boxed-sum shell after the match (it is a dead temporary).
            // SAFETY vs the v-patterns UAF (an arm MOVES a payload child out that the deep drop would double-
            // free): every consuming compound-child extraction rooted at the scrutinee was `dup`'d in the
            // UPFRONT `collect_shell_reclaim_child_dups` pass (→ `dup_sites`, emitted at each `Core::SumPayload`
            // child), so the shell keeps its own reference and the deep drop nets correctly (dup rc++ → the
            // consumer takes one → the drop takes the shell's). A borrow-only arm moves nothing (empty dup
            // set) and reclaims directly; an all-scalar shell copies its payloads out (empty dup set) and
            // reclaims as before. Requires a freshly-stashed owned scrutinee (a reused param/local is borrowed,
            // left to its owner), a REAL BOXED SUM (`is_heap_type && !ty_is_enum_disc`), and a non-diverging
            // match.
            // WARNING: SAFETY RESTRICTION (sread UAF fix, 2026-07-19): reclaim ONLY an ALL-SCALAR-payload shell.
            // The inc2 broadening ("any owned boxed sum + dup the consumed children") was UNSOUND: it handled
            // a child MOVED OUT (consumed → dup'd) but NOT a child BORROWED OUT via an ALIASING op — e.g.
            // `(match tree ((Arena m _) (Map.lookup m id)))` where `Map.lookup` returns a handle that ALIASES
            // INTO `m` (a shell child); the result outlives the match, and the deep shell drop then frees `m`
            // → the looked-up node is freed-then-read (the sread 33/9 OOB/unreachable UAF). `List.at` has the
            // same alias-out shape. `sum_has_only_scalar_payloads` is the sound floor: a scalar payload COPIES
            // out, so NO handle can alias the shell → the deep drop is safe. A compound-payload shell is left
            // un-dropped (a residual leak — the bbox-arc's consumed-child/tail leak reopens for compound
            // shells, value-correct/non-OOB, tracked as the reclaim-the-compound-shell increment) rather than
            // risk the UAF. The `collect_shell_reclaim_child_dups` dup-injection is now a no-op (empty for a
            // scalar sum) — retained but never fires here.
            // Also reclaim an owned-single-view (String.at/Bytes.slice) shell the global `Owned` gate misses
            // (the local>global discipline — see `matchsum_view_shell_reclaim_ok`). Non-tail here, so no
            // back-edge: the post-match drop below covers the fall-through result. A non-recursive
            // `(match (String.at …) …)` with a borrow-clean arm reclaims its Some shell (else a per-call
            // leak); a view-consuming arm fails the payload gate and stays a defined leak.
            let reclaim_shell = sum_shell_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            ) || matchsum_view_shell_reclaim_ok(
                db,
                scrutinee,
                &scrut_ty,
                stashed_slot,
                never_diverges,
                &root,
            );
            emit_sum_cont(
                db,
                scrutinee,
                &root,
                result_it,
                block_ty,
                &arms_slots,
                arms_base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::NonTail,
            )?;
            if reclaim_shell {
                let slot = stashed_slot
                    .expect("reclaim_shell implies a stashed slot")
                    .0;
                out.push(Lir::LocalGet(slot)); // [result, shell]
                out.push(Lir::CallImport(OP_DROP)); // → [result] (reclaim the owned all-scalar-payload sum shell)
            }
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // A runtime LIST match → dispatch by LENGTH. Read `vec-len(scrutinee)` once, then a chain of
        // `if (len <cond>) then <arm-body> else …`. Each arm's element/rest binders read the list on their
        // own (`SumPayload` `Elem`/`RestFrom` → `vec-get`/`vec-split`). The scrutinee is materialized ONCE
        // into a fresh i32 slot so every arm-body binder re-reads the SAME handle. Exhaustiveness (checked
        // in `lower`) guarantees the last arm is a catch-all, so the innermost `else` runs unconditionally.
        Core::MatchList { scrutinee, arms } => {
            // A `Never` list match (all arms diverge): empty block + trailing `unreachable`. A genuinely
            // unrepresentable non-diverging result still DECLINES.
            let mut never_diverges = false;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None if body_diverges(db, id) => {
                        never_diverges = true;
                        BlockType::Empty
                    }
                    None => {
                        return Err(Reject::decline(
                            "list match result type has no machine representation",
                        ));
                    }
                },
            };
            let (arm_slots, len_slot, arm_base, owned_stash) = materialize_list_match_scrutinee(
                db, scrutinee, slots, high, scratch_ty, layout, out,
            )?;
            let reclaim = list_shell_reclaim_slot(
                db,
                scrutinee,
                &arms,
                owned_stash,
                TailPos::NonTail,
                never_diverges,
            );
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            emit_list_arms_tailable(
                db,
                &arms,
                len_slot,
                block_ty,
                result_it,
                &arm_slots,
                arm_base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::NonTail,
            )?;
            // Reclaim the owned-temporary list shell after the arms (the match's value is on the stack).
            if let Some(slot) = reclaim {
                out.push(Lir::LocalGet(slot)); // [result, shell]
                out.push(Lir::CallImport(OP_DROP)); // → [result]
            }
            if never_diverges {
                out.push(Lir::Unreachable);
            }
            Ok(())
        }
        // A parameter reference — read its local slot. The slot was assigned in `select_function`. A
        // reference to a binder with NO slot is one of: (1) a `Unit` param (elided from the signature —
        // Unit occupies no slot, so reading it pushes nothing, the read analogue of `Core::Unit`); (2) a
        // CAPTURED ENCLOSING param — a param of a DIFFERENT def than the one being emitted, reached inside
        // a LOCAL function that could not be inlined (a RECURSIVE local function that captures its
        // enclosing scope). The current function's OWN params are all slotted here, so a `Core::Param`
        // with no slot whose binder belongs to another def is definitionally such a capture; a
        // non-recursive capturing local inlines (the binding flows in) so never reaches here — only the
        // recursion+capture combination does. Lambda-lifting that case is not yet built, so decline it with
        // the coded recursive-local-capture message (CDZ0900, reject-not-miscompile — item3 interim). (3)
        // Otherwise a genuine represented-param-not-in-signature compiler bug — the plain internal decline.
        Core::Param { binder } => match slots.get(&binder) {
            Some(&slot) => {
                emit_binder_ref(id, slot, out);
                Ok(())
            }
            None if matches!(type_of(db, binder).strip_nominal(), Ty::Unit) => Ok(()),
            None if crate::infer::def_of_param(db, binder).is_some() => Err(Reject::unsupported(
                crate::diag::RECURSIVE_LOCAL_CAPTURE_DECLINE,
            )),
            None => Err(Reject::decline("parameter reference has no local slot")),
        },
        // An A-normal binding sequence: give each binding a PERSISTENT local slot (unlike the reused
        // scratch pool, a binding's value must survive across every `LocalRef` to it), emit its value
        // ONCE into that slot, then emit the body reading the slots. This is where naming pays off:
        // a value used N times is computed once here and read N times as `local.get`. Slots are
        // allocated at the current scratch floor; the floor rises past them so the body's scratch does
        // not clobber a live binding. A `let*` binding's value may reference an earlier binding, so the
        // extended slot map carries the earlier bindings when a later value is emitted.
        Core::Let { bindings, body } => {
            let mut extended = slots.clone();
            let mut floor = base;
            // Track each HEAP-typed binding as `(binder, slot, value)`, to `drop` after the body UNLESS it
            // escapes (Perceus). A kept binding is a genuine runtime value — a constant tuple folds and is
            // never kept (H2c). MOST heap bindings are OWNED allocations (a `let`-bound producer / call /
            // constructor result): released once their scope ends, or transferred out if they escape. But a
            // self-keyed row-op materialize binding `(record, record)` (from `materialize_row_op_operand`)
            // binds its OPERAND, which may be a BORROW — a `Core::SumPayload` from a `(Slot.Filled r)` match
            // arm, a `Param`, a kept `LocalRef` — owned by the scrutinee/enclosing binder, NOT here. So the
            // drop below is GATED ON OWNERSHIP (`heap_operand_ownership` at the drop site): an owned binding
            // is dropped/transferred as usual, but a BORROWED row-op operand must NOT be dropped (its owner
            // reclaims it — dropping it double-frees a still-live borrow, breaker#45 witness-2 UAF). The
            // `value` is kept in the tuple so the drop site can consult its ownership. (A scalar binding owns
            // no heap cell → no drop.)
            let mut heap_bindings: Vec<(StructId, u32, StructId)> = Vec::new();
            for (binder, value) in bindings.iter() {
                let ty = type_of(db, *binder);
                // The binding's machine value type — read off its solved type (the value's type). A
                // binding whose type has no machine rep (a compound/unresolved value) declines.
                let vt = valtype_of(&ty).ok_or_else(|| {
                    Reject::decline("a let binding's type has no machine representation")
                })?;
                // WIDTH-PARTITIONED CLAIM (func[58] emit-db slot-width collision): reuse `floor` only when
                // that slot is FREE or already RECORDED at this binding's width; otherwise take a FRESH slot
                // at `*high`. `scratch_ty` is function-scoped and persists across SIBLING match arms, which
                // each reset `floor` to `base` — so arm A's i64 binder and arm B's i32 binder both target
                // `base`, but one wasm local cannot be declared at two widths (last-writer-wins in
                // `scratch_ty` leaves arm A's `LocalSet` storing i64 into an i32-declared slot → invalid
                // module, the func[58] collision at scale). Bumping the conflicting claim to `*high` keeps
                // every slot single-width by construction. Same-width reuse is preserved (the common case —
                // no local-count growth); only a genuine width conflict spills to a fresh slot. Reads follow
                // automatically: the binder→slot mapping (`extended`) records the chosen slot.
                let slot = match scratch_ty.get(&floor) {
                    Some(&w) if w != vt => *high,
                    _ => floor,
                };
                // RESERVE the binding slot BEFORE emitting the initializer: record its type and lift `*high`
                // past it now. The initializer emits at `slot + 1`, but many emit sites float their own
                // scratch off `*high` (a tuple/record element `elem_base = *high`, an `if`-branch base, a
                // call arg) — those all ASSUME `*high >= base`. Without pre-reserving, `*high` still LAGS at
                // the pre-let value (< `slot + 1`), so an initializer whose value is an `if`/compound would
                // hand its inner scratch the binding's OWN slot: `(let ((r (if … (tuple (+ p 5) …) …)))
                // (+ (. r 0) (. r 1)))` teed the i64 `(+ p 5)` into slot `r` (declared i32, the tuple
                // handle) → invalid wasm (`expected i32, found i64`). Reserving keeps every inner scratch
                // strictly above the binding slot. (`scratch_ty` for `slot` is set here; the `LocalSet`
                // below stores into it. A `match`-producer initializer already worked because `MatchSum`
                // pre-advances `*high` for its own handle slot — this brings the general `let` in line.)
                scratch_ty.insert(slot, vt);
                *high = (*high).max(slot + 1);
                // IF-JOIN OWNERSHIP-EQUALIZE detection (v-memory-safety co-design, FIX A). When THIS binding's
                // VALUE is a `Core::If` (`let pick = (if C b …)`) and an EARLIER heap binder `b` MOVE-ALIASES
                // the result on exactly one arm (b ESCAPES that arm → pick == b, a borrow-view) while being
                // DEAD on the other (pick is OWNED-FRESH there), pick has DIVERGENT ownership. The arm-blind
                // post-body gate classifies pick `!Owned` (from the alias arm) and suppresses its drop → the
                // fresh arm's shell leaks (map-select 05:4862 +1). Equalize: `dup(b)` on the ALIAS arm so pick
                // is uniformly OWNED, then FORCE pick's post-body drop. Detection runs BEFORE the value emit so
                // the plan is in place when the `Core::If` handler emits pick's value. Uses the upfront
                // `dup_sites` so the per-arm escape verdict matches the post-body loop's.
                if is_heap_type(&ty)
                    && let Core::If { then_, else_, .. } = core_of(db, *value)
                {
                    let mut dup_plan: Vec<(u32, bool)> = Vec::new();
                    for &(pb, pslot, _pv) in &heap_bindings {
                        let esc_then = binding_escapes_dup_aware(
                            db,
                            then_,
                            EscapeTarget::Binder(pb),
                            false,
                            Some(&out.dup_sites),
                        );
                        let esc_else = binding_escapes_dup_aware(
                            db,
                            else_,
                            EscapeTarget::Binder(pb),
                            false,
                            Some(&out.dup_sites),
                        );
                        // DIVERGENT ownership iff `b` escapes exactly one arm — that arm is the ALIAS arm
                        // (pick == b there, a move-alias); dup `b` there so pick owns its own reference.
                        // `dup_is_then = esc_then` (dup on the arm b escapes).
                        if esc_then != esc_else {
                            dup_plan.push((pslot, /* dup_is_then = */ esc_then));
                        }
                    }
                    if !dup_plan.is_empty() {
                        out.ifjoin_arm_dups.insert(*value, dup_plan);
                        // pick is now uniformly OWNED after the dup → its post-body drop must fire.
                        out.ifjoin_forced_drops.insert(*binder);
                    }
                }
                // Emit the value into scratch ABOVE this persistent slot (its own scratch floats), then
                // store it once. The value sees the earlier bindings via `extended`.
                emit(
                    db,
                    *value,
                    &extended,
                    slot + 1,
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(Lir::LocalSet(slot));
                if is_heap_type(&ty) {
                    heap_bindings.push((*binder, slot, *value));
                }
                // DEBUG (D3 locals): a SCALAR binding with a source name lives in this slot for its whole
                // scope — record it so a `DW_TAG_variable` DIE lets a debugger `print` the local. (A heap
                // binding is a handle DWARF can't walk (§3), so only scalars are recorded.) The binder key
                // is the initializer occurrence, so recover the name from its `(name init)` pair.
                if matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
                    && let Some(name) = db.let_binding_name(*binder)
                {
                    out.binding_local(slot, name.to_string(), ty.clone());
                }
                extended.insert(*binder, slot);
                // ALSO map the VALUE node → this slot, for a SCALAR binding. A closure that captures a
                // let-bound value records the capture as the VALUE node itself (`collect_captures` keys the
                // capture by the binding's value occurrence, NOT a `LocalRef` to the binder), so the closure
                // build-site's `emit(cap)` would RE-LOWER the value — a SECOND host call for a `(let ((v
                // (io.get))) …)` init captured by ≥2 escaping closures (adv-62: the host op fired once per
                // capturing closure → the extra call had no recorded response and TRAPPED, a soundness bug;
                // the rust backend fixed the same double-emit at expr.rs's `Core::Let`/`Core::Closure` arms).
                // The node→slot fast path at the top of `emit` reads this: a capture of `*value` now emits
                // `local.get slot` (the value computed ONCE at the `let`) instead of re-running the init.
                // SCALAR ONLY: a scalar slot holds the value directly (a `local.get` is a faithful re-read),
                // and a scalar host-result is the confirmed miscompile domain. A HEAP binding is EXCLUDED —
                // its slot holds a refcounted handle whose Perceus dup/drop accounting is per-OCCURRENCE
                // (`dup_sites`/`binding_escapes_dup_aware`), so aliasing the value node to the slot could
                // skew that bookkeeping; a heap value captured by a closure declines today (CDZ0201) anyway,
                // so it is not a live miscompile. Insert only when the slot is not already a node-key (a
                // materialized scrutinee), so this never shadows an existing fast-path entry.
                if !is_heap_type(&ty)
                    && matches!(ty.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_))
                {
                    extended.entry(*value).or_insert(slot);
                }
                // The body emits ABOVE both this binding slot AND any scratch the INITIALIZER used (its
                // transient slots are recorded in `scratch_ty` at a fixed TYPE; a body reusing one at a
                // different type would re-type a wasm local → invalid module — e.g. a runtime-`(bin …)`
                // scrutinee initializer uses an i64 `val` slot, and the match body reuses it as an i32).
                // `*high` tracks the top slot touched so far. For a scalar/handle initializer with no
                // scratch, `*high == slot+1`, so this is byte-identical to before.
                floor = (slot + 1).max(*high);
            }
            // The body computes its value (left on the stack). A heap binding is RECLAIMED
            // (`local.get <slot>; drop`) only if it is DEAD after the body — used solely in BORROWING
            // positions (a `Core::Proj` operand: `arr-get` borrows). A binding that ESCAPES — its
            // reference flows into the RETURNED value: it IS the result (`(let ((t …)) t)`), or an
            // element of a constructed tuple (`arr-set` CONSUMES it), or a call argument — is NOT
            // dropped: its ownership transfers to the caller (the ownership-transfer-on-return rule of
            // Perceus — `value-heap-runtime.md` §the ordering obligation). Dropping an escaped value
            // would be a use-after-free / double-free; `binding_escapes` is conservative (any non-borrow
            // use → escapes → keep), so we never drop a live value.
            // IF-JOIN PER-ARM DROP detection (v-memory-safety co-design). When the let BODY is a `Core::If`,
            // a heap binding that DIVERGES across its arms — ESCAPES on one arm (W: carried-whole / is the
            // result) but is DEAD on the other (D) — leaks on the D arm: the post-body scope-drop below sees
            // `binding_escapes_dup_aware(WHOLE body) == true` (it escapes on the W arm) and SUPPRESSES its
            // drop, but nothing reclaims it on the D path. Record the divergent binder's slot + which arm is
            // D so the `Core::If` handler drops it PER-ARM (on the D arm only — never the W arm, the UAF bar)
            // before that arm's join value. Uses the UPFRONT `dup_sites` (populated at function entry), so
            // the per-arm escape verdict matches the post-body loop's whole-body one. Fires ONLY on a
            // genuine divergence (W xor D); uniform-D is handled by the post-body drop, uniform-W kept live.
            if let Core::If { then_, else_, .. } = core_of(db, body) {
                let mut plan: Vec<(u32, bool)> = Vec::new();
                for &(binder, slot, _value) in &heap_bindings {
                    let esc_then = binding_escapes_dup_aware(
                        db,
                        then_,
                        EscapeTarget::Binder(binder),
                        false,
                        Some(&out.dup_sites),
                    );
                    let esc_else = binding_escapes_dup_aware(
                        db,
                        else_,
                        EscapeTarget::Binder(binder),
                        false,
                        Some(&out.dup_sites),
                    );
                    // DIVERGENT iff it escapes exactly one arm; the D (dead) arm is the one it does NOT
                    // escape → drop there.
                    if esc_then != esc_else {
                        plan.push((slot, /* d_is_then = */ !esc_then));
                    }
                }
                if !plan.is_empty() {
                    out.ifjoin_arm_drops.insert(body, plan);
                }
            }
            emit(db, body, &extended, floor, high, scratch_ty, layout, out)?;
            // DROP a dead heap binding. DUP-AWARE escape: a CONSUMING occurrence that is a Perceus retain
            // (`dup_sites`) does NOT count as an escape — the `dup` gave the consuming op its own reference,
            // so the binding's slot reference survives and MUST be reclaimed here (else the leak: a captured-
            // then-inlined `xs` dup'd for `List.push xs` beside `List.len xs` was never dropped). Sound: a
            // LAST consume (`live_after=false`) is NOT a dup_site → still escapes → suppresses the drop, so a
            // multi-consume binding is never double-freed. `dup_sites` is cloned out of `out` so the drop's
            // `out.push` can borrow `out` mutably in the same loop.
            let dup_sites = out.dup_sites.clone();
            // A binding's index in `bindings`, so its drop check can also inspect LATER sibling initializers
            // (the D1 flat-multi-binding-let over-drop, v-memory-safety-aligned drop-elide).
            let binder_index: HashMap<StructId, usize> = bindings
                .iter()
                .enumerate()
                .map(|(i, (b, _))| (*b, i))
                .collect();
            for &(binder, slot, value) in &heap_bindings {
                // ESCAPES THE BODY → ownership transfers to the caller (it IS the result / a constructed
                // element / a call arg) → do NOT drop (the ownership-transfer-on-return rule).
                if binding_escapes_dup_aware(
                    db,
                    body,
                    EscapeTarget::Binder(binder),
                    false,
                    Some(&dup_sites),
                ) {
                    continue;
                }
                // DROP-ELIDE for a binder CONSUMED INTO A LATER SIBLING INITIALIZER (D1, the concat-child-of-
                // keep over-drop). The body-only escape check above misses this: the optimizer copy-prop-SINKS
                // single-use siblings into ONE flat `let` — `(let ((a X) (b (String.concat a y))) …)` moves
                // `a` into `b`'s concat CHILD, so `a` is absent from the body yet already consumed. Its own
                // scope drop then frees `a` a SECOND time (once as `b`'s child when `b` drops, once here) → the
                // D1 double-free. Elide it: `b` subsumes `a`, `b`'s drop is the sole reclaim. GUARD = PURE MOVE
                // ONLY (`!binder_has_dup_site_in` over the later inits): if `a` was DUP'd there — the 980 shape,
                // a then-arm MOVE-OUT plus an else-arm concat that dup'd `a` so its slot SURVIVES — the existing
                // drop is its sole reclaim and eliding it LEAKS (980 mode1 0→1). A pure move (no dup) is fully
                // subsumed; a dup means the slot lives → keep the drop. Sound: only ever REMOVES a drop for a
                // provably-moved binder → converts the current double-free to at-worst a benign leak on a
                // conditional non-consuming path, never the reverse. Nested one-binding lets never reach here
                // (the sibling init lives inside the outer body); only a flat multi-binding let does.
                let idx = binder_index[&binder];
                let later_inits = || bindings.iter().skip(idx + 1).map(|(_, v)| *v);
                let escapes_later_init = later_inits().any(|v| {
                    binding_escapes_dup_aware(
                        db,
                        v,
                        EscapeTarget::Binder(binder),
                        false,
                        Some(&dup_sites),
                    )
                });
                // Elide ONLY a PURE FRESH-ALLOC-CHILD consume: on NO path may the binder be moved out or
                // reused-in-place as the base (that keeps its slot alive → the drop is its reclaim: LIST
                // `push`, the 980 then-arm move-out), and it must have NO dup_site there (a dup keeps the
                // slot alive too). Then the later binding fully subsumes it (concat child) → its own drop
                // double-frees → elide. Otherwise KEEP the drop.
                let reuses_or_moves =
                    later_inits().any(|v| binder_reuses_or_moves_on_some_path(db, v, binder));
                let dup_in_later_init =
                    later_inits().any(|v| binder_has_dup_site_in(db, v, binder, &dup_sites));
                if escapes_later_init && !reuses_or_moves && !dup_in_later_init {
                    continue;
                }
                // BORROWED-OPERAND materialize (breaker #45 witness-2 UAF): a self-keyed materialize
                // binding `(record, record)` (the row-op operand `materialize_row_op_operand` emits) whose
                // bound VALUE is a BORROW — a `Core::SumPayload` from a `(Slot.Filled r)` match arm, a
                // `Param`, or a kept `LocalRef` — is NOT owned here; the scrutinee/owner reclaims it. Dropping
                // it (`local.get slot; drop`) frees a handle the owner still references → the shared record is
                // freed while a live borrow (a re-projection `(. r qty)` off the still-live `Some`/scrutinee)
                // reads it → UAF (silent wrong value / oob trap). The general "every kept heap binding is an
                // owned allocation" assumption (H2c) holds for a genuine `let`-bound producer, but a row-op
                // materialize binds its OPERAND, which may be a borrow. Gate the drop on ownership: only a
                // genuinely OWNED value (a constructor / call / producer result — the W1 owned-map-lookup
                // path, which STILL drops + relies on the field-dup) is reclaimed here; a borrowed operand is
                // materialized-once for the eval-once benefit but left for its owner to reclaim. Non-self-keyed
                // bindings are unaffected (a normal `let` binds a fresh value, always Owned or escaping).
                // IF-JOIN OWNERSHIP-EQUALIZE (FIX A): a binder whose divergent-ownership value-If was
                // equalized by an arm dup above is now genuinely OWNED on BOTH arms — BYPASS this arm-blind
                // borrowed-operand skip for it so its post-body drop fires (its fresh-arm shell would else
                // leak). The bypass is scoped to exactly these binders (never the genuine self-keyed row-op
                // materialize-borrow the gate protects, breaker #45); and the earlier `escapes_body`/D1 gates
                // still ran first, so a forced binder that genuinely escapes is already (correctly) not here.
                if binder == value
                    && !out.ifjoin_forced_drops.contains(&binder)
                    && !matches!(
                        heap_operand_ownership(db, value),
                        Ok(HandleOwnership::Owned)
                    )
                {
                    continue;
                }
                out.push(Lir::LocalGet(slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(())
        }
        // A reference to a kept `let` binding — read its persistent slot, exactly like a parameter. The
        // slot was assigned when the enclosing `Core::Let` was emitted; a `LocalRef` with no slot is a
        // compiler bug (a ref lowered as `LocalRef` without its binding kept), so decline.
        Core::LocalRef { binder } => match slots.get(&binder) {
            Some(&slot) => {
                emit_binder_ref(id, slot, out);
                Ok(())
            }
            None => Err(Reject::decline("let-binding reference has no local slot")),
        },
        // A runtime CALL — push each argument (in the CALLER's frame, so an arg's own scratch floats
        // above `base`), then `call` the callee's absolute wasm-function index (its position in the
        // layout's emission order). The callee is reachable (`layout` added it), so its index exists; a
        // callee not in the emission order is a compiler bug (reachability missed it) → decline.
        Core::Call { callee, args } => {
            let mut drop_slots: Vec<u32> = Vec::new();
            emit_call_args(
                db,
                callee,
                &args,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
                Some(&mut drop_slots),
            )?;
            // OPTION C (consumer emit): a CROSS-EDGE callee is NOT a local emitted func — it lives in the
            // shared-closure PROVIDER component, imported through the peer interface. Emit a
            // `CallExternImport` to its import position (the same slot the provider exports it at — the
            // index-agreement `compute_tests_consumer` guarantees) instead of a local `Call`. The map is
            // EMPTY for every non-consumer layout, so this branch never fires there (byte-identical). The
            // args were already pushed above (identical to a local call — a cross-edge is an ordinary call
            // in the Core; only the target differs). Mirrors the peer-bound `Core::HostCall` extern path.
            if let Some(&pos) = layout.cross_edge_import.get(&callee) {
                trace!(target: "rcdzc::select", callee, pos, args = args.len(), "emit cross-edge extern call");
                out.push(Lir::CallExternImport(pos));
            } else {
                match layout.abs(callee) {
                    Some(idx) => {
                        trace!(target: "rcdzc::select", callee, idx, args = args.len(), "emit runtime call");
                        out.push(Lir::Call(idx));
                    }
                    None => {
                        return Err(Reject::decline(
                            "a called function is not in the emission order (reachability gap)",
                        ));
                    }
                }
            }
            // Caller-side reclaim: `drop` each owned-temporary arg the borrowing (boundary-owned, non-looped)
            // callee left live. Result is on TOS; each `local.get slot ; drop` pushes+drops below it.
            for slot in drop_slots {
                out.push(Lir::LocalGet(slot));
                out.push(Lir::CallImport(OP_DROP));
            }
            Ok(())
        }
        // A runtime comparison: emit both operands, then the machine comparison selected from the
        // operands' width AND signedness (`_s` for a signed type, `_u` for an unsigned one; `eq` is
        // sign-agnostic). The result is an i32 boolean. A comparison never overflows, so both operands
        // share the same scratch base (each is dead once pushed).
        Core::Compare { op, lhs, rhs } => {
            // FLOW-SENSITIVE COMPARISON FOLD: when the enclosing branch has REFINED a variable operand's
            // range so this comparison against a constant is already decided (`(if (> n 0) (if (> n 0) …)
            // …)` — the inner test is known true; or an IMPLIED test `(if (>= n 5) (if (> n 0) …))`), emit
            // the constant bool directly instead of recomputing the compare. Refinements live only during
            // emit, so `lower`'s `fold_comparison_at_type_bound` could not see this; the runtime operand is
            // a refined `Param`/`LocalRef` (trap-free), so discarding it drops no trap. Result is an i32
            // bool (`0`/`1`), exactly what a comparison leaves.
            if let Some(r) = crate::lower::refined_comparison_const(db, op, lhs, rhs) {
                trace!(target: "rcdzc::select", node = id.0, result = r, "comparison folded to a constant by the active branch refinement");
                out.push(Lir::ConstI32(r as i32));
                return Ok(());
            }
            // Both operands must share the comparison's machine slot; ground a bare-literal operand to
            // the shared width so `(> x 50)` over a narrow `x` does not push an i64 beside `x`'s i32.
            let it = operand_int_ty(db, lhs, rhs);
            // EQUALITY-WITH-ZERO → `eqz`. `(= x 0)` (or `(= 0 x)`) is the shape of every recursion base
            // case `(if (= n 0) …)`; `x == 0` is exactly wasm's `i32.eqz`/`i64.eqz` (1 if the operand is
            // zero) — one instruction instead of pushing a `0` constant and an `eq` (three). This is
            // INSTRUCTION SELECTION at the site where the semantics ("compare with zero") are known — not
            // a byte peephole re-recognizing `const 0 ; eq` after the fact. A CONSTANT-vs-constant `= 0`
            // would already have folded in `lower`, so here the non-zero operand is a runtime value.
            if op == Prim::Eq
                && let Some(nonzero) = eq_zero_operand(db, lhs, rhs)
            {
                // DIVISIBILITY-BY-POWER-OF-TWO: `(= (% x 2^k) 0)` — the "is x divisible by 2^k?" test (the
                // even test `(= (% x 2) 0)` is the k=1 case) — is exactly `(x & (2^k − 1)) == 0`, for BOTH
                // signednesses (a number is divisible by 2^k iff its low k bits are zero, regardless of
                // sign). Emitting `x & mask ; eqz` skips the whole `%` — for a SIGNED `%` that is the ~10
                // instruction round-toward-zero bias sequence (`emit_div_rem`), collapsed to 3. Verified
                // `(x % 2^k == 0) ≡ (x & (2^k−1) == 0)` exhaustively over signed inputs and every k. Only
                // fires when the divisor is a compile-time power of two > 1 (`k=1` divisor `2` even test;
                // `%1` folds to 0 in `lower`, never reaching here).
                if let Some((x, mask)) = rem_pow2_mask(db, nonzero) {
                    emit_operand(db, x, it, slots, base, high, scratch_ty, layout, out)?;
                    out.push(if it.ground_width() <= 32 {
                        Lir::ConstI32(mask as i32)
                    } else {
                        Lir::ConstI64(mask)
                    });
                    out.push(if it.ground_width() <= 32 {
                        Lir::I32And
                    } else {
                        Lir::I64And
                    });
                    out.push(if it.ground_width() <= 32 {
                        Lir::I32Eqz
                    } else {
                        Lir::I64Eqz
                    });
                    return Ok(());
                }
                emit_operand(db, nonzero, it, slots, base, high, scratch_ty, layout, out)?;
                out.push(if it.ground_width() <= 32 {
                    Lir::I32Eqz
                } else {
                    Lir::I64Eqz
                });
                return Ok(());
            }
            // Both operands are simultaneously live on the stack for the compare, so — like sibling call
            // args and checked-arith's A/B — the RHS emits ABOVE the LHS's high-water, never reusing an
            // LHS scratch slot at a different width. An LHS that inlines a heap-match (`(= (cbor-major b
            // i) 4)` — `cbor-major` β-reduces to a `Bytes.at` MatchSum materializing an i32 handle) types
            // its slots i32; an RHS (or the LHS of a sibling compare) reusing them as an i64 arith temp
            // would re-type one wasm local to two widths → an invalid module. Floating the RHS above
            // `*high` hands it fresh, never-typed slots.
            emit_operand(db, lhs, it, slots, base, high, scratch_ty, layout, out)?;
            let rhs_base = base.max(*high);
            emit_operand(db, rhs, it, slots, rhs_base, high, scratch_ty, layout, out)?;
            out.push(compare_op(op, it));
            Ok(())
        }
        // RUNTIME FLOAT EQUALITY under the CANONICAL BYTE FORM — `nan == nan` TRUE, `-0.0 != +0.0`, all
        // NaN equal (core-semantics.md §Floating-Point Equality Follows The Canonical Byte Form). NOT IEEE
        // `f64.eq` (which says `nan != nan` and `-0.0 == 0.0` — a miscompile). Each operand is
        // CANONICALIZED to its integer bit pattern with NaN folded to one canonical form
        // (`canon(x) = select(x != x, CANON_NAN_BITS, reinterpret_int(x))`), then the two canonical bit
        // patterns compare with an INTEGER `eq` — so any two NaNs compare equal and the sign bit of a zero
        // is significant. `x != x` is the isnan test (true only for NaN); a bare f64 param can carry a
        // non-canonical NaN across the host boundary, so the canonicalize is load-bearing (it matches
        // `op_box_float`'s runtime canonicalization and the constant fold's `to_f64_bits` basis).
        Core::FloatCompare {
            op,
            lhs,
            rhs,
            width,
        } => {
            // DISJOINT-SLOT DISCIPLINE (the sibling-scratch collision class, as `Core::MapRemove`/`Tuple`):
            // the LHS float result is left PENDING ON THE STACK while the RHS is emitted, so the RHS's
            // operand scratch must FLOAT above the LHS's high-water — NOT reuse the shared `base`. Concretely
            // `(= (Float64.of-int n) (Float64.of-int (+ n 1)))`: the LHS `Float64.of-int n` leaves its
            // canonical f64 bits pending; the RHS's `(+ n 1)` is a CHECKED i64 add whose overflow-guard temp,
            // if laid at the same `base`, re-types a wasm local the LHS emit already fixed at f64/i64 → an
            // invalid module (`function[0]` fails to compile), order-specific (only param-left/arith-right —
            // arith-left's i64 scratch is consumed before anything is pending). Advancing the RHS base past
            // `*high` gives the two operands disjoint scratch. (Both the FEq canon-bits path and the ordering
            // path share this structure; fixing both closes the whole `Core::FloatCompare` operand-pair class.)
            if op == Prim::FEq {
                // EQUALITY: canonical-byte bit compare (see above).
                emit_canon_float_bits(db, lhs, width, slots, base, high, scratch_ty, layout, out)?;
                let rhs_base = base.max(*high);
                emit_canon_float_bits(
                    db, rhs, width, slots, rhs_base, high, scratch_ty, layout, out,
                )?;
                out.push(if width == 32 { Lir::I32Eq } else { Lir::I64Eq });
            } else {
                // ORDERING (`< <= > >=`): the RAW IEEE float compare (operator ruling — IEEE partialOrd).
                // `f64.lt`/etc. already give the wanted semantics: a NaN operand → 0 (unordered → false),
                // and `-0.0`/`+0.0` compare EQUAL (`f64.le -0.0 0.0` = 1). NO canonicalization — that's the
                // equality path; ordering DISAGREES with it on NaN + signed zero, by design. Emit each float
                // operand directly (grounded to the op width), then the raw compare op.
                emit_float_operand(db, lhs, width, slots, base, high, scratch_ty, layout, out)?;
                let rhs_base = base.max(*high);
                emit_float_operand(
                    db, rhs, width, slots, rhs_base, high, scratch_ty, layout, out,
                )?;
                out.push(float_ordering_op(op, width));
            }
            Ok(())
        }
        // RUNTIME STRUCTURAL EQUALITY on two COMPOUND heap values — a `value-eq` (`champ_eq`) call. The
        // op BORROWS both operands, so refcounts are balanced by DROPPING each OWNED-temporary operand
        // after the compare (a bare reference — a parameter/binding the owner reclaims — is NOT dropped).
        // Each operand is emitted, `tee`d into a scratch slot (kept on the stack for the call AND
        // remembered for a possible drop), then `value-eq` pops both and pushes the i32 bool; a drop of a
        // remembered owned handle leaves that bool on top. An operand whose ownership cannot be proved
        // (an `if`/`match`/`let` that may return a borrowed sub-value) DECLINES — reject, never a leak or
        // a double-free.
        // Canonicalizing every rope operand before the physical `champ_eq` is what makes a value that
        // SHARES another's storage (a `String.concat`/`Bytes.concat` rope over shared segments) and a value
        // that COPIES it (a flat leaf of the same content) compare EQUAL — indistinguishable by equality and
        // by the canonical byte form, exactly as the memory model requires. (The float leaf's twin of this
        // is `op_box_float`'s canonicalize-on-construct; here it is the rope compaction.)
        //= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
        //# A value that shares another value's storage and a value that copies it MUST be indistinguishable by every operation the executable semantics defines, including equality, length, indexing, and the value's canonical byte form, so that whether storage is shared is never observable.
        Core::ValueEq { lhs, rhs } => {
            // `value-eq` is `champ_eq` — a PHYSICAL-byte compare (the map-key contract). A runtime String OR
            // Bytes operand can be a ROPE (a `String.concat`/`Bytes.concat`/`.slice` lowers to a rope node),
            // whose bytes differ from a flat leaf of IDENTICAL content — so comparing a rope directly would
            // return the WRONG answer. CANONICALIZE EVERY String/Bytes operand with `bytes-compact` (a
            // content-equal flat leaf; a no-op-shaped pass on an already-flat value) before the compare, so a
            // rope and its flat twin compare equal — whether the operand is OWNED (a fresh `String.concat`/
            // `Bytes.concat`/`String.to-bytes` result) or BORROWED (a param / a `Map.lookup`/`SumPayload`-
            // extracted rope value, `(= s "…")` where `s` is a variant/map payload). `bytes-compact` FLATTENS
            // its argument IN PLACE and returns the SAME handle with an UNCHANGED refcount when it was already
            // owned-consuming — but critically, the runtime op is refcount-NEUTRAL (`op_bytes_compact` =
            // `bytes_flatten(buf); buf`): it mutates the node into a leaf (content-preserving, UNOBSERVABLE
            // even on a shared value per the memory model's #Sharing Is Not Observable) and hands the SAME
            // handle back. So compacting a BORROWED operand neither consumes it nor mints a new handle — the
            // borrow stays the owner's and is NOT dropped here; an OWNED operand's handle is likewise threaded
            // through and dropped after the borrowing compare. A non-text operand (a tuple/sum/map compound
            // handle) is NOT compacted — it is passed as-is (a String/Bytes NESTED inside a compound rides the
            // construction-site element compaction; only a DIRECT String/Bytes operand is canonicalized here,
            // which is why `compound_eq_heap_walkable` admits a DIRECT Bytes but `ty_heap_walkable` still
            // declines a nested one).
            let lo = heap_operand_ownership(db, lhs)?;
            let ro = heap_operand_ownership(db, rhs)?;
            // Compact ANY String/Bytes operand — owned OR borrowed. Since `bytes-compact` is refcount-neutral
            // (it flattens in place, returning the same handle), it is safe on a borrow: the flatten is
            // unobservable, and no drop follows a borrowed operand. This closes the rope miscompile for the
            // WHOLE class — previously only an OWNED String was compacted, so a genuine rope reaching `=`
            // through a BORROWED operand (a `Map.lookup`/`SumPayload` payload, or a runtime-rope param)
            // compared by its unflattened header bytes and silently returned the wrong answer.
            let lhs_str = operand_is_string_or_bytes(db, lhs);
            let rhs_str = operand_is_string_or_bytes(db, rhs);
            // Two i32 scratch slots for the operand handles, above the running high-water (they must not
            // clash with an operand emit's own transient scratch — a `Call` arg's i64 guard slot).
            let slot_l = *high;
            let slot_r = *high + 1;
            *high = slot_r + 1;
            scratch_ty.insert(slot_l, ValType::I32);
            scratch_ty.insert(slot_r, ValType::I32);
            let op_base = *high;
            emit(db, lhs, slots, op_base, high, scratch_ty, layout, out)?;
            // `bytes-compact` flattens the operand IN PLACE and returns the SAME handle (refcount-neutral),
            // so it is applied uniformly to an owned OR a borrowed String — the borrow is not consumed and
            // the returned handle carries the operand's original ownership through to the drop decision.
            if lhs_str {
                out.push(Lir::CallImport(OP_BYTES_COMPACT)); // rope/flat → canonical flat leaf (in place)
            }
            out.push(Lir::LocalTee(slot_l));
            emit(db, rhs, slots, op_base, high, scratch_ty, layout, out)?;
            if rhs_str {
                out.push(Lir::CallImport(OP_BYTES_COMPACT));
            }
            out.push(Lir::LocalTee(slot_r));
            out.push(Lir::CallImport(OP_VALUE_EQ)); // pops both handles (borrowed) → [bool]
            // Drop each operand handle the compare borrowed but we OWN. Ownership is unchanged by the
            // compaction (`bytes-compact` returns the same handle), so drop iff the operand was OWNED to
            // begin with — an OWNED temporary (a constructor / call / concat result) leaks otherwise; a
            // BORROWED operand (param / kept-local / payload read) is left to its owner (dropping it would
            // be a double-free), whether or not it was compacted in place.
            lo.drop_slot_if_owned(slot_l, out);
            ro.drop_slot_if_owned(slot_r, out);
            Ok(())
        }
        // RUNTIME COMPOUND ORDERING — a `value-cmp(a, b, desc)` call: the blessed three-way lexicographic
        // walk over two compound heap values (core-semantics.md §Compound Ordering Is Lexicographic). Bake
        // the operands' shape descriptor as a Bytes constant (the same discipline as `Set.to-list`), emit
        // both operands (owned-temporaries dropped after the borrowing compare, like `ValueEq`), call
        // `value-cmp` → i32 in {-1,0,1} (2 = unordered, never reached — the compiler declines a non-orderable
        // type at lower time), then map the three-way result to the bool `op` wants (`res <ₛ 0` for `<`, etc.).
        Core::ValueCmp { op, lhs, rhs, ty } => {
            let Some(desc) = crate::lower::value_cmp_shape_descriptor(db, &ty) else {
                return Err(Reject::decline(
                    "runtime compound ordering: operand shape has no orderable descriptor",
                ));
            };
            let lo = heap_operand_ownership(db, lhs)?;
            let ro = heap_operand_ownership(db, rhs)?;
            // Scratch slots (above the running high-water): the two operand handles (dropped if owned) + the
            // descriptor Bytes (a borrowed-only owned temporary, dropped after the compare — as `Set.to-list`).
            let slot_l = *high;
            let slot_r = *high + 1;
            let desc_slot = *high + 2;
            *high = desc_slot + 1;
            for s in [slot_l, slot_r, desc_slot] {
                scratch_ty.insert(s, ValType::I32);
            }
            let op_base = *high;
            // BAKE THE DESCRIPTOR FIRST — into desc_slot, with NOTHING else on the stack — so the operand
            // emits (which build tuples/lists via their own arr/vec scratch) never sit under a partially-baked
            // descriptor. (An earlier order that baked between the tee'd operands produced invalid wasm.) The
            // bake leaves the buffer on the stack; store it to desc_slot (and DROP the value the ConstI32/
            // bytes-alloc left — no, LocalSet consumes it), clearing the stack for the operands.
            out.push(Lir::ConstI32(desc.len() as i32));
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32));
                out.push(Lir::ConstI32(byte as i32));
                out.push(Lir::CallImport(OP_BYTES_SET)); // [desc-buf] (bytes-set returns the buffer)
            }
            out.push(Lir::LocalSet(desc_slot)); // [] — descriptor stored, stack clear
            // a, b — emitted onto the clean stack, tee'd into slots (kept for the call AND a possible drop).
            emit(db, lhs, slots, op_base, high, scratch_ty, layout, out)?;
            out.push(Lir::LocalTee(slot_l)); // [a]
            emit(db, rhs, slots, op_base, high, scratch_ty, layout, out)?;
            out.push(Lir::LocalTee(slot_r)); // [a, b]
            out.push(Lir::LocalGet(desc_slot)); // [a, b, desc]
            out.push(Lir::CallImport(OP_VALUE_CMP)); // → [res:i32 ∈ {-1,0,1}]
            // Drop the borrowed-only descriptor Bytes (a fresh owned temporary), then any owned operand.
            out.push(Lir::LocalGet(desc_slot));
            out.push(Lir::CallImport(OP_DROP));
            lo.drop_slot_if_owned(slot_l, out);
            ro.drop_slot_if_owned(slot_r, out);
            // Map the three-way result `res ∈ {-1,0,1}` (still on top) to what `op` wants:
            //   - Lt/Le/Gt/Ge: the BOOLEAN it names — Lt res<0, Le res<=0, Gt res>0, Ge res>=0 (signed
            //     compare against 0).
            //   - Compare: the three-way `Ordering` SUM directly (§331 — the boolean ops and the three-way
            //     `compare` surface the SAME total order). `Ordering` is an ALL-NULLARY enum (Less=disc 0,
            //     Equal=disc 1, Greater=disc 2) with a bare i32 discriminant rep (no payload → no `sum-new`
            //     box, no `ordering_discs` lookup — the discs are prelude-fixed), so the Ordering value is
            //     exactly `res + 1` (res=-1→0=Less, 0→1=Equal, 1→2=Greater). Paired with the `lower_compare`
            //     routing that emits `Core::ValueCmp{op: Prim::Compare}` for a runtime orderable compound.
            match op {
                Prim::Compare => {
                    out.push(Lir::ConstI32(1));
                    out.push(Lir::I32Add); // res + 1 = the Ordering discriminant (Less/Equal/Greater)
                }
                Prim::Lt => {
                    out.push(Lir::ConstI32(0));
                    out.push(Lir::I32LtS);
                }
                Prim::Le => {
                    out.push(Lir::ConstI32(0));
                    out.push(Lir::I32LeS);
                }
                Prim::Gt => {
                    out.push(Lir::ConstI32(0));
                    out.push(Lir::I32GtS);
                }
                Prim::Ge => {
                    out.push(Lir::ConstI32(0));
                    out.push(Lir::I32GeS);
                }
                Prim::Eq => {
                    // EQUALITY via the value-cmp walk: equal iff the three-way result is 0. `res == 0` =
                    // `i32.eqz`. Used for a runtime LIST(-containing) `=`, where `value-eq`'s champ_eq byte
                    // walk is unsound (an RRB list is element- but NOT shape-canonical) — the descriptor-guided
                    // value-cmp walk compares element-wise, so `res==0` is exact structural equality
                    // (collections §"Two lists ... equal ... independent of how each was constructed").
                    out.push(Lir::I32Eqz);
                }
                _ => {
                    return Err(Reject::decline("ValueCmp carries a non-ordering prim"));
                }
            }
            Ok(())
        }
        // RUNTIME DESCRIPTOR-GUIDED STRUCTURAL EQUALITY — a `value-eq-shaped(a, b, desc)` call: the element-
        // wise structural-equality walk over two compound heap values, exact for a LIST(-containing) compound
        // with a FLOAT/BYTES leaf (the physical `value-eq` byte-walk is unsound for an RRB list; `value-cmp`
        // is unavailable — a float has no total ORDER). Same emit SHAPE as `ValueCmp` (bake the descriptor
        // Bytes, emit both borrowed operands, drop the descriptor + any owned temporary after the compare),
        // but the call returns a `bool` DIRECTLY — no three-way→boolean mapping. Reuses the SAME bare-rooted
        // descriptor `value-cmp`/`value-encode` bake (`value_eq_shaped` resolves the root and walks it).
        Core::ValueEqShaped { lhs, rhs, ty } => {
            let Some(desc) = crate::lower::value_cmp_shape_descriptor(db, &ty) else {
                return Err(Reject::decline(
                    "runtime structural equality: operand shape has no descriptor",
                ));
            };
            let lo = heap_operand_ownership(db, lhs)?;
            let ro = heap_operand_ownership(db, rhs)?;
            let slot_l = *high;
            let slot_r = *high + 1;
            let desc_slot = *high + 2;
            *high = desc_slot + 1;
            for s in [slot_l, slot_r, desc_slot] {
                scratch_ty.insert(s, ValType::I32);
            }
            let op_base = *high;
            // Bake the descriptor into desc_slot FIRST (clean stack for the operand emits), exactly as ValueCmp.
            out.push(Lir::ConstI32(desc.len() as i32));
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32));
                out.push(Lir::ConstI32(byte as i32));
                out.push(Lir::CallImport(OP_BYTES_SET)); // [desc-buf]
            }
            out.push(Lir::LocalSet(desc_slot)); // [] — descriptor stored, stack clear
            emit(db, lhs, slots, op_base, high, scratch_ty, layout, out)?;
            out.push(Lir::LocalTee(slot_l)); // [a]
            emit(db, rhs, slots, op_base, high, scratch_ty, layout, out)?;
            out.push(Lir::LocalTee(slot_r)); // [a, b]
            out.push(Lir::LocalGet(desc_slot)); // [a, b, desc]
            out.push(Lir::CallImport(OP_VALUE_EQ_SHAPED)); // → [res:bool]
            // Drop the borrowed-only descriptor Bytes, then any owned operand (like ValueCmp/ValueEq).
            out.push(Lir::LocalGet(desc_slot));
            out.push(Lir::CallImport(OP_DROP));
            lo.drop_slot_if_owned(slot_l, out);
            ro.drop_slot_if_owned(slot_r, out);
            // The bool result is already on top — no mapping needed.
            Ok(())
        }
        // `Value.encode v` (R2) — render the runtime value `v` to its canonical binary-AST document via
        // `value-encode(v, desc)`, guided by the compiler-baked shape `desc`. Same emit SHAPE as `ValueCmp`
        // (bake the descriptor Bytes into a slot on a clean stack, emit the borrowed operand, drop the
        // descriptor + any owned temporary after the borrowing call). UNLIKE the resource-escape path it does
        // NOT copy the doc into the export retarea: the fresh owned doc handle IS the `Bytes` value, left on
        // the stack. `value-encode` BORROWS `v` (an inspector) so an owned-temporary `v` is dropped after.
        Core::ValueEncode { value, desc } => {
            // The runtime `value-encode(v, desc)` op walks an i32 HEAP HANDLE `v` guided by the descriptor.
            // A value whose machine rep is already a HANDLE (a record/tuple/sum/collection/bignum/bytes/…)
            // feeds it directly. A value whose type is descriptor-eligible but whose machine rep ERASES TO A
            // BARE SCALAR — a single-FIELD single-ctor newtype over a scalar, e.g. `type Envelope =
            // | FireAfter(UInt64)` erases to `Ty::Nominal{inner:UInt64}` = a bare i64 — is NOT a handle. Its
            // descriptor is `Named(TypeName, <scalar-shape>)` (the `(: <value> Type)` frame), so the runtime
            // `Named` arm just FRAMES the inner scalar leaf — there is NO sum node/disc to read. So we BOX the
            // erased scalar into a leaf handle first (`box-int`/`box-bool`/`box-float`/`box-float32`, with the
            // narrow-int i32→i64 extend), then feed THAT leaf to `value-encode`; the canonical form is the
            // elided-head `(: N Envelope)` (rep-independent — the ctor identity rides in the descriptor, not
            // the document — matching the single-ctor ELISION of the compiler's canonical value form).
            // `box_op_ty` discriminates the three reps: `Some(op)` = an erased SCALAR (box it); `None` with an
            // i32 rep = an already-built HANDLE (feed directly); `None` with NO rep = `Unit` (a NULLARY
            // single-ctor, `type Ack = | Ack` → `Nominal{inner:Unit}`, no machine value). The Unit case FRAMES
            // the runtime's inline-unit handle: its descriptor is `Named(Type, Unit)` and the runtime `Named`
            // → `Unit` arm renders the bare `unit` atom, so `value-encode(IMM_UNIT, desc)` yields the elided-
            // head `(: unit Ack)` — the same rep-independent single-ctor form as the scalar case.
            let vty = type_of(db, value);
            let box_op = box_op_ty(db, &vty)?;
            let is_handle = box_op.is_none() && valtype_of(&vty) == Some(ValType::I32);
            let is_unit = box_op.is_none() && valtype_of(&vty).is_none();
            // Only a genuine handle operand carries transferable ownership; a boxed scalar leaf is a FRESH
            // OWNED temporary (dropped after the borrowing encode); the Unit handle is an IMMEDIATE (no drop).
            let vo = if is_handle {
                Some(heap_operand_ownership(db, value)?)
            } else {
                None
            };
            let val_slot = *high;
            let desc_slot = *high + 1;
            *high = desc_slot + 1;
            for s in [val_slot, desc_slot] {
                scratch_ty.insert(s, ValType::I32);
            }
            let op_base = *high;
            // Bake the descriptor into desc_slot FIRST (clean stack for the operand emit), exactly as ValueCmp.
            out.push(Lir::ConstI32(desc.len() as i32));
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32));
                out.push(Lir::ConstI32(byte as i32));
                out.push(Lir::CallImport(OP_BYTES_SET)); // [desc-buf]
            }
            out.push(Lir::LocalSet(desc_slot)); // [] — descriptor stored, stack clear
            emit(db, value, slots, op_base, high, scratch_ty, layout, out)?; // handle:[h] scalar:[s] unit:[]
            if is_unit {
                // Unit (nullary single-ctor): `emit` left NOTHING (Unit occupies no slot; any effects in the
                // operand still ran). Push the runtime's inline-unit handle for `value-encode` to frame.
                out.push(Lir::ConstI32(super::super::runtime_abi::IMM_UNIT as i32)); // [unit]
            } else if let Some(op) = box_op {
                // Erased scalar → box to a leaf handle: extend a narrow int/char to the i64 `box-int` cell
                // (no-op for a wide int / float / bool), then the width-correct box op → a fresh i32 leaf.
                emit_box_i32_to_i64_extend(db, value, out);
                out.push(Lir::CallImport(op)); // [leaf]
            }
            out.push(Lir::LocalTee(val_slot)); // [v]
            out.push(Lir::LocalGet(desc_slot)); // [v, desc]
            out.push(Lir::CallImport(OP_VALUE_ENCODE)); // [v, desc] → [doc] (borrows both)
            // Drop the borrowed-only descriptor Bytes, then the value operand: a fresh boxed leaf is always
            // owned (drop it); a handle operand drops only if it was owned. The fresh doc retains neither.
            out.push(Lir::LocalGet(desc_slot));
            out.push(Lir::CallImport(OP_DROP));
            match vo {
                Some(vo) => vo.drop_slot_if_owned(val_slot, out),
                // A boxed scalar leaf is a fresh owned temporary → drop it; the Unit handle is an IMMEDIATE
                // (`IMM_UNIT`, not a heap node) → nothing to reclaim.
                None if !is_unit => {
                    out.push(Lir::LocalGet(val_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
                None => {}
            }
            Ok(()) // leaves [doc]
        }
        // `Value.decode b` (R2, the inverse) — parse the binary-AST document `b` into a fresh owned value of
        // the call-site expected type via `value-decode(b, desc)` (returns the value handle or the NULL
        // signal on a shape/format mismatch), then wrap into `(Option a)`: `Some(handle)` when non-NULL, else
        // `None`. Descriptor bake + borrow/drop discipline as `ValueEncode`; the Some/None wrap mirrors
        // `MapLookup`, EXCEPT the decoded handle is FRESH OWNED (value-decode constructs it) so the `Some`
        // payload uses it directly with NO `dup` (MapLookup dup'd because the map still owned its value).
        Core::ValueDecode {
            bytes,
            desc,
            disc_some,
            disc_none,
        } => {
            // `value-decode` CONSTRUCTS a fresh value and the emit wraps it into the `(Option a)` result's
            // `Some(handle)`. The result node's type is `Option a`; `a` (peeled by `lower_value_decode`) is
            // the decode TARGET. Two target reps decode:
            //   - `a` is a HEAP HANDLE (record/tuple/sum/collection/bytes/…): value-decode returns the handle,
            //     used directly as the Some payload (the original R2 carve-out).
            //   - `a` is a SCALAR-ERASED single-field single-ctor (e.g. Envelope=|FireAfter(UInt64) →
            //     Ty::Nominal{inner:UInt64}): its descriptor is `Named(Type, <scalar>)`, and value-decode's
            //     Int/Bool/Float leaf arm constructs a BOXED leaf (`op_box_int`/…). That boxed leaf is EXACTLY
            //     the rep a scalar `Option` payload uses (a `Some(scalar)` boxes the scalar the same way), so
            //     the decoded handle is used directly as the Some payload — NO unbox/rebox. `box_op_ty(a)`
            //     being `Some` is the scalar discriminator (mirrors the `ValueEncode` box path).
            //   - `a` is a NULLARY single-ctor (`type Ack = | Ack` → `Ty::Nominal{inner:Unit}`): its
            //     descriptor is `Named(Ack, Unit)`, and value-decode's `Unit` arm returns the inline-unit
            //     handle (`imm_unit()`, non-NULL), used directly as the `Some` payload (a `Some(unit)` carries
            //     the same immediate — see the nullary `sum-new` payload). NULL still signals a decode miss.
            // An UNSOLVED target (`Ty::Var`/`Any`) is DECLINED (also caught earlier at lowering); an
            // unencodable target type (Fn/Cont/…) declines via `box_op_ty`'s `?`.
            let target_ty = type_of(db, id);
            let target_ok = match &target_ty {
                Ty::Sum { args, .. } => match args.first() {
                    Some(Ty::Var(_)) | Some(Ty::Any) => false,
                    Some(a) => {
                        box_op_ty(db, a)?; // declines an unencodable target; handle/scalar/Unit all decode
                        true
                    }
                    None => false,
                },
                _ => false,
            };
            if !target_ok {
                return Err(Reject::decline(
                    "Value.decode into an unsolved or unencodable target — annotate the scrutinee / use a \
                     typed let-binder so `a` grounds to a concrete decodable type",
                ));
            }
            let bo = heap_operand_ownership(db, bytes)?;
            let bytes_slot = *high;
            let desc_slot = *high + 1;
            let res_slot = *high + 2;
            *high = res_slot + 1;
            for s in [bytes_slot, desc_slot, res_slot] {
                scratch_ty.insert(s, ValType::I32);
            }
            let op_base = *high;
            // Bake the descriptor into desc_slot FIRST (clean stack for the operand emit).
            out.push(Lir::ConstI32(desc.len() as i32));
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [desc-buf]
            for (j, &byte) in desc.iter().enumerate() {
                out.push(Lir::ConstI32(j as i32));
                out.push(Lir::ConstI32(byte as i32));
                out.push(Lir::CallImport(OP_BYTES_SET)); // [desc-buf]
            }
            out.push(Lir::LocalSet(desc_slot)); // [] — descriptor stored, stack clear
            emit(db, bytes, slots, op_base, high, scratch_ty, layout, out)?; // [bytes]
            out.push(Lir::LocalTee(bytes_slot)); // [bytes]
            out.push(Lir::LocalGet(desc_slot)); // [bytes, desc]
            out.push(Lir::CallImport(OP_VALUE_DECODE)); // [bytes, desc] → [handle-or-null] (borrows both)
            out.push(Lir::LocalSet(res_slot)); // [] — result stashed
            // Drop the borrowed-only descriptor Bytes + any owned-temporary bytes operand. The decoded value
            // (or NULL) in res_slot is independent of both, so this is safe before the wrap.
            out.push(Lir::LocalGet(desc_slot));
            out.push(Lir::CallImport(OP_DROP));
            bo.drop_slot_if_owned(bytes_slot, out);
            // present = (handle != NULL).
            out.push(Lir::LocalGet(res_slot));
            out.push(Lir::ConstI32(NULL_HANDLE));
            out.push(Lir::I32Ne); // [present]
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            // THEN — Some(handle): the decoded value is FRESH OWNED, used directly as the payload (no dup).
            out.push(Lir::ConstI32(disc_some as i32)); // [disc_some]
            out.push(Lir::LocalGet(res_slot)); // [disc_some, handle]
            out.push(Lir::CallImport(OP_SUM_NEW)); // [Some-handle]
            out.push(Lir::Else);
            // ELSE — None: value-decode allocated nothing on a NULL return, so there is nothing to reclaim.
            emit_none_option(disc_none, out); // [None-handle]
            out.push(Lir::End);
            Ok(())
        }
        // A runtime arithmetic op. The numeric model fixes each operation's DEFINED outcome, which the
        // emitted instruction must honor at run time exactly as the constant fold does (`numeric-
        // model.md`; the const path folds in `lower` with the SAME traps → CDZ0304). Every op is
        // WIDTH-GENERIC — the operand type's width `N` and signedness drive it, no hard-coded 64:
        //   - the value lives in a MACHINE slot `M` = i32 (N ≤ 32) or i64 (else), normalized (sign-/
        //     zero-extended) — so a machine op computes the exact result whenever it fits M bits;
        //   - `+`/`-`/`*`/`<<` are CHECKED in two composed steps: an M-OVERFLOW guard (carry/borrow/
        //     round-trip) traps when the true result exceeds the machine slot, leaving `r` EXACT; then a
        //     RANGE-CHECK traps when `r` fits `M` but not the narrower `[min_N, max_N]`. Together they
        //     trap iff the true result leaves the N-bit type — at any N (an Int8 `100+100`, a UInt48
        //     `*` past 2^48, an Int64 `+` past 2^63 all trap by the same recipe);
        //   - `/`/`%` map to `div_s`/`rem_s`/`div_u`/`rem_u`; wasm traps natively on ÷0 and (`div_s` at
        //     N==M) on `MIN/-1`; a NARROW signed `/` whose `min_N / -1` the machine op does not trap is
        //     caught by the same range-check;
        //   - `&`/`|`/`^` are total on the two's-complement value — just the operands then the op.
        // A runtime operand only arises from a boundary parameter, and only the aliased widths
        // (8/16/32/64) have a boundary representation — but the recipe is correct for any N in 1..=64,
        // so nothing here assumes a machine width. A constant of any width folds in `lower` instead.
        // A runtime FLOAT arithmetic op (`+.`/`-.`/`*.`/`/.`) — emit the two operands then the machine
        // `f64`/`f32` op at the result width (read off the solved type). IEEE, NEVER traps, so NO overflow
        // guard (unlike the integer arith below). Both operands share the result's float type (binary-op
        // unification), so they emit at the same width.
        Core::Arith { op, lhs, rhs } if op.is_float_arith() => {
            // PEEL `Ty::Qty`/`Ty::Nominal` (strip_nominal → peel Qty → strip_nominal, via `peel_qty_ty`) —
            // a float arith over a QUANTITY, `(Qty Float32 u)`, solves to `Ty::Qty { inner: Float32 }`, NOT a
            // bare `Ty::Float`. WITHOUT the peel this fell to the f64 DEFAULT, so a `(Qty Float32)` `*`/`+`
            // emitted `f64.mul` (promoting its operands f32→f64) while a nested `Qty.value`-of-a-`*` operand
            // — itself now f64 — was RE-promoted `f64.promote_f32` on an already-f64 → "expected f32, found
            // f64", INVALID wasm (a nested Qty-mul over a Float32 magnitude; the arith twin of the ConstFloat
            // `peel_qty_ty` fix b4ce14cb + `int_ty_of`'s Qty peel). Peeling grounds every nested op to the
            // SAME f32 width, so all are `f32.mul` with no promote.
            let width = match peel_qty_ty(crate::infer::type_of(db, id)) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            trace!(target: "rcdzc::select", node = id.0, ?op, width, "emit runtime float op");
            // Ground each operand to the OP width: a bare float literal defaults to Float64 (an f64 slot),
            // which beside a Float32 operand would push an f64 into an `f32.add` and wasm rejects the
            // module. `emit_float_operand` materializes a literal at the op width (and demotes/promotes a
            // control-flow operand whose slot disagrees), the float analogue of the integer `emit_operand`.
            emit_float_operand(db, lhs, width, slots, base, high, scratch_ty, layout, out)?;
            emit_float_operand(db, rhs, width, slots, base, high, scratch_ty, layout, out)?;
            out.push(float_arith_op(op, width));
            Ok(())
        }
        Core::Arith { op, lhs, rhs } => {
            let m = Machine::of(int_ty_of(db, id));
            trace!(target: "rcdzc::select", node = id.0, ?op, width = m.width, signed = m.signed, "emit runtime integer op");
            match op {
                // STRENGTH REDUCTION: `x * 2^k` → `x << k`. A left shift IS exact multiplication by a
                // power of two, with the SAME defined overflow-trap (`numeric-model.md` §Overflow Is
                // Defined for shifts: "a left shift is exact multiplication by a power of two, so an
                // overflowing left shift MUST behave like an overflowing *"), so the rewrite preserves
                // BOTH value and trap at every width/signedness (verified: `* 8` and `<< 3` agree on
                // value and overflow-trap). The shift's overflow check is a cheap round-trip vs mul's
                // division; `k < width` is a compile-time constant so there is no count guard. Only for
                // `Mul` with a constant power-of-two operand ≥ 2 (`*1`/`*0` are handled by `lower`).
                Prim::Mul if mul_pow2_shift(db, lhs, rhs, m).is_some() => {
                    let (val, k) = mul_pow2_shift(db, lhs, rhs, m).unwrap();
                    emit_mul_pow2_as_shift(
                        db,
                        m,
                        val,
                        k,
                        slots,
                        base,
                        high,
                        scratch_ty,
                        layout,
                        out,
                        ResultDest::Stack,
                    )
                }
                Prim::Add | Prim::Sub | Prim::Mul => emit_checked_arith(
                    db, id, op, m, lhs, rhs, slots, base, high, scratch_ty, layout, out,
                ),
                // WRAPPING arithmetic — the RAW machine `add`/`sub`/`mul`, NO overflow guard (wasm's op
                // wraps modulo the SLOT). At a NARROW width the slot (i32/i64) is WIDER than the type, so the
                // raw op wraps mod 2^slot, NOT mod 2^width — the un-wrapped wide result is then OBSERVABLE (a
                // widening read `Int64.of`, an `=`-compare at the narrow width): `UInt8.wrapping-add 250 10`
                // must be `4` (mod 256), but the raw i32 add yields `260`. Nothing downstream re-masks (the
                // earlier "masked by ordinary consumer normalization" claim was FALSE — adv-57 miscompile), so
                // RE-NORMALIZE the result to `m.width` here, exactly as `emit_wrap` does for a narrowing wrap:
                // mask to the low `width` bits (unsigned), plus sign-extend from bit `width-1` (signed). A
                // FULL-width op needs nothing (the slot IS the width — the raw op already wraps modulo it).
                Prim::WrappingAdd | Prim::WrappingSub | Prim::WrappingMul => {
                    let ot = IntTy::fixed(m.signed, m.width);
                    emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    emit_operand(db, rhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    out.push(match op {
                        Prim::WrappingAdd => m.add(),
                        Prim::WrappingSub => m.sub(),
                        _ => m.mul(),
                    });
                    // Re-normalize a NARROW result to its width (the raw op wrapped mod the wider slot). The
                    // same mask/sign-extend `emit_wrap` step 3 applies; a full-width result is already the
                    // slot's modulus so it is left alone.
                    if m.narrow() {
                        let slot_bits = m.slot_bits();
                        if m.signed {
                            // Sign-extend from bit width-1: `(x << (M-N)) >> (M-N)` (arithmetic shr masks the
                            // high bits out AND sign-fills). `m.shr()` is arithmetic for a signed machine.
                            let shift = (slot_bits - m.width) as i64;
                            out.push(m.konst(shift));
                            out.push(m.shl());
                            out.push(m.konst(shift));
                            out.push(m.shr());
                        } else {
                            // Zero-fill: mask to the low `width` bits.
                            let mask = (1i64 << m.width) - 1;
                            out.push(m.konst(mask));
                            out.push(m.and());
                        }
                    }
                    Ok(())
                }
                Prim::BitAnd | Prim::BitOr | Prim::BitXor => {
                    let ot = IntTy::fixed(m.signed, m.width);
                    // REDUNDANT-MASK ELISION (flow-sensitive): `(& v M)` where the constant `M` covers `v`'s
                    // whole provable range is just `v` — emit the value alone, drop the mask. This is the
                    // emit-time sibling of the `is_full_mask_for` lower fold; it fires where `lower` cannot,
                    // on a value the branch REFINEMENT bounds (`(if (and (>= x 0) (< x 256)) (& x 255) …)` →
                    // `x & 255 == x` under `x ∈ [0,255]`). `&` is total (no trap dropped) and the value
                    // operand is emitted so its own evaluation/traps stay.
                    if op == Prim::BitAnd
                        && let Some(v) = crate::lower::redundant_and_mask_value(db, lhs, rhs)
                    {
                        return emit_operand(db, v, ot, slots, base, high, scratch_ty, layout, out);
                    }
                    // OR-SATURATION ELISION (flow-sensitive): `(| v M)` where the constant `M` covers `v`'s
                    // whole provable range is just `M` — `v | M == M`, so emit the constant alone (`v`'s
                    // bits are all already set in `M`). The emit-time sibling of the `BitOr` OR-saturation
                    // lower fold, firing on a branch-REFINED `v` (`(if (and (>= x 0) (< x 256)) (| x 255) …)`
                    // → `255` under `x ∈ [0,255]`). DISCARDS `v` — `redundant_or_mask_const` already checked
                    // `v` is trap-free — so no defined trap is dropped.
                    if op == Prim::BitOr
                        && let Some(c) = crate::lower::redundant_or_mask_const(db, lhs, rhs)
                    {
                        return emit_operand(db, c, ot, slots, base, high, scratch_ty, layout, out);
                    }
                    emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    emit_operand(db, rhs, ot, slots, base, high, scratch_ty, layout, out)?;
                    out.push(m.bitwise(op));
                    Ok(())
                }
                Prim::Div | Prim::Rem => emit_div_rem(
                    db, op, m, lhs, rhs, slots, base, high, scratch_ty, layout, out,
                ),
                Prim::Shl | Prim::Shr => emit_shift(
                    db, op, m, lhs, rhs, slots, base, high, scratch_ty, layout, out,
                ),
                // A comparison prim never reaches `Core::Arith` (it lowers to `Core::Compare`); a type
                // constructor never lowers as a runtime op. Decline rather than emit a wrong op.
                _ => Err(Reject::decline(
                    "not a runtime integer arithmetic operation",
                )),
            }
        }
        // A runtime integer CONVERSION. `wrap` TRUNCATES the operand to this node's target width/sign
        // (read off `type_of(id)`): move the operand into the target slot, keep its low N bits, and
        // reinterpret them at the target sign. Total (never traps) — the const path folds identically in
        // `lower`. This is the runtime sibling of `IntValue::wrap_to`.
        // INT→FLOAT conversion (`Float N.of-int`): emit the integer operand (an i64 machine value —
        // grounded to Int64 as the `of-int` source type) then `f{64,32}.convert_i64_s` at the target
        // float width. Handled BEFORE the integer-target `Machine::of` below (its target is a float).
        Core::Convert {
            op: Prim::FloatOfInt,
            operand,
        } => {
            let width = match type_of(db, id) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            emit_operand(
                db,
                operand,
                IntTy::i64(),
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
            )?;
            out.push(if width == 32 {
                Lir::F32ConvertI64S
            } else {
                Lir::F64ConvertI64S
            });
            Ok(())
        }
        // FLOAT-WIDTH conversion (`Float N.of`): emit the float operand, then demote/promote by the
        // SOURCE (operand) and TARGET (this node) widths — `f32.demote_f64` (64→32), `f64.promote_f32`
        // (32→64), or NOTHING (same width = identity). Handled before the integer-target arm below.
        Core::Convert {
            op: Prim::FloatOf,
            operand,
        } => {
            let src_w = match type_of(db, operand) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            let dst_w = match type_of(db, id) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?;
            match (src_w, dst_w) {
                (64, 32) => out.push(Lir::F32DemoteF64),
                (32, 64) => out.push(Lir::F64PromoteF32),
                // same width — the conversion is the identity, no opcode.
                _ => {}
            }
            Ok(())
        }
        Core::Convert { op, operand } => {
            let src = Machine::of(int_ty_of(db, operand));
            let dst = Machine::of(int_ty_of(db, id));
            trace!(target: "rcdzc::select", node = id.0, ?op, from_width = src.width, to_width = dst.width, "emit runtime conversion");
            match op {
                Prim::Wrap => emit_wrap(
                    db, src, dst, operand, slots, base, high, scratch_ty, layout, out,
                ),
                _ => Err(Reject::decline("not a runtime conversion")),
            }
        }
        // A runtime boolean NEGATION `!operand` — emit the logical NOT of the operand (a Bool i32). From
        // the `(if c false true)` fold. `emit_negated_bool` folds `(not (CMP a b))` into the complement
        // comparison and otherwise emits `operand ; i32.eqz`.
        Core::Not { operand } => {
            emit_negated_bool(db, operand, slots, base, high, scratch_ty, layout, out)
        }
        // A SHORT-CIRCUITING boolean connective — emitted as an `if` over `lhs` (a Bool i32), so `rhs` is
        // evaluated on ONLY ONE branch (the shield core-semantics.md §Boolean Connectives Short-Circuit
        // requires): `and` → `if lhs then rhs else 0`; `or` → `if lhs then 1 else rhs`. The `if` yields an
        // i32 Bool. (A constant `lhs` folded in `lower`, so here `lhs` is a runtime bool.)
        Core::And { lhs, rhs, is_and } => {
            // BRANCHLESS BOOLEAN: when `rhs` can neither TRAP nor EFFECT, the short-circuit is unnecessary
            // — `and`/`or` become a bitwise `i32.and`/`i32.or`. Booleans are canonical i32 `0`/`1`, so
            // `p & q` IS the boolean AND and `p | q` IS the boolean OR; the only thing short-circuit
            // preserves is NOT evaluating `rhs` when `lhs` decides the result — and a trap-free,
            // effect-free `rhs` has nothing to skip, so evaluating it unconditionally is identical. This
            // covers a bare LEAF and — the common case — a COMPARISON `(and (< a b) (< c d))` or a
            // bitwise/`not`/`wrap` combination of leaves, all total (`is_branchless_bool_rhs`). A `rhs`
            // that could trap (a checked op, `/`), call, allocate, or effect KEEPS the short-circuit `if`
            // so it runs only when reached.
            // `rhs` starts its scratch ABOVE `lhs`'s high-water (`base.max(*high)`), NOT at `base`. `lhs`
            // may leave a TRANSIENT scratch slot TYPED for the whole function (a `bin` length probe's
            // checked-arith `off + n` stashes its operand `n`/result at an i64 slot); `rhs` reusing that
            // slot index at a DIFFERENT width (a `(bytes p n)` `BinSizedRead` handle is an i32, blindly
            // `scratch_ty.insert`ed) would declare one wasm local at two widths → an invalid module
            // (`expected i32, found i64`, breaker cg3c: `(guard (bin (u8 k) (bytes p k)) (> (Bytes.len p) 1))`
            // — the guard's `BinSizedRead` of `p` aliased the predicate's i64 `off + k` slot). Floating
            // `rhs` above `*high` hands it fresh, never-typed slots — the same disjoint-slot discipline the
            // `Core::If` branches and checked-arith B-operand already apply (a slot's TYPE is fixed for the
            // whole function, so width-disjoint temps must not alias even when their lifetimes don't overlap).
            // `base.max(*high)` is read AFTER `emit(lhs)` raises `*high`, so it clears lhs's typed scratch.
            if is_branchless_bool_rhs(db, rhs) {
                emit(db, lhs, slots, base, high, scratch_ty, layout, out)?;
                emit(
                    db,
                    rhs,
                    slots,
                    base.max(*high),
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(if is_and { Lir::I32And } else { Lir::I32Or });
                return Ok(());
            }
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?;
            out.push(Lir::If(BlockType::Val(ValType::I32)));
            if is_and {
                // then: rhs ; else: false (0)
                emit(
                    db,
                    rhs,
                    slots,
                    base.max(*high),
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
                out.push(Lir::Else);
                out.push(Lir::ConstI32(0));
            } else {
                // then: true (1) ; else: rhs
                out.push(Lir::ConstI32(1));
                out.push(Lir::Else);
                emit(
                    db,
                    rhs,
                    slots,
                    base.max(*high),
                    high,
                    scratch_ty,
                    layout,
                    out,
                )?;
            }
            out.push(Lir::End);
            Ok(())
        }
        // A runtime CLOSURE VALUE — a NO-CAPTURE closure is exactly its funcref-TABLE SLOT, an i32
        // constant. The element section maps slot `code` → the lifted function's wasm index, so pushing
        // the slot is the whole closure value (no heap cell — captures would add an `arr-alloc` here).
        // A runtime CLOSURE VALUE — a heap CELL `arr-alloc(1 + captures)`: slot 0 = `box-int(code)` (the
        // funcref-table slot), slots 1.. = each captured value (boxed if a scalar). Leaves the cell's u32
        // handle on the stack. Uniform shape for capturing AND non-capturing closures, so a fn-typed
        // parameter holds either interchangeably. Built exactly like a tuple (`arr-set` returns the array
        // handle for threading, so no scratch local needed).
        Core::Closure { code, captures } => {
            out.push(Lir::ConstI32(1 + captures.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [cell]
            // Slot 0: box-int(code). `box-int` takes an i64, so the table slot is an `i64.const`.
            out.push(Lir::ConstI32(0)); // index
            out.push(Lir::ConstI64(code as i64)); // the table slot (box-int wants i64)
            out.push(Lir::CallImport(OP_BOX_INT));
            out.push(Lir::CallImport(OP_ARR_SET)); // → [cell]
            // Slots 1..: each captured value, boxed if scalar (a narrow int extends i32→i64 first, like a
            // tuple element), arr-set into place.
            for (k, &cap) in captures.iter().enumerate() {
                out.push(Lir::ConstI32(1 + k as i32)); // index
                emit(db, cap, slots, base, high, scratch_ty, layout, out)?;
                // A scalar capture boxes; a compound is stored as-is; a UNIT capture holds the inline-unit
                // sentinel in its cell slot (the value pushed nothing).
                let boxed = box_op(db, cap)?;
                emit_heap_store_tail(db, cap, boxed, out);
                out.push(Lir::CallImport(OP_ARR_SET)); // → [cell]
            }
            Ok(())
        }
        // A runtime CLOSURE APPLICATION — `call_indirect` through the funcref table. The lifted function
        // is `(env, arg) -> result`: push the arg, then the env cell, then read the table slot from the
        // cell (`arr-get(cell, 0)` + `get-int`) as the indirection index, then `call_indirect`. The cell
        // must be materialized into a local so it is read TWICE (once passed as env, once for the code
        // slot) without recomputation.
        Core::CallClosure { closure, args } => {
            let type_index = match closure_type_index(db, closure, &args, layout) {
                Some(ti) => ti,
                None => {
                    // No lifted lambda has this application's machine signature. Distinguish a PROVABLY
                    // DEAD site from a merely UNSUPPORTED one by the CLOSURE OPERAND'S TYPE, not the
                    // application's arity: a runtime closure value arises ONLY from a lambda lift, so if
                    // NO lift can produce a value of the operand's full-curried type
                    // (`closure_operand_is_dead`), the operand holds no callable value and this
                    // application can NEVER execute — emit `unreachable` (validates by wasm's
                    // stack-polymorphic typing, traps only if somehow reached, which the type system
                    // forbids). This is the "two distinctly-typed boxed closures in ONE sum, only one
                    // ever built" shape: an iterator sum with both a binary `scan` accumulator and an
                    // element→sub-iterator `flat-map` closure — each `next` arm statically applies its
                    // boxed closure, but a program constructing only the `scan` variant never lifts the
                    // `flat-map` machine shape, so its arm's `call_indirect` is dead.
                    //
                    // When a lift DOES inhabit the operand's type but no lift matches the APPLICATION's
                    // arity (a curried multi-param closure lifted as nested unaries, applied at flattened
                    // higher arity — `(f 2 3)` over `(fn a (fn x …))`), the site is LIVE and must NOT be
                    // stubbed with `unreachable`; it declines as an unsupported application shape, exactly
                    // as before. A non-function / non-representable operand type also declines here.
                    let operand_ty = type_of(db, closure);
                    if closure_operand_is_dead(&operand_ty, layout) {
                        out.push(Lir::Unreachable);
                        return Ok(());
                    }
                    return Err(Reject::decline(
                        "a runtime closure application has no matching function type",
                    ));
                }
            };
            // Materialize the closure cell into a scratch local (read twice: env arg + code slot).
            let cell_slot = base.max(*high);
            *high = (*high).max(cell_slot + 1);
            scratch_ty.insert(cell_slot, ValType::I32);
            emit(
                db,
                closure,
                slots,
                cell_slot + 1,
                high,
                scratch_ty,
                layout,
                out,
            )?;
            out.push(Lir::LocalSet(cell_slot));
            // KNOWN-CLOSURE DEVIRTUALIZATION (S1): if the closure operand folds to a compile-time-visible
            // `Core::Closure { code, .. }` — the closure\'s CONSTRUCTOR SITE is statically known at this
            // call (a closure stored in a const variant/record field then applied at full arity, the
            // ad-hoc-poly dispatch shape; and NOT beta-reduced away because the closure survives as a
            // runtime value, e.g. it captures state and is applied more than once) — then the funcref table
            // slot is a compile-time constant, so the runtime indirection (read `code` from the cell +
            // `call_indirect`) is pure overhead. Emit a DIRECT `call` to the lifted function at that slot
            // instead: the calling convention is IDENTICAL (`(env=cell, args…)`), only the dispatch changes
            // from indirect to direct. The cell is still built + passed as env (its captures are read back
            // by the body\'s `Captured` nodes). This makes an ad-hoc-poly call through a known closure a
            // direct call the wasm engine can inline, matching the rust backend — which already emits a
            // direct closure call that rustc/LLVM devirtualizes. Falls back to `call_indirect` for a
            // genuinely runtime closure whose constructor site is NOT visible here (an unknown `Param` /
            // variant payload threaded through a recursive driver — the case S2 addresses via specialization).
            let known_code = match core_of(db, closure) {
                Core::Closure { code, .. } => Some(code),
                _ => None,
            };
            // The lifted function is `(env, args…) -> result`, so push env (param 0) THEN each arg, in
            // order, before the call. Each arg emits above the cell slot AND above every scratch slot the
            // CLOSURE OPERAND's own emit consumed — use `(*high).max(cell_slot + 1)`, not a bare
            // `cell_slot + 1`. A closure operand that is a dup-site `Core::SumPayload` (the looked-up-closure
            // shape: `(match (Map.lookup …) ((Some f) (f …)))`) floats its retain child into a slot at `*high`
            // typed I32 (the closure cell); a plain `cell_slot + 1` base lets a following perform-threaded arg
            // (an i64) materialize into that SAME index, and a wasm local has ONE type function-wide → i32/i64
            // collision → an invalid module (`func failed to validate: expected i64, found i32`). Threading the
            // base past `*high` keeps the arg's scratch DISJOINT from the closure operand's retain slot. (The
            // host-call arg emit below already threads `arg_base` for the identical two-widths hazard.)
            let arg_base = (*high).max(cell_slot + 1);
            out.push(Lir::LocalGet(cell_slot)); // env (the cell)
            for &arg in args.iter() {
                emit(db, arg, slots, arg_base, high, scratch_ty, layout, out)?;
            }
            match known_code {
                // Devirtualized: the table slot is known, so call the lifted function directly.
                Some(code) => out.push(Lir::Call(layout.lifted_abs(code))),
                // Runtime closure: read the indirection index — arr-get(cell, 0) -> box-int(code); get-int
                // -> the table slot as an i64; `call_indirect` needs an i32, so narrow it. The code is a
                // small table slot, so the wrap is exact.
                None => {
                    out.push(Lir::LocalGet(cell_slot));
                    out.push(Lir::ConstI32(0));
                    out.push(Lir::CallImport(OP_ARR_GET));
                    out.push(Lir::CallImport(OP_GET_INT));
                    out.push(Lir::I32WrapI64);
                    out.push(Lir::CallIndirect(type_index));
                }
            }
            // SITE-A OWNED-TEMP ENV-CELL RECLAIM (part b, co-owned with v-memory-safety who landed part a).
            // The call BORROWS the env cell (the lifted body reads captures via `Core::Captured` arr-get; it
            // does not consume the cell), so after a FULL application the cell in `cell_slot` is a dead owned
            // temporary. When the closure operand is a freshly-built OWNED producer (`heap_operand_ownership
            // == Owned` — part a classifies a `Core::Closure`, and an `If`/`match` join of fresh partial-ctor
            // closures, as Owned) AND the application's RESULT is not itself a function (so the cell is not
            // re-applied / returned as a residual closure — the curried/thunk escape the root memory flags as
            // unsound to drop), drop the cell after the call: the result is already on the stack, so
            // `local.get cell_slot; drop` reclaims the env cell (+ its dup'd boxed captures, via the runtime
            // dtor) without disturbing the result beneath it. A BORROWED operand (a param/local/captured cell
            // whose owner reclaims it) or a FUNCTION-typed result leaves the cell untouched — leak-safe: an
            // unproven ownership just leaves it un-dropped, never double-frees. This flips the SITE-A leak
            // probe 4→2 (the closure's env cell + boxed capture; the remaining 2 are the general match/tuple
            // Perceus gap). Devirtualized + indirect paths share the reclaim (both borrow the cell identically).
            let result_is_fn = matches!(type_of(db, id), crate::ty::Ty::Fn(_, _));
            let operand_owned = matches!(
                heap_operand_ownership(db, closure),
                Ok(HandleOwnership::Owned)
            );
            if operand_owned && !result_is_fn {
                out.push(Lir::LocalGet(cell_slot)); // [result, cell]
                out.push(Lir::CallImport(OP_DROP)); // → [result] (reclaim the owned env cell)
            }
            Ok(())
        }
        // A CAPTURED free-variable read inside a lifted closure body — `arr-get(env, 1 + index)` then
        // unbox by the captured value's type (a scalar `get-int`/`get-bool`, then a NARROW int narrows
        // i64→i32; a compound handle is used as-is). The env cell is the lifted function's local slot 0.
        // The node's own `type_of` is the captured value's type (set at lowering), so `get_op`/`is_narrow`
        // read it exactly as a tuple projection does.
        Core::Captured { index, .. } => {
            // hcz CAPTURE-ESCAPE RETAIN: a compound capture whose sole read ESCAPES (returned / consumed) must
            // `dup` so the returned value owns an INDEPENDENT ref — else the monolithic env-cell drop (which
            // cascades to this capture) and the consumer both free the SAME ref (hcz1/hcz2 double-release).
            // Emitted via a SEPARATE `arr-get` that `OP_DUP` consumes (rc++), mirroring `emit_binder_ref`'s
            // extra-read-then-dup; the normal read below then leaves the handle as the value. Marked only for
            // the single-read escaping compound shape (`collect_captured_escape_dup_sites`).
            if out.captured_escape_dup_sites.contains(&id) {
                out.push(Lir::LocalGet(0));
                out.push(Lir::ConstI32(1 + index as i32));
                out.push(Lir::CallImport(OP_ARR_GET));
                out.push(Lir::CallImport(OP_DUP)); // rc++ — pops this copy, returns nothing
            }
            out.push(Lir::LocalGet(0)); // the env cell (lifted fn's 1st param)
            out.push(Lir::ConstI32(1 + index as i32));
            out.push(Lir::CallImport(OP_ARR_GET));
            // A scalar capture unboxes; a compound is used as-is; a UNIT capture drops the inline-unit
            // sentinel the cell slot held (a `Unit`-typed captured variable has no machine value).
            let unboxed = get_op(db, id)?;
            emit_heap_read_tail(db, id, unboxed, out);
            Ok(())
        }
        // A HOST CALL — a perform delegated to the component boundary. Emit each scalar argument (in
        // order), then `call <host-import-index>` (the imported op's core-func index, its position in the
        // program's host-import set, resolved via `layout.host_index`). A `Unit` argument occupies no
        // boundary slot, so it is skipped (a nullary op `(E.op)` pushes nothing). The op's scalar result
        // is left on the stack by the imported call.
        //
        // This is JUST an ordinary imported-function call: the program pushes its arguments and reads the
        // response the import leaves on the stack — nothing here encodes or observes how the host produces
        // that response (inline, suspend-and-resume, or abort). The emitted instruction is the same under
        // every host resolution strategy.
        //= spec/capabilities/capabilities-and-effects.md#a-host-call-returns-a-response
        //# A host call MUST be an ordinary call to an imported function that returns its response to the program, so that from the program's side reaching the host is a plain function call and how the host produces the response is the host's concern.
        //= spec/capabilities/capabilities-and-effects.md#a-host-call-returns-a-response
        //# The program MUST NOT observe or encode how a host call is resolved, so that whether the host answers inline, suspends the run and resumes it later, or aborts it is invisible to the program and not part of the program's meaning.
        Core::HostCall {
            effect,
            op,
            args,
            result,
        } => {
            // EFFECTS-UNIFICATION (U2): an escaping effect BOUND to a peer contract
            // (`db.effect_bindings`) is a PEER call — resolve it against the extern-import set and emit a
            // `CallExternImport`, exactly as a `Core::ExternCall` did. An unbound effect stays a host call.
            if let Some(iface) = db.effect_bindings.get(&*effect).cloned() {
                let index = layout.extern_index(&iface, &op).ok_or_else(|| {
                    // The peer op is not in `extern_order` — which the RESOURCE-ESCAPE emit paths
                    // (`emit_runtime_resource`/`emit_recursive_sum_resource`) do not populate: they carry
                    // the runtime import but not the peer extern envelope. So a peer-bound op reached in a
                    // body whose ENTRYPOINT RESULT escapes as a runtime resource (the entrypoint RETURNS
                    // the compound/Option a peer produced) has no import to call. This is a known gap (a
                    // resource×peer-extern envelope fusion is the fix); until then, the workaround is to
                    // consume the peer's value into a SCALAR the entrypoint returns (e.g. read the field/
                    // element and return it, or `List.len`) rather than returning the raw compound, OR
                    // handle the effect in-program instead of binding it to a peer.
                    Reject::unsupported(format!(
                        "a peer-bound effect op (`{op}` on `{iface}`) is reached in an entrypoint whose \
                         RESULT escapes as a runtime resource (it returns the compound/collection the peer \
                         produced) — the resource-escape boundary does not support carrying the peer import. \
                         Consume the peer's value into a scalar the entrypoint returns, or handle the \
                         effect in-program instead of binding it to a peer"
                    ))
                })?;
                for &arg in args.iter() {
                    if matches!(crate::infer::type_of(db, arg), Ty::Unit) {
                        continue;
                    }
                    emit(db, arg, slots, base, high, scratch_ty, layout, out)?;
                }
                out.push(Lir::CallExternImport(index));
                return Ok(());
            }
            let index = layout.host_index(&effect, &op).ok_or_else(|| {
                Reject::decline("a host call's operation is not in the host-import set")
            })?;
            // A runtime String/Bytes arg is marshalled into a scratch region of the shared `mem` (copy the
            // rope's logical bytes in, pass `(ptr,len)`). N such args in one call each need a DISJOINT region,
            // so a running CURSOR (a scratch i32 local) starts at the fixed scratch base and advances by each
            // marshalled arg's runtime length — the k-th runtime compound arg lands past the first k-1. The
            // cursor is reserved (and the per-arg `emit` floor raised past it) ONLY when a runtime compound arg
            // is actually present, so an all-scalar / const-string host call stays byte-identical to before.
            let has_runtime_compound = args.iter().any(|&a| {
                let at = crate::infer::type_of(db, a);
                (matches!(at, Ty::String | Ty::Bytes) && !matches!(core_of(db, a), Core::ConstStr(_)))
                    // A record arg with a `Bytes` FIELD (shape d2) also copies rope bytes into `mem`, so it
                    // needs the running scratch cursor reserved just like a Bytes arg.
                    || crate::backend::wasm::host::record_has_bytes_field(&at)
                    // A record arg with a `list<T>` FIELD marshals that list's backing into `mem` → cursor too.
                    || crate::backend::wasm::host::record_has_list_field(&at)
                    // A record arg with an `option<bytes>` FIELD copies the payload rope into `mem` → cursor.
                    || crate::backend::wasm::host::record_has_option_bytes_field(db, &at)
                    // A record arg with a `tuple<…>` FIELD may copy a Bytes element's rope → reserve the cursor.
                    || crate::backend::wasm::host::record_has_tuple_field(&at)
                    // A `list<T>` arg marshals into `mem` (its outer array + each element) → needs the cursor.
                    || matches!(at.strip_nominal(), Ty::List(_))
            });
            let scratch_cursor_slot = if has_runtime_compound {
                let slot = base.max(*high);
                *high = (*high).max(slot);
                scratch_ty.insert(slot, ValType::I32);
                // Seed the cursor at the fixed scratch base once, before any arg is marshalled.
                out.push(Lir::ConstI32(host_arg_scratch_base(layout) as i32));
                out.push(Lir::LocalSet(slot));
                Some(slot)
            } else {
                None
            };
            // Each arg is emitted above the scratch its predecessors consumed — `arg_base` rises to `*high`
            // after every arg (the same `arg_base = *high` threading the call/tail-call arg loops use —
            // `emit_call_args` and `emit_loop_iteration`). WITHOUT this, a runtime
            // String/Bytes marshal reserves i32 rope/len/pos slots at `base.max(*high)` and bumps `*high`, but
            // a FOLLOWING scalar arg emitted with the stale `base` would tee its i64 checked-arith guard into
            // that same slot — one wasm local declared at two widths → an invalid module (the marshalled-arg-
            // BEFORE-scalar order; the reverse worked only because the scalar bumped `*high` first).
            // When a scratch cursor is reserved, per-arg slots must start ABOVE it so no marshal/scalar arg
            // reuses (and re-types) the cursor slot, which must survive across the whole arg list.
            let mut arg_base = scratch_cursor_slot.map_or(base, |c| c + 1);
            // The op's declared param WIT types (declaration order) — a RECORD arg marshals its fields in the
            // host WIT record's field order, not the guest's name-lex order (else the component-linker's
            // structural match fails → silent no-instantiate). `arg_i` indexes it (args ↔ WIT params align).
            let wit_params = crate::backend::wasm::host::wit_op_param_types(db, &effect, &op);
            for (arg_i, &arg) in args.iter().enumerate() {
                let at = crate::infer::type_of(db, arg);
                match at {
                    // A unit argument carries no boundary value.
                    Ty::Unit => continue,
                    // A STRING or BYTES argument crosses as `(ptr, len)` into the SHARED host memory
                    // (`assemble_host_mem` provides it; `set_needs_memory` fires for any String/Bytes-param
                    // op). A CONSTANT string's bytes were laid in the data segment at `host_string_offset` —
                    // push that ptr + len. Everything else (a RUNTIME string rope OR any `Bytes` value, const
                    // or runtime — a `Bytes.of`/slice-view byte-buffer) is MARSHALED here: copy its logical
                    // bytes into a cursor-advanced scratch region of `mem` via the rep-agnostic
                    // `bytes-len`/`bytes-get` walk (transparent through a rope OR a slice-view), then push
                    // `(cursor, len)` — N runtime compound args per call each get a disjoint region. The
                    // component boundary declares the Bytes param as `list<u8>` (a defined type-index) vs the
                    // String's inline `string`; the CORE marshalling here is identical. adv-62b sibling: this
                    // is the wasm side of the Bytes-host-arg reverse-parity gap (rust already crossed it).
                    Ty::String | Ty::Bytes => match core_of(db, arg) {
                        Core::ConstStr(s) => {
                            let offset = layout.host_string_offset(&s).ok_or_else(|| {
                                Reject::decline(
                                    "a host-arg string was not laid in the data segment",
                                )
                            })?;
                            out.push(Lir::ConstI32(offset as i32));
                            out.push(Lir::ConstI32(s.len() as i32));
                        }
                        // RUNTIME string/Bytes arg → copy the rope into `mem` at the running cursor, push
                        // `(cursor, len)`, then advance the cursor by `len` so a following runtime compound arg
                        // lands in a DISJOINT region (N runtime compound args per call are supported; the cursor
                        // starts at the fixed scratch base past the const-string data in the 1-page shared
                        // `mem`). A host arg is consumed IMMEDIATELY by the call (not retained), so all the
                        // marshalled args coexist in scratch only until the `CallHostImport`. The copy loop
                        // mirrors `String.scalar-len`'s byte-scan (~7086) with `I32Store8` in place of the
                        // counter. `bytes-len`/`bytes-get` are declared for this path in `collect_used_ops_into`
                        // (else their `CallImport` resolves to u32::MAX).
                        _ => {
                            let cursor = scratch_cursor_slot.expect(
                                "a runtime compound arg reserves the scratch cursor (pre-scan)",
                            );
                            let rope_slot = arg_base.max(*high);
                            *high = (*high).max(rope_slot + 3);
                            scratch_ty.insert(rope_slot, ValType::I32);
                            let len_slot = rope_slot + 1;
                            scratch_ty.insert(len_slot, ValType::I32);
                            let pos_slot = rope_slot + 2;
                            scratch_ty.insert(pos_slot, ValType::I32);
                            emit(db, arg, slots, rope_slot + 3, high, scratch_ty, layout, out)?;
                            out.push(Lir::LocalSet(rope_slot));
                            out.push(Lir::LocalGet(rope_slot));
                            out.push(Lir::CallImport(OP_BYTES_LEN)); // [len:i32]
                            out.push(Lir::LocalSet(len_slot));
                            out.push(Lir::ConstI32(0));
                            out.push(Lir::LocalSet(pos_slot)); // pos = 0
                            // block { loop { br_out if pos>=len; mem[cursor+pos] = bytes-get(rope,pos);
                            //   pos++; br loop } }
                            out.push(Lir::Block(BlockType::Empty)); // $done
                            out.push(Lir::Loop(BlockType::Empty)); // $copy
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::LocalGet(len_slot));
                            out.push(Lir::I32GeS);
                            out.push(Lir::BrIf(1)); // pos >= len → $done
                            out.push(Lir::LocalGet(cursor));
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::I32Add); // [addr = cursor + pos]
                            out.push(Lir::LocalGet(rope_slot));
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::CallImport(OP_BYTES_GET)); // [addr, byte:i32]
                            out.push(Lir::I32Store8 { offset: 0 }); // mem[cursor+pos] = byte
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::ConstI32(1));
                            out.push(Lir::I32Add);
                            out.push(Lir::LocalSet(pos_slot)); // pos++
                            out.push(Lir::Br(0)); // → $copy
                            out.push(Lir::End); // end $copy
                            out.push(Lir::End); // end $done
                            // Push (ptr = cursor-before-advance, len), then advance cursor += len for the next
                            // runtime compound arg. The advance is stack-neutral (leaves the pushed (ptr,len)
                            // in place below it on the operand stack).
                            out.push(Lir::LocalGet(cursor));
                            out.push(Lir::LocalGet(len_slot)); // push (ptr, len)
                            out.push(Lir::LocalGet(cursor));
                            out.push(Lir::LocalGet(len_slot));
                            out.push(Lir::I32Add);
                            out.push(Lir::LocalSet(cursor)); // cursor += len
                        }
                    },
                    // A RECORD argument (shape d) crosses NATIVELY: the guest DECOMPOSES the value-heap record
                    // field-by-field into the flattened core slots the component `record` param lowers to. The
                    // fields are pushed in the host WIT record's DECLARATION order (`emit_record_arg_marshal`
                    // reads the value-heap cell's name-lex position per WIT field), NOT the guest's name-lex
                    // order — the component-linker requires the import's flattened args to match the host.
                    //  • a SCALAR field reads back with `arr-get` + the field's wrap-free scalar get-op.
                    //  • a BYTES field is copied rope→`mem` at the running cursor and pushed as `(ptr,len)`.
                    // The reads BORROW the record (no consume), so the handle is not dropped here.
                    Ty::Record(fields) => {
                        // The host WIT record type for THIS arg — required to order the fields (declaration
                        // order); without it (world absent / arg not a WIT record) the marshal can't match the
                        // host, so decline rather than emit a name-lex order that won't link.
                        let Some(crate::wit_world::WitType::Record(_)) =
                            wit_params.as_ref().and_then(|p| p.get(arg_i))
                        else {
                            return Err(Reject::decline(
                                "a record host-arg has no matching WIT record type in the target world (needed \
                                 to order its fields to the host's declaration order)",
                            ));
                        };
                        let wit = wit_params.as_ref().unwrap()[arg_i].clone();
                        let rec_slot = arg_base.max(*high);
                        scratch_ty.insert(rec_slot, ValType::I32);
                        *high = (*high).max(rec_slot + 1);
                        emit(db, arg, slots, rec_slot + 1, high, scratch_ty, layout, out)?; // [rec]
                        out.push(Lir::LocalSet(rec_slot));
                        let work_base = *high;
                        emit_record_arg_marshal(
                            db,
                            rec_slot,
                            &fields,
                            &wit,
                            scratch_cursor_slot,
                            work_base,
                            high,
                            scratch_ty,
                            out,
                        )?;
                    }
                    // A `list<T>` argument (`graph.set-edges`'s `targets: list<reducer-id>`) — the guest
                    // MARSHALS the value-heap `List` into the shared `mem`: an outer array of `count` element
                    // slots at the running cursor, each element canonical-encoded after it, then passes
                    // `(outer-ptr, count)`. This slice marshals a `list<list<u8>>` (a `list<u8>` element =
                    // `(ptr,len)`); a non-`list<u8>` element is a later increment (decline).
                    Ty::List(elem) => {
                        let elem = (*elem).clone();
                        let cursor = scratch_cursor_slot.expect(
                            "a list arg reserves the scratch cursor (has_runtime_compound)",
                        );
                        // The element's declared WIT type (the list param's `WitType::List(elem)`) — threaded so
                        // a RECORD element orders its fields to the host WIT declaration order. `None` for a
                        // scalar/bytes element (offset-agnostic) or when no world declares this param.
                        let elem_wit = match wit_params.as_ref().and_then(|p| p.get(arg_i)) {
                            Some(crate::wit_world::WitType::List(ew)) => Some(ew.as_ref()),
                            _ => None,
                        };
                        let list_slot = arg_base.max(*high);
                        scratch_ty.insert(list_slot, ValType::I32);
                        *high = (*high).max(list_slot + 1);
                        emit(db, arg, slots, list_slot + 1, high, scratch_ty, layout, out)?; // [list]
                        out.push(Lir::LocalSet(list_slot));
                        let work_base = *high;
                        emit_list_arg_marshal(
                            db, &elem, elem_wit, list_slot, cursor, work_base, high, scratch_ty,
                            out,
                        )?;
                    }
                    // A bare scalar-payload VARIANT argument (the top-level param position): the guest emits
                    // the value-heap variant HANDLE into a slot, then decomposes it into the canonical
                    // `(disc, payload)` register-flatten via the SAME `emit_variant_reg_flatten` the record-
                    // field variant uses. Checked before the scalar `_` arm (a Sum's `emit` yields a HANDLE,
                    // not the flattened slots the component `variant` param expects).
                    _ if crate::backend::wasm::host::variant_scalar_payload_cases(db, &at)
                        .is_some() =>
                    {
                        let var_slot = arg_base.max(*high);
                        scratch_ty.insert(var_slot, ValType::I32);
                        *high = (*high).max(var_slot + 1);
                        emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                        out.push(Lir::LocalSet(var_slot));
                        let work_base = *high;
                        emit_variant_reg_flatten(
                            db, var_slot, &at, work_base, high, scratch_ty, out,
                        )?;
                    }
                    // A scalar argument emits its value directly.
                    _ => emit(db, arg, slots, arg_base, high, scratch_ty, layout, out)?,
                }
                // Raise the floor past ANY scratch this arg consumed, so the NEXT arg allocates fresh slots
                // (never reusing — and thus never re-typing — a slot a prior marshal/checked-op still owns).
                arg_base = (*high).max(arg_base);
            }
            // GENERAL RESULT LIFT: a host op whose result is a SPILLED compound (its flattened core form
            // exceeds one value, so the canonical ABI returns it through a caller-provided pointer) is
            // canon-lowered `(args…, retptr) -> ()`. Allocate the return area sized+aligned by the result
            // type's canonical layout, pass it as the trailing arg, call, then lift the host-written value
            // into a Cadenza value-heap handle by recursing over the WIT result type (`emit_result_lift`).
            // This ONE recursion REPLACES the former per-shape lift blocks (`option<list<u8>>`, bare
            // `list<u8>`, `list<tuple<list<u8>,list<u8>>>`) — the general shape mechanism, not a 4th shortcut.
            // The admit predicate is the SAME `host::result_is_liftable` the host-import collection + the
            // component defined-type emission use, so a new structural shape (`list<list<u8>>` for
            // graph.neighbors) rides this recursion in lockstep with its core-sig/comp-type plumbing.
            let spilled_result = crate::backend::wasm::host::result_is_liftable(db, &result);
            if spilled_result {
                let (size, align) = canonical_layout(db, &result);
                let retptr = (*high).max(base);
                *high = (*high).max(retptr + 1);
                scratch_ty.insert(retptr, ValType::I32);
                // retptr = cabi_realloc(0, 0, align, size); leave it as the trailing call arg (tee-stash).
                out.push(Lir::ConstI32(0));
                out.push(Lir::ConstI32(0));
                out.push(Lir::ConstI32(align as i32));
                out.push(Lir::ConstI32(size as i32));
                out.push(Lir::CallImport("cabi_realloc"));
                out.push(Lir::LocalTee(retptr)); // [args…, retptr]
                out.push(Lir::CallHostImport(index)); // (args…, retptr) -> () ; host stored the result
                // The op's declared WIT result type (the host's canonical layout) drives a record result's
                // field ORDER in the lift — the result-side of the follow-the-WIT rule. Fall back to the
                // guest-`Ty`-derived WIT when the world is absent (structural results are order-agnostic).
                let result_wit = crate::backend::wasm::host::wit_op_result_type(db, &effect, &op)
                    .or_else(|| crate::backend::wasm::host::spilled_result_wit_type(db, &result));
                emit_result_lift(
                    db,
                    &result,
                    result_wit.as_ref(),
                    retptr,
                    0,
                    high,
                    scratch_ty,
                    out,
                )?;
                return Ok(());
            }
            out.push(Lir::CallHostImport(index));
            Ok(())
        }
        // (The `Core::ExternCall` emit arm was REMOVED in U4 — a peer op is now a peer-bound effect's
        // escaping `Core::HostCall`, which the `Core::HostCall` arm above emits as a `CallExternImport`
        // when the effect is peer-bound.)
        // A SEQUENCING block — emit each non-final statement FOR ITS EFFECT (in order), then the tail as the
        // value. The `do`-fold (`lower.rs`) puts a statement here only when SOME non-final statement reaches a
        // host call; a statement itself is classified per the DEAD-INIT ruling (rust already does this,
        // adv-56 — this is the wasm parity face):
        //   • a statement that does NOT reach a host call is a DISCARDED PURE form — its value flows nowhere,
        //     so it is UNOBSERVED and must be ELIDED (not emitted). Emitting it would (a) leave a dangling
        //     value on the stack (imbalance) and (b) FORCE a trap that must not fire — `(do (/ 100 d) …)` at
        //     d=0 yields the tail, the div-by-zero is dead. This is EXACTLY the predicate CDZ0307 warns on
        //     (`collect_discarded_value_warnings` reads the same `subtree_reaches_host_call`), so no drift.
        //   • a statement that DOES reach a host call must be EMITTED (the call crosses the boundary). A
        //     Unit-result host call leaves nothing (a `func()`-typed import) — emit as-is. A value-leaving
        //     host call (a discarded non-Unit result, e.g. `(io.put 1)` returning Int64) leaves its value on
        //     the stack, so `Drop` it to keep the block balanced (the tail is the block's value).
        //= spec/capabilities/core-semantics.md#a-sequencing-block-evaluates-its-forms-in-order
        //# A sequencing block MUST evaluate to the value of its last form.
        Core::Seq { stmts, tail } => {
            for s in stmts.iter() {
                if !crate::lower::subtree_reaches_host_call(db, *s) {
                    // (A) STRICT heap-collection construction (#5194 CASE2): a strict-construction arg
                    // computation `lower_let` decomposed out of a DEAD list/set/map ctor is marked in
                    // `db.strict_force_eval` and MUST be evaluated (its trap fires) — the (A)-overrides-§283
                    // rule (v-spec-oracle): a reached heap-collection ctor's args are strict, NOT deferrable.
                    // A SCALAR-typed marked arg's discarded result is popped with a bare `drop` (no refcount).
                    // A HEAP-typed one (a `Rational.of` / checked-BigInt PRODUCER — `lower_let` admits only
                    // owned producers here, never a borrowed leaf) leaves a FRESH OWNED handle → rc-reclaim it
                    // with `OP_DROP` (NOT a bare stack `Lir::Drop`, which would leak the fresh allocation).
                    if db.strict_force_eval.contains(s) {
                        emit(db, *s, slots, base, high, scratch_ty, layout, out)?;
                        let ty = crate::infer::type_of(db, *s);
                        if crate::core_analysis::is_heap_type(&ty) {
                            out.push(Lir::CallImport(OP_DROP));
                        } else if valtype_of(&ty).is_some() {
                            out.push(Lir::Drop);
                        }
                        continue;
                    }
                    // A discarded PURE statement is unobserved — elide it (its trap, if any, is dead-init).
                    continue;
                }
                // A host-reaching statement must run. Emit it; if it left a MACHINE VALUE on the stack (a
                // discarded non-Unit host-call result), drop that value so the block stays stack-balanced.
                // The drop condition is EXACTLY "did this statement leave a machine value?" =
                // `valtype_of(type_of(..)).is_some()` — NOT `strip_nominal() != Unit`: `valtype_of` returns
                // None for `Unit` AND for `Char` / `Type` / `Var` / `Any` (all no-runtime-slot), so a
                // statement of one of those types leaves nothing and must NOT be dropped — a `Lir::Drop` on
                // an empty stack underflows → an invalid module. (valtype_of strips nominals internally, so
                // a newtype-over-Unit is covered too — the #1721/#1733 nominal-Unit case.)
                emit(db, *s, slots, base, high, scratch_ty, layout, out)?;
                if valtype_of(&crate::infer::type_of(db, *s)).is_some() {
                    out.push(Lir::Drop);
                }
            }
            emit(db, tail, slots, base, high, scratch_ty, layout, out)
        }
        // RUNTIME STRING/SYMBOL ORDERING — `<`/`<=`/`>`/`>=` on two String/Symbol byte leaves, compared
        // CONTENT-LEXICOGRAPHICALLY (core-semantics.md §Compound Ordering / 17-symbols §order). Walk both
        // buffers byte-by-byte via `bytes-get` (which reads the i-th LOGICAL byte, transparently flattening a
        // rope — so NO pre-compaction is needed, unlike `ValueEq`'s physical `champ_eq`): at the first
        // differing byte the smaller-byte string is Less; if one is a proper prefix of the other, the SHORTER
        // is Less. Produce a three-way result `res ∈ {-1,0,1}` (an i32), then map `op` to the bool. HASH-
        // NEUTRAL: only `bytes-len`/`bytes-get`, both already-exported (the ops `String.at`/scalar-len use).
        Core::StrCmp { op, lhs, rhs } => {
            // Operands are String/Symbol HANDLES; an OWNED temporary (a `String.concat` result, say) must be
            // dropped after the borrowing walk, a borrowed param/local left to its owner.
            let reclaim_l = matches!(heap_operand_ownership(db, lhs), Ok(HandleOwnership::Owned));
            let reclaim_r = matches!(heap_operand_ownership(db, rhs), Ok(HandleOwnership::Owned));
            let sa = *high; // lhs handle
            let sb = *high + 1; // rhs handle
            let ia = *high + 2; // loop index
            let minl = *high + 3; // min(len_a, len_b)
            let la = *high + 4; // len_a
            let lb = *high + 5; // len_b
            let res = *high + 6; // three-way result {-1,0,1}
            *high = res + 1;
            scratch_ty.insert(sa, ValType::I32);
            scratch_ty.insert(sb, ValType::I32);
            for s in [ia, minl, la, lb, res] {
                scratch_ty.insert(s, ValType::I32);
            }
            // Evaluate both operands (Cadenza order: lhs then rhs) into the handle slots.
            emit(db, lhs, slots, base + 7, high, scratch_ty, layout, out)?;
            out.push(Lir::LocalSet(sa));
            emit(db, rhs, slots, base + 7, high, scratch_ty, layout, out)?;
            out.push(Lir::LocalSet(sb));
            // la = bytes-len(sa); lb = bytes-len(sb); minl = min(la, lb) (both borrows).
            out.push(Lir::LocalGet(sa));
            out.push(Lir::CallImport(OP_BYTES_LEN));
            out.push(Lir::LocalSet(la));
            out.push(Lir::LocalGet(sb));
            out.push(Lir::CallImport(OP_BYTES_LEN));
            out.push(Lir::LocalSet(lb));
            // minl = la < lb ? la : lb  (unsigned; lengths are non-negative)
            out.push(Lir::LocalGet(la));
            out.push(Lir::LocalGet(lb));
            out.push(Lir::LocalGet(la));
            out.push(Lir::LocalGet(lb));
            out.push(Lir::I32LtU);
            out.push(Lir::Select); // [ (la<lb ? la : lb) ]
            out.push(Lir::LocalSet(minl));
            out.push(Lir::ConstI32(0));
            out.push(Lir::LocalSet(ia)); // i = 0
            out.push(Lir::ConstI32(0));
            out.push(Lir::LocalSet(res)); // res = 0 (Equal so far)
            // block { loop { br_out if i>=minl; ca=get(sa,i); cb=get(sb,i);
            //   if ca!=cb { res = (ca<cb? -1 : 1); br_out }; i++; br loop } }
            out.push(Lir::Block(BlockType::Empty)); // $done
            out.push(Lir::Loop(BlockType::Empty)); // $scan
            out.push(Lir::LocalGet(ia));
            out.push(Lir::LocalGet(minl));
            out.push(Lir::I32GeU);
            out.push(Lir::BrIf(1)); // i >= minl → $done
            // ca, cb
            out.push(Lir::LocalGet(sa));
            out.push(Lir::LocalGet(ia));
            out.push(Lir::CallImport(OP_BYTES_GET)); // [ca]
            out.push(Lir::LocalGet(sb));
            out.push(Lir::LocalGet(ia));
            out.push(Lir::CallImport(OP_BYTES_GET)); // [ca, cb]
            // if ca != cb: set res and break. Keep ca,cb on stack for the compare; recompute via slots is
            // avoided by duplicating through the byte reads — read ca,cb once more is simpler, but we already
            // popped; instead compute (ca<cb ? -1 : 1) and whether they differ from the two values on stack.
            // Stack has [ca, cb]. We need both (ca!=cb) and (ca<cb). Spill to two temp slots.
            {
                let cb_slot = *high;
                let ca_slot = *high + 1;
                *high = ca_slot + 1;
                scratch_ty.insert(cb_slot, ValType::I32);
                scratch_ty.insert(ca_slot, ValType::I32);
                out.push(Lir::LocalSet(cb_slot)); // [ca]
                out.push(Lir::LocalSet(ca_slot)); // []
                // if ca != cb { res = (ca <u cb ? -1 : 1); br $done }
                out.push(Lir::LocalGet(ca_slot));
                out.push(Lir::LocalGet(cb_slot));
                out.push(Lir::I32Ne);
                out.push(Lir::If(BlockType::Empty));
                out.push(Lir::ConstI32(-1));
                out.push(Lir::ConstI32(1));
                out.push(Lir::LocalGet(ca_slot));
                out.push(Lir::LocalGet(cb_slot));
                out.push(Lir::I32LtU);
                out.push(Lir::Select); // [ (ca<cb ? -1 : 1) ]
                out.push(Lir::LocalSet(res));
                out.push(Lir::Br(2)); // → $done (out of the `if` and the `loop`)
                out.push(Lir::End); // end if
            }
            // i++
            out.push(Lir::LocalGet(ia));
            out.push(Lir::ConstI32(1));
            out.push(Lir::I32Add);
            out.push(Lir::LocalSet(ia));
            out.push(Lir::Br(0)); // → $scan
            out.push(Lir::End); // end $scan
            out.push(Lir::End); // end $done
            // If no differing byte (res still 0), the shorter string is Less: res = la<lb ? -1 : (la>lb ? 1 : 0).
            out.push(Lir::LocalGet(res));
            out.push(Lir::I32Eqz);
            out.push(Lir::If(BlockType::Empty));
            // res = (la != lb) ? (la <u lb ? -1 : 1) : 0
            out.push(Lir::ConstI32(-1));
            out.push(Lir::ConstI32(1));
            out.push(Lir::LocalGet(la));
            out.push(Lir::LocalGet(lb));
            out.push(Lir::I32LtU);
            out.push(Lir::Select); // [ prev = (la<lb ? -1 : 1) ]
            out.push(Lir::ConstI32(0));
            out.push(Lir::LocalGet(la));
            out.push(Lir::LocalGet(lb));
            out.push(Lir::I32Ne);
            // stack [prev, 0, la!=lb] → select = (la!=lb ? prev : 0) = (la==lb ? 0 : (la<lb ? -1 : 1)).
            out.push(Lir::Select);
            out.push(Lir::LocalSet(res));
            out.push(Lir::End);
            // Reclaim owned-temporary operands now that both walks are done.
            if reclaim_l {
                out.push(Lir::LocalGet(sa));
                out.push(Lir::CallImport(OP_DROP));
            }
            if reclaim_r {
                out.push(Lir::LocalGet(sb));
                out.push(Lir::CallImport(OP_DROP));
            }
            // Map the three-way `res` to the boolean the op wants: Lt res<0, Le res<=0, Gt res>0, Ge res>=0.
            out.push(Lir::LocalGet(res));
            out.push(Lir::ConstI32(0));
            match op {
                Prim::Lt => out.push(Lir::I32LtS),
                Prim::Le => out.push(Lir::I32LeS),
                Prim::Gt => out.push(Lir::I32GtS),
                Prim::Ge => out.push(Lir::I32GeS),
                _ => {
                    return Err(Reject::decline("StrCmp carries a non-ordering prim"));
                }
            }
            Ok(())
        }
        // The `?`/try boundary block + break are the `block`/`br` emit (BRICK 3): a `Core::Block` emits a
        // wasm `block` whose result type is `T_B`'s core repr, with each contained `Core::Break` emitting a
        // `br` to that block's label. BRICK 1 lays down the node + its non-emit arms; until BRICK 3 fills
        // the `block`/`br` bytes, emitting one is a clean decline (never wrong code).
        Core::Block { .. } | Core::Break { .. } => Err(Reject::decline(
            "the `?`/try boundary block/break is not supported on the wasm backend (the block/br lowering is unimplemented)",
        )),
        // A poison that reached selection is an unconditionally-reached fault; the poison collector
        // surfaces it before emission, so reaching here is a decline rather than emitted code.
        Core::Poison(reject) => Err(reject),
    }
}
