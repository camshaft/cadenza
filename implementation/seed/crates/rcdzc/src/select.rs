//! `select : Mir → Lir` — instruction selection (target-independent → the stock-wasm rung),
//! per function.
//!
//! Mirrors the selection step of `cdzc/50-compile.cdz`. Produces a `SelectedFunc` per module
//! function: its flat `Vec<Lir>` body plus the valtypes of its declared locals (the slots BEYOND the
//! parameters — `let`-bound locals and arithmetic scratch). A function body is a sequence over the
//! stack + locals model, so the checked-arithmetic overflow guard is a RUN of flat instructions over
//! scratch locals (the IDEAL trapping sequence, the corpus oracle), not a helper call.
//!
//! Local layout: the parameters are wasm locals `0..arity` (resolve assigned them ids `0..arity`, so
//! a param's resolve-id IS its wasm slot). `let`-locals and scratch claim fresh slots from `arity`
//! upward; `declared` records their valtypes for the code section's local-declaration vector.

use crate::heap_envelope::himport;
use crate::ir::{ArithOp, BitOp, BlockType, CmpOp, HeapIntrinsic, Intrinsic, Lir, Mir, MirFunc, MirModule, ShiftOp, ValType};
use crate::layout::Layout;
use crate::ty::Ty;
use std::collections::HashMap;

/// The himport op that BOXES a scalar of this type into a heap value (for storing as a tuple/array
/// element), or `None` if the value is already a heap handle (a compound needs no boxing).
fn box_op(ty: &Ty) -> Option<u32> {
    match ty {
        Ty::Int => Some(himport::BOX_INT),
        Ty::Bool => Some(himport::BOX_BOOL),
        Ty::Tuple(_) | Ty::Record(_) | Ty::List(_) | Ty::Map(..) | Ty::Set(_) | Ty::Sum { .. } | Ty::Bytes | Ty::String => None, // a compound is already a handle
        // Type is compile-time-only, erased before runtime — should never reach select.
        Ty::Unit | Ty::Var(_) | Ty::Param(_) | Ty::Fn(..) | Ty::Type => None,
    }
}

/// The himport op that UNBOXES a heap value back to a scalar of this type (projecting a tuple
/// element), or `None` if the element is itself a compound handle.
fn unbox_op(ty: &Ty) -> Option<u32> {
    match ty {
        Ty::Int => Some(himport::GET_INT),
        Ty::Bool => Some(himport::GET_BOOL),
        Ty::Tuple(_) | Ty::Record(_) | Ty::List(_) | Ty::Map(..) | Ty::Set(_) | Ty::Sum { .. } | Ty::Bytes | Ty::String => None,
        // Type is compile-time-only, erased before runtime — should never reach select.
        Ty::Unit | Ty::Var(_) | Ty::Param(_) | Ty::Fn(..) | Ty::Type => None,
    }
}

/// The discriminant a lowered PATTERN guards on: a constructor pattern (`Mir::Sum{disc,..}`) matches
/// that discriminant; a binder/wildcard is a CATCH-ALL (no guard → `None`); a literal pattern guards by
/// value (a later phase — for now, treated as unconditional). Drives the `emit_match` cascade.
fn pattern_disc(pat: &Mir) -> Option<u32> {
    match pat {
        Mir::Sum { disc, .. } => Some(*disc),
        _ => None,
    }
}

/// A selected function: its flat body, the value types of its DECLARED locals (beyond the params, in
/// slot order), the parameter value types, and the return type (for the type section).
pub struct SelectedFunc {
    pub params: Vec<ValType>, // per parameter, in order
    pub ret: Ty,
    pub code: Vec<Lir>,
    pub declared: Vec<ValType>, // per declared (non-param) local, in slot order
}

/// A selected module: each function selected. The boundary surface (which functions are exported and
/// under what name/ABI) lives in the `Layout`, threaded separately into `serialize` — not re-carried
/// here.
pub struct SelectedModule {
    pub funcs: Vec<SelectedFunc>,
}

/// Select the whole module against its `layout` — every user-function call is resolved to its
/// ABSOLUTE wasm index here, so `Lir::Call` is concrete downstream. Only functions the layout marks
/// EMITTED (reachable from an export — `layout.order`) are selected; a dead function (a folded-away
/// module helper / record value-def, which may hold a non-emittable `FuncRef`) gets a placeholder that
/// is never serialized. `SelectedModule.funcs` stays indexed by original function index (serialize
/// picks the emitted ones via `layout.order`).
pub fn select_module(module: MirModule, layout: &Layout) -> Result<SelectedModule, String> {
    let emitted: std::collections::BTreeSet<usize> = layout.order.iter().copied().collect();
    let funcs = module
        .funcs
        .into_iter()
        .enumerate()
        .map(|(i, f)| {
            if emitted.contains(&i) {
                select_func(f, layout)
            } else {
                // Dead function — not emitted; a placeholder keeps the index dense.
                Ok(SelectedFunc { params: Vec::new(), ret: crate::ty::Ty::Unit, code: Vec::new(), declared: Vec::new() })
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SelectedModule { funcs })
}

fn select_func(f: MirFunc, layout: &Layout) -> Result<SelectedFunc, String> {
    // A parameter occupies a wasm local slot only if it has a value type. A UNIT parameter (the ignored
    // `_` a unit-wrapped nullary module function carries) is zero-width — it takes NO slot; the
    // following params shift down. `Unit` here is not "unresolved" (that is a `Var`, reported below).
    let mut params: Vec<ValType> = Vec::new();
    let mut slot_of: HashMap<u32, u32> = HashMap::new();
    for (i, t) in f.params.iter().enumerate() {
        match t.core_valtype() {
            Some(vt) => {
                let slot = params.len() as u32;
                slot_of.insert(i as u32, slot); // resolve-id i → its wasm slot
                params.push(vt);
            }
            None if matches!(t, Ty::Unit) => { /* zero-width unit param — no slot */ }
            None => return Err("parameter type unresolved".to_string()),
        }
    }
    // The next free slot starts after the (value-typed) params.
    let mut sel = Sel { next_slot: params.len() as u32, declared: Vec::new(), slot_of, layout };
    let mut code = Vec::new();
    sel.emit(&f.body, &mut code)?;
    Ok(SelectedFunc { params, ret: f.ret, code, declared: sel.declared })
}

struct Sel<'a> {
    next_slot: u32,
    /// Value type per DECLARED (non-param) local, in the order allocated (slot = params + this index).
    declared: Vec<ValType>,
    /// resolve-local id → wasm slot index.
    slot_of: HashMap<u32, u32>,
    /// The module layout — resolves a user-function module index to its absolute wasm index.
    layout: &'a Layout,
}

impl Sel<'_> {
    /// Allocate a fresh declared local of the given value type; return its wasm slot index.
    fn alloc(&mut self, valtype: ValType) -> u32 {
        let slot = self.next_slot;
        self.next_slot += 1;
        self.declared.push(valtype);
        slot
    }

    fn emit(&mut self, mir: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        match mir {
            Mir::Int(n) => {
                out.push(Lir::ConstI64(*n));
                Ok(())
            }
            Mir::Bool(b) => {
                out.push(Lir::ConstI32(if *b { 1 } else { 0 }));
                Ok(())
            }
            // A string literal — a Bytes-backed UTF-8 heap leaf: `bytes-alloc(len)` then per NFC byte
            // `bytes-set(buf, i, byte)` (no range guard — a UTF-8 byte is statically 0..=255). Leaves the
            // handle. Same build as a literal `Bytes.of`, but its `Ty::String` renders it `"…"`.
            Mir::Str(s) => self.emit_str(s, out),
            Mir::Unit => {
                // The unit value occupies no machine slot — it pushes nothing. A unit-returning
                // function has no wasm result (see serialize's functype), and a unit `let`-value binds
                // no slot (see the Let arm), so emitting nothing is correct.
                Ok(())
            }
            Mir::Local(id) => {
                let slot = *self
                    .slot_of
                    .get(id)
                    .ok_or_else(|| format!("local {id} referenced before binding"))?;
                out.push(Lir::LocalGet(slot));
                Ok(())
            }
            Mir::Call { func, args } => {
                for a in args {
                    self.emit(a, out)?;
                }
                // Resolve the callee's ABSOLUTE wasm index up front (via the layout), so `Lir::Call`
                // is concrete — no serialize-time remap, no import-vs-user ambiguity.
                out.push(Lir::Call(self.layout.abs[*func]));
                Ok(())
            }
            // A function value that SURVIVED folding is not runtime-emittable — there are no
            // first-class runtime function values yet. The fold reduces every module
            // projection-and-apply to a direct `Call`; a residual means a function value used as
            // runtime data — a clean decline, never miscompiled.
            Mir::FuncRef(_) => Err("a function value cannot yet cross to run time".to_string()),
            // A bare intrinsic value (unapplied) is likewise not runtime-emittable.
            Mir::Intrinsic(_) => Err("a built-in operation value cannot yet cross to run time".to_string()),
            // A bare constructor value (unapplied) is not runtime-emittable — the fold reduces every
            // application to a `Mir::Sum`; a survivor is a ctor used as runtime data (a clean decline).
            Mir::Ctor { .. } => Err("a constructor value cannot yet cross to run time".to_string()),
            // A bare lambda (unapplied) is not runtime-emittable — the fold β-reduces every application;
            // a survivor is a lambda that escaped (stored in runtime data, selected by a runtime `if`/
            // `match` and then applied, or applied through a recursive HOF). Runtime closures are Increment B.
            Mir::Lambda { .. } => Err("a closure cannot yet cross to run time (runtime closures are a later increment)".to_string()),
            // A wildcard only appears in a lowered PATTERN (consumed by `emit_match`), never in emit
            // position — reaching here is a compiler bug surfaced as a decline.
            Mir::Wildcard => Err("a wildcard cannot be emitted as a value".to_string()),
            // A runtime trap → `unreachable` (the intended halt for a match's absent arm / `expect`).
            Mir::Trap(_) => {
                out.push(Lir::Unreachable);
                Ok(())
            }
            // Construct a heap sum `(disc, payload)` via `sum-new`.
            Mir::Sum { disc, payload_ty, payload, .. } => self.emit_sum(*disc, payload_ty, payload, out),
            // Dispatch on a sum's discriminant — a `sum-disc` cascade binding each arm's payload.
            Mir::Match { scrutinee, scrut_ty, arms, ty } => {
                self.emit_match(scrutinee, scrut_ty, arms, ty, out)
            }
            // An application: an applied INTRINSIC lowers to its wasm instruction sequence (the id→
            // instruction translation — the whole point of intrinsics at LIR). Any other residual
            // application (a runtime closure / incomplete currying) declines.
            Mir::Apply { func, args } => match func.as_ref() {
                Mir::Intrinsic(op) => self.emit_intrinsic(*op, args, out),
                _ => Err("applying a non-constant function value (a runtime closure or partial application that never reached full arity at compile time) is not yet supported (Increment B)".to_string()),
            },
            Mir::Tuple(elems) => self.emit_product(elems, out),
            Mir::List(elems) => self.emit_list(elems, out),
            Mir::Map(entries) => self.emit_map(entries, out),
            Mir::Set(elems) => self.emit_set(elems, out),
            Mir::HeapOp { op, args } => self.emit_heap_op(*op, args, out),
            Mir::Proj { slot, elem_ty, operand } => {
                // The ONE projection for both tuple and record: arr-get(operand, slot), then unbox by
                // the element's scalar kind. Named vs positional is already resolved to `slot` at
                // lowering — select does not know or care which compound shape this came from.
                self.emit(operand, out)?;
                out.push(Lir::ConstI32(*slot as i32));
                // Heap imports occupy absolute indices `0..RT_N_IMPORTS`, so a himport index IS its
                // absolute wasm function index — `Lir::Call` stays concrete.
                out.push(Lir::Call(himport::ARR_GET));
                if let Some(unbox) = unbox_op(elem_ty) {
                    out.push(Lir::Call(unbox));
                }
                Ok(())
            }
            Mir::Arith(op, a, b) => self.emit_checked_arith(*op, a, b, out),
            Mir::Bit(op, a, b) => {
                // Raw i64 bit ops; `div_s`/`rem_s` trap natively on /0 and Int64.min/-1.
                self.emit(a, out)?;
                self.emit(b, out)?;
                out.push(match op {
                    BitOp::And => Lir::I64And,
                    BitOp::Or => Lir::I64Or,
                    BitOp::Xor => Lir::I64Xor,
                    BitOp::Div => Lir::I64DivS,
                    BitOp::Rem => Lir::I64RemS,
                });
                Ok(())
            }
            Mir::Shift(op, a, b) => self.emit_shift(*op, a, b, out),
            Mir::Cmp { op, operand_ty, a, b } => {
                // Unit equality: there is exactly one unit value, so `(= unit unit)` is always true and
                // neither operand pushes a machine value. Emit the constant `true` directly (do NOT
                // emit the operands then a comparison — there is nothing on the stack to compare).
                if matches!(operand_ty, Ty::Unit) {
                    return match op {
                        CmpOp::Eq => {
                            out.push(Lir::ConstI32(1));
                            Ok(())
                        }
                        _ => Err("unit supports only equality, not ordering".to_string()),
                    };
                }
                self.emit(a, out)?;
                self.emit(b, out)?;
                // Pick the machine comparison by the OPERAND type: Int → signed i64; Bool → unsigned
                // i32 (Bool is i32 with false=0 < true=1, so its total order is the unsigned compare).
                let instr = match operand_ty {
                    Ty::Int => match op {
                        CmpOp::Lt => Lir::I64LtS,
                        CmpOp::Gt => Lir::I64GtS,
                        CmpOp::Le => Lir::I64LeS,
                        CmpOp::Ge => Lir::I64GeS,
                        CmpOp::Eq => Lir::I64Eq,
                    },
                    Ty::Bool => match op {
                        CmpOp::Lt => Lir::I32LtU,
                        CmpOp::Gt => Lir::I32GtU,
                        CmpOp::Le => Lir::I32LeU,
                        CmpOp::Ge => Lir::I32GeU,
                        CmpOp::Eq => Lir::I32Eq,
                    },
                    other => {
                        return Err(format!("comparison on unsupported operand type {other:?}"))
                    }
                };
                out.push(instr);
                Ok(())
            }
            Mir::If { cond, then_, else_, ty } => {
                self.emit(cond, out)?;
                // The `if`'s result type is the block's type: a value-producing block for a scalar
                // result, or an EMPTY block for a Unit result (both branches push nothing).
                let blockty = match ty {
                    Ty::Unit => BlockType::Empty,
                    _ => BlockType::Val(
                        ty.core_valtype().ok_or_else(|| "if result type unresolved".to_string())?,
                    ),
                };
                out.push(Lir::If(blockty));
                self.emit(then_, out)?;
                out.push(Lir::Else);
                self.emit(else_, out)?;
                out.push(Lir::End);
                Ok(())
            }
            Mir::Let { id, value_ty, value, body } => {
                // A Unit-valued binding occupies no slot (unit pushes nothing); emit the value for its
                // effects and bind nothing. A scalar value claims a fresh local.
                if matches!(value_ty, Ty::Unit) {
                    self.emit(value, out)?;
                    return self.emit(body, out);
                }
                let vt = value_ty.core_valtype().ok_or_else(|| "let value type unresolved".to_string())?;
                self.emit(value, out)?;
                let slot = self.alloc(vt);
                out.push(Lir::LocalSet(slot));
                self.slot_of.insert(*id, slot);
                self.emit(body, out)
            }
            // A type-value or type-constructor that survived the fold/fence is a compiler bug — should have
            // been rejected by the erasure fence in fold. Decline rather than emit invalid wasm.
            Mir::TypeVal(_) => Err("a type-value reached select (erasure fence should have rejected it)".to_string()),
            Mir::TypeCtor(_) => Err("a type constructor reached select (should have β-reduced in fold)".to_string()),
            Mir::Error(_) => {
                out.push(Lir::Unreachable);
                Ok(())
            }
        }
    }

    /// Emit a heap PRODUCT construction (a tuple OR a record — a record's fields were sorted into slot
    /// order at lowering, so construction is identical): `arr = arr-alloc(arity)`, then per element
    /// `arr-set(arr, i, box(elem))` (dropping the array handle `arr-set` returns), leaving the array
    /// handle. Boxes each element by its solved type; a compound element is already a handle (no box).
    fn emit_product(&mut self, elems: &[(Ty, Mir)], out: &mut Vec<Lir>) -> Result<(), String> {
        let arr = self.alloc(ValType::I32); // the array handle
        out.push(Lir::ConstI32(elems.len() as i32));
        out.push(Lir::Call(himport::ARR_ALLOC));
        out.push(Lir::LocalSet(arr));
        for (i, (ety, elem)) in elems.iter().enumerate() {
            out.push(Lir::LocalGet(arr));
            out.push(Lir::ConstI32(i as i32));
            self.emit(elem, out)?;
            if let Some(box_fn) = box_op(ety) {
                out.push(Lir::Call(box_fn));
            }
            out.push(Lir::Call(himport::ARR_SET));
            out.push(Lir::Drop); // arr-set returns the array; keep our local, drop the returned copy
        }
        out.push(Lir::LocalGet(arr));
        Ok(())
    }

    /// Emit a heap LIST construction: `vec-empty()`, then per element `vec-push(list, box(elem))` —
    /// each `vec-push` consumes the list-so-far + the boxed element and RETURNS the new list, which
    /// stays on the stack as the next push's list operand, so the elements accumulate left-to-right and
    /// the final list handle is left on the stack. Backed by the runtime's persistent sequence (`vec-*`,
    /// himports 24/27), NOT the fixed `arr-*` a tuple uses — the same representation `List.push` grows,
    /// so a literal and a grown list read alike. Boxes each element by its solved type (a compound
    /// element is already a handle).
    fn emit_list(&mut self, elems: &[(Ty, Mir)], out: &mut Vec<Lir>) -> Result<(), String> {
        // list = vec-empty()
        out.push(Lir::Call(himport::VEC_EMPTY));
        for (ety, elem) in elems {
            // list-so-far is on the stack; push the boxed element, then vec-push (leaves the new list).
            self.emit(elem, out)?;
            if let Some(box_fn) = box_op(ety) {
                out.push(Lir::Call(box_fn));
            }
            out.push(Lir::Call(himport::VEC_PUSH));
        }
        Ok(())
    }

    /// Emit a `(map …)` literal → the runtime CHAMP: `map-empty`, then per entry `map-insert(m, boxed-k,
    /// boxed-v)` (each scalar boxed by its solved type; a compound is already a handle), leaving the map
    /// handle. The empty `(map)` is just `map-empty`.
    fn emit_map(&mut self, entries: &[((Ty, Mir), (Ty, Mir))], out: &mut Vec<Lir>) -> Result<(), String> {
        out.push(Lir::Call(himport::MAP_EMPTY));
        for ((kty, k), (vty, v)) in entries {
            // map-so-far is on the stack; push boxed key, boxed value, then map-insert (leaves new map).
            self.emit(k, out)?;
            if let Some(box_fn) = box_op(kty) {
                out.push(Lir::Call(box_fn));
            }
            self.emit(v, out)?;
            if let Some(box_fn) = box_op(vty) {
                out.push(Lir::Call(box_fn));
            }
            out.push(Lir::Call(himport::MAP_INSERT));
        }
        Ok(())
    }

    /// Emit a `(set …)` literal → the runtime CHAMP set: `set-empty`, then per element `set-insert(s,
    /// boxed-e)`, leaving the set handle. The empty `(set)` is just `set-empty`.
    fn emit_set(&mut self, elems: &[(Ty, Mir)], out: &mut Vec<Lir>) -> Result<(), String> {
        out.push(Lir::Call(himport::SET_EMPTY));
        for (ety, elem) in elems {
            self.emit(elem, out)?;
            if let Some(box_fn) = box_op(ety) {
                out.push(Lir::Call(box_fn));
            }
            out.push(Lir::Call(himport::SET_INSERT));
        }
        Ok(())
    }

    /// Emit a [`HeapIntrinsic`] — the ops that BOX a scalar argument by its SOLVED type (threaded from
    /// `lower` in the `Mir::HeapOp` args), which the argument's `Mir` shape alone cannot decide. The
    /// COLLECTION operand (list/map/set) is CONSUMED by the runtime op (FBIP in-place at rc==1), so a
    /// shared `Local` collection is `dup`'d first; a KEY/VALUE/ELEMENT is emitted then boxed by its type
    /// (a scalar Int/Bool → box-int/box-bool; a compound is already a handle). `Map.lookup` guards the
    /// runtime's NULL-on-absent into an `Option`; the rest map to one himport call.
    fn emit_heap_op(&mut self, op: HeapIntrinsic, args: &[(Ty, Mir)], out: &mut Vec<Lir>) -> Result<(), String> {
        // Emit an argument, boxing it by its solved type (a scalar → its box himport; a handle → none).
        let boxed = |s: &mut Self, i: usize, out: &mut Vec<Lir>| -> Result<(), String> {
            let (ty, m) = &args[i];
            s.emit(m, out)?;
            if let Some(box_fn) = box_op(ty) {
                out.push(Lir::Call(box_fn));
            }
            Ok(())
        };
        match op {
            // `List.push list elem` → `vec-push`: the (dup'd) list, then the boxed element.
            HeapIntrinsic::ListPush => {
                self.emit_consuming_operand(&args[0].1, out)?;
                boxed(self, 1, out)?;
                out.push(Lir::Call(himport::VEC_PUSH));
                Ok(())
            }
            // `Map.insert m k v` → `map-insert`: the (dup'd) map, boxed key, boxed value.
            HeapIntrinsic::MapInsert => {
                self.emit_consuming_operand(&args[0].1, out)?;
                boxed(self, 1, out)?;
                boxed(self, 2, out)?;
                out.push(Lir::Call(himport::MAP_INSERT));
                Ok(())
            }
            // `Map.lookup m k` → `Option v`: `map-lookup` returns the value handle, or NULL(0) if absent.
            // Guard: `if lookup != 0 { Some(<handle unboxed to v>) } else { None(unit) }`.
            HeapIntrinsic::MapLookup => self.emit_map_lookup(args, out),
            // `Map.remove m k` → `map-remove`: the (dup'd) map, boxed key.
            HeapIntrinsic::MapRemove => {
                self.emit_consuming_operand(&args[0].1, out)?;
                boxed(self, 1, out)?;
                out.push(Lir::Call(himport::MAP_REMOVE));
                Ok(())
            }
            // `Set.of (list …)` → `set-empty` then `set-insert` per boxed element. LITERAL list only
            // (unrolled); a runtime list arg DECLINES (needs a loop `Lir` cannot express).
            HeapIntrinsic::SetOf => match &args[0].1 {
                Mir::List(elems) => {
                    out.push(Lir::Call(himport::SET_EMPTY));
                    for (ety, elem) in elems {
                        self.emit(elem, out)?;
                        if let Some(box_fn) = box_op(ety) {
                            out.push(Lir::Call(box_fn));
                        }
                        out.push(Lir::Call(himport::SET_INSERT));
                    }
                    Ok(())
                }
                _ => Err("Set.of of a non-literal list is a later phase (needs a runtime loop)".to_string()),
            },
            // `Set.insert s e` → `set-insert`: the (dup'd) set, boxed element.
            HeapIntrinsic::SetInsert => {
                self.emit_consuming_operand(&args[0].1, out)?;
                boxed(self, 1, out)?;
                out.push(Lir::Call(himport::SET_INSERT));
                Ok(())
            }
            // `Set.contains s e` → `set-contains` (→ i32 bool, borrows both): the set handle, boxed element.
            HeapIntrinsic::SetContains => {
                self.emit(&args[0].1, out)?;
                boxed(self, 1, out)?;
                out.push(Lir::Call(himport::SET_CONTAINS));
                Ok(())
            }
            // `Set.remove s e` → `set-remove`: the (dup'd) set, boxed element.
            HeapIntrinsic::SetRemove => {
                self.emit_consuming_operand(&args[0].1, out)?;
                boxed(self, 1, out)?;
                out.push(Lir::Call(himport::SET_REMOVE));
                Ok(())
            }
        }
    }

    /// Emit `Map.lookup m k` → `Option v`. `map-lookup(m, boxed-k)` returns the stored value handle or
    /// NULL(0) if absent. `if handle != 0 { Some(handle) } else { None(unit) }` — the value handle IS
    /// the `Some` payload (already boxed on insert; the match binder unboxes it by `v`'s type, like
    /// `List.at`). Some=disc 0, None=disc 1 (Option declaration order).
    fn emit_map_lookup(&mut self, args: &[(Ty, Mir)], out: &mut Vec<Lir>) -> Result<(), String> {
        // v = map-lookup(m, boxed-k)  — m borrowed (no dup needed; lookup does not consume).
        self.emit(&args[0].1, out)?;
        self.emit(&args[1].1, out)?;
        if let Some(box_fn) = box_op(&args[1].0) {
            out.push(Lir::Call(box_fn));
        }
        out.push(Lir::Call(himport::MAP_LOOKUP));
        let v = self.alloc(ValType::I32);
        out.push(Lir::LocalSet(v));
        // if v != 0 (present) → Some(v) else None(unit).
        out.push(Lir::LocalGet(v));
        out.push(Lir::ConstI32(0));
        out.push(Lir::I32Eq);
        out.push(Lir::I32Eqz); // v != 0
        out.push(Lir::If(BlockType::Val(ValType::I32)));
        out.push(Lir::ConstI32(0)); // Some disc
        out.push(Lir::LocalGet(v));
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::Else);
        out.push(Lir::ConstI32(1)); // None disc
        out.push(Lir::ConstI32(0));
        out.push(Lir::Call(himport::ARR_ALLOC));
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::End);
        Ok(())
    }

    /// Emit an operand that a runtime op CONSUMES (`vec-push`/`vec-concat`/`bytes-concat`/`bytes-slice`
    /// take ownership of their sequence operand — the runtime does FBIP in-place mutation when the
    /// handle's rc == 1). If the operand is a bare `Local`, it may be USED AGAIN in the same scope
    /// (shared), so bump its refcount with `dup` BEFORE the consuming op — the runtime then path-copies
    /// (rc > 1) instead of mutating the shared value in place, so the other use observes it unchanged.
    /// A conservative Perceus: it may `dup` a last-use local too (rc 2, a missed FBIP + a leak — `drop`
    /// is a no-op in the current runtime phase, so never a double-free), but it NEVER under-dups (which
    /// would corrupt a shared value). A non-`Local` operand is a freshly-built value (unshared) — no dup.
    /// (A precise per-scope linear-use analysis is the eventual Perceus; this is the safe subset.)
    fn emit_consuming_operand(&mut self, operand: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        if let Mir::Local(id) = operand {
            out.push(Lir::LocalGet(*id));
            out.push(Lir::Call(himport::DUP));
        }
        self.emit(operand, out)
    }

    /// Emit a heap SUM construction `(disc, payload)` via `sum-new`: push the discriminant, emit the
    /// payload (boxed by its type; a Unit payload is the empty array `arr-alloc(0)`, the unit value),
    /// then `sum-new(disc, payload) -> handle`. himport `SUM_NEW`=10.
    fn emit_sum(&mut self, disc: u32, payload_ty: &Ty, payload: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        out.push(Lir::ConstI32(disc as i32));
        if matches!(payload_ty, Ty::Unit) {
            // A nullary variant's payload is unit — the empty array (same as the empty tuple).
            out.push(Lir::ConstI32(0));
            out.push(Lir::Call(himport::ARR_ALLOC));
        } else {
            self.emit(payload, out)?;
            if let Some(box_fn) = box_op(payload_ty) {
                out.push(Lir::Call(box_fn));
            }
        }
        out.push(Lir::Call(himport::SUM_NEW));
        Ok(())
    }

    /// Emit a `(match scrutinee arms…)` — a `sum-disc` cascade. Store the scrutinee handle in a local;
    /// for each arm emit an `if sum-disc(h) == <arm disc>` whose THEN binds the arm's payload (via
    /// `sum-payload(h)` + the pattern's binders) and emits the body, and whose ELSE is the next arm. The
    /// last arm is unconditional (its guard omitted) — exhaustiveness (checked at lowering) guarantees
    /// it always matches, so no fall-through trap. Nested/tuple patterns bind by walking the payload.
    fn emit_match(&mut self, scrutinee: &Mir, scrut_ty: &Ty, arms: &[(Mir, Mir)], ty: &Ty, out: &mut Vec<Lir>) -> Result<(), String> {
        // scrutinee handle → a local, re-read per arm.
        self.emit(scrutinee, out)?;
        let h = self.alloc(ValType::I32);
        out.push(Lir::LocalSet(h));
        // A TUPLE-scrutinee match is single-arm DESTRUCTURING (no discriminant): the scrutinee handle IS
        // the tuple `arr`, so bind the single arm's tuple pattern against it directly (the tuple-payload
        // path) and emit the body — no `sum-disc`/`sum-payload`, no cascade. (infer proved one arm whose
        // pattern is a tuple or a catch-all binder.)
        if matches!(scrut_ty, Ty::Tuple(_)) {
            let (pat, body) = &arms[0];
            match pat {
                // The tuple pattern's elements bind by `arr-get` on the scrutinee handle (which IS the
                // tuple array) — exactly the payload-tuple binding path.
                Mir::Tuple(_) => self.bind_payload(h, scrut_ty, pat, out)?,
                // A catch-all binder binds the whole tuple handle.
                Mir::Local(id) => {
                    self.slot_of.insert(*id, h);
                }
                Mir::Wildcard => {}
                _ => return Err("a tuple-scrutinee match arm must be a tuple pattern or a binder".to_string()),
            }
            return self.emit(body, out);
        }
        let block_ty = match ty {
            Ty::Unit => BlockType::Empty,
            _ => BlockType::Val(ty.core_valtype().ok_or_else(|| "match result type unresolved".to_string())?),
        };
        self.emit_arms(h, arms, block_ty, out)
    }

    /// Emit the arm cascade for a match on the sum in local `h`. The first arm whose pattern's
    /// discriminant matches runs; a catch-all (`Wildcard`/`Local`) pattern or the LAST arm is
    /// unconditional. Recurses to build the else-chain.
    fn emit_arms(&mut self, h: u32, arms: &[(Mir, Mir)], block_ty: BlockType, out: &mut Vec<Lir>) -> Result<(), String> {
        match arms {
            [] => {
                // No arm matched — exhaustiveness should prevent this; emit a trap defensively.
                out.push(Lir::Unreachable);
                Ok(())
            }
            [(pat, body)] => {
                // The last arm is unconditional — bind its pattern against `h`, emit the body.
                self.bind_pattern(h, pat, out)?;
                self.emit(body, out)
            }
            [(pat, body), rest @ ..] => {
                // A catch-all pattern (a bare binder / wildcard) is unconditional — no disc guard.
                if let Some(disc) = pattern_disc(pat) {
                    // if sum-disc(h) == disc { bind; body } else { rest }
                    out.push(Lir::LocalGet(h));
                    out.push(Lir::Call(himport::SUM_DISC));
                    out.push(Lir::ConstI32(disc as i32));
                    out.push(Lir::I32Eq);
                    out.push(Lir::If(block_ty));
                    self.bind_pattern(h, pat, out)?;
                    self.emit(body, out)?;
                    out.push(Lir::Else);
                    self.emit_arms(h, rest, block_ty, out)?;
                    out.push(Lir::End);
                    Ok(())
                } else {
                    // A catch-all (binder/wildcard) — matches unconditionally; later arms are dead.
                    self.bind_pattern(h, pat, out)?;
                    self.emit(body, out)
                }
            }
        }
    }

    /// Bind a lowered PATTERN against the sum value in local `h`: a `Sum{payload-pattern}` binds its
    /// payload via `sum-payload(h)` then recurses; a `Local` binds the whole `h` (or the payload it
    /// faces) to that local; a `Wildcard`/literal binds nothing. Called once the arm's disc has matched.
    fn bind_pattern(&mut self, h: u32, pat: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        match pat {
            // A constructor pattern: read the payload handle, bind the inner pattern against it.
            Mir::Sum { payload_ty, payload, .. } => {
                out.push(Lir::LocalGet(h));
                out.push(Lir::Call(himport::SUM_PAYLOAD));
                let payload_h = self.alloc(ValType::I32);
                out.push(Lir::LocalSet(payload_h));
                self.bind_payload(payload_h, payload_ty, payload, out)
            }
            // A bare binder at the top matches the whole sum value.
            Mir::Local(id) => {
                let slot = self.alloc(ValType::I32);
                out.push(Lir::LocalGet(h));
                out.push(Lir::LocalSet(slot));
                self.slot_of.insert(*id, slot);
                Ok(())
            }
            Mir::Wildcard => Ok(()),
            _ => Ok(()),
        }
    }

    /// Bind the PAYLOAD (in local `payload_h`, of type `payload_ty`) against the payload sub-pattern.
    /// A `Local` binder: unbox the payload to its scalar kind (or keep the handle for a compound) into
    /// the binder's slot. A `Tuple` pattern: `arr-get` each element and recurse. `Wildcard`/literal
    /// binds nothing. A nested `Sum` pattern (a ctor under a ctor) recurses via `bind_pattern`.
    fn bind_payload(&mut self, payload_h: u32, payload_ty: &Ty, pat: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        match pat {
            Mir::Wildcard => Ok(()),
            Mir::Local(id) => {
                // Unbox the payload to the binder's kind (a scalar) or keep the handle (a compound).
                match payload_ty.core_valtype() {
                    Some(vt) => {
                        out.push(Lir::LocalGet(payload_h));
                        if let Some(unbox) = unbox_op(payload_ty) {
                            out.push(Lir::Call(unbox));
                        }
                        let slot = self.alloc(vt);
                        out.push(Lir::LocalSet(slot));
                        self.slot_of.insert(*id, slot);
                        Ok(())
                    }
                    // A Unit payload occupies no slot — bind nothing (the binder is never read).
                    None => Ok(()),
                }
            }
            // A tuple payload pattern: the payload handle IS the tuple array; bind each element.
            Mir::Tuple(elems) => {
                for (i, (ety, sub)) in elems.iter().enumerate() {
                    // element handle = arr-get(payload_h, i)
                    out.push(Lir::LocalGet(payload_h));
                    out.push(Lir::ConstI32(i as i32));
                    out.push(Lir::Call(himport::ARR_GET));
                    let elem_h = self.alloc(ValType::I32);
                    out.push(Lir::LocalSet(elem_h));
                    self.bind_payload(elem_h, ety, sub, out)?;
                }
                Ok(())
            }
            // A nested constructor pattern under this payload — the payload IS a sum; recurse.
            Mir::Sum { .. } => self.bind_pattern(payload_h, pat, out),
            // A literal / other pattern binds nothing (its disc/value was the guard).
            _ => Ok(()),
        }
    }

    /// Emit an applied INTRINSIC to its wasm instruction sequence — the ONE id→instruction table (the
    /// point of intrinsics: the built-in operation is an opaque value through HIR/MIR and becomes
    /// opcodes only HERE). A new intrinsic op is a new arm; the runtime semantics live in this table,
    /// not in scattered head-name special-cases. Arg count is guaranteed by inference (the op's
    /// signature).
    fn emit_intrinsic(&mut self, op: Intrinsic, args: &[Mir], out: &mut Vec<Lir>) -> Result<(), String> {
        match op {
            // Wrapping Int64 arithmetic: emit both operands, then the raw i64 op — NO overflow guard
            // (wrapping is defined two's-complement, it never traps), the difference from the checked
            // `+`/`-`/`*` operators which trap.
            Intrinsic::WrappingAdd | Intrinsic::WrappingSub | Intrinsic::WrappingMul => {
                self.emit(&args[0], out)?;
                self.emit(&args[1], out)?;
                out.push(match op {
                    Intrinsic::WrappingAdd => Lir::I64Add,
                    Intrinsic::WrappingSub => Lir::I64Sub,
                    Intrinsic::WrappingMul => Lir::I64Mul,
                    _ => unreachable!("outer arm guarantees a wrapping op"),
                });
                Ok(())
            }
            // `Bytes.len` / `Bytes.concat` — emit the operand handle(s), then the himport (result = an
            // i32 handle / an i64 count-extended... no: bytes-len returns i32 count, extend to i64).
            Intrinsic::BytesLen => {
                self.emit(&args[0], out)?;
                out.push(Lir::Call(himport::BYTES_LEN));
                // bytes-len is i32; the Int64 result must be an i64 — zero-extend (a length is >=0).
                out.push(Lir::I64ExtendI32U);
                Ok(())
            }
            Intrinsic::BytesConcat => {
                // Both operands are CONSUMED by `bytes-concat` — dup a shared `Local` operand first.
                self.emit_consuming_operand(&args[0], out)?;
                self.emit_consuming_operand(&args[1], out)?;
                out.push(Lir::Call(himport::BYTES_CONCAT));
                Ok(())
            }
            // `Bytes.of` — build a byte sequence from a `(list …)` of byte values. This slice handles a
            // LITERAL list arg (unrolled, flat — no runtime loop, which `Lir` cannot yet express): alloc
            // a buffer of the list's length, then per element `bytes-set(buf, i, wrap(range-check(v)))`.
            // Each byte is range-checked 0..=255 at run time (trap outside — a defined trap,
            // 10-bytes.sexp). A non-literal (runtime) list arg DECLINES (needs a runtime loop — later).
            Intrinsic::BytesOf => match &args[0] {
                Mir::List(elems) => self.emit_bytes_of(elems, out),
                _ => Err("Bytes.of of a non-literal list is a later phase (needs a runtime loop)".to_string()),
            },
            // `Int.to-byte` = `n & 255` (low 8 bits) — emit the operand, then `i64.const 255 ; i64.and`.
            Intrinsic::IntToByte => {
                self.emit(&args[0], out)?;
                out.push(Lir::ConstI64(255));
                out.push(Lir::I64And);
                Ok(())
            }
            // `List.len` → `vec-len`(25) then extend the i32 count to the Int64 result.
            Intrinsic::ListLen => {
                self.emit(&args[0], out)?;
                out.push(Lir::Call(himport::VEC_LEN));
                out.push(Lir::I64ExtendI32U);
                Ok(())
            }
            // `List.concat` → `vec-concat`(41): both list handles, each CONSUMED — dup a shared `Local`.
            Intrinsic::ListConcat => {
                self.emit_consuming_operand(&args[0], out)?;
                self.emit_consuming_operand(&args[1], out)?;
                out.push(Lir::Call(himport::VEC_CONCAT));
                Ok(())
            }
            // FALLIBLE reads → an `Option` sum: `if 0<=i<len { Some(elem) } else { None(unit) }`.
            // `Bytes.at`: len=`bytes-len`, get=`bytes-get` (a raw byte i32 → box-int the Int payload).
            // `List.at`: len=`vec-len`, get=`vec-get` (the stored element is ALREADY a boxed handle — no
            // re-box; the `Some` payload is that handle, unboxed by the match binder per its type).
            Intrinsic::BytesAt => self.emit_fallible_at(&args[0], &args[1], himport::BYTES_LEN, himport::BYTES_GET, true, out),
            Intrinsic::ListAt => self.emit_fallible_at(&args[0], &args[1], himport::VEC_LEN, himport::VEC_GET, false, out),
            Intrinsic::BytesSlice => self.emit_bytes_slice(&args[0], &args[1], &args[2], out),
            // `String.from-bytes` → `Option String`: validate the Bytes leaf via the fixed
            // `utf8-valid` helper, then `if valid { Some(buf) } else { None(unit) }`. The String's
            // payload IS the validated Bytes leaf (String is a Bytes-backed leaf).
            Intrinsic::StrFromBytes => self.emit_str_from_bytes(&args[0], out),
            // `Bytes.compact` → `bytes-compact`(31): the (consumed) Bytes handle → a storage-independent
            // copy. Dup a shared `Local` operand (it consumes its buffer).
            Intrinsic::BytesCompact => {
                self.emit_consuming_operand(&args[0], out)?;
                out.push(Lir::Call(himport::BYTES_COMPACT));
                Ok(())
            }
            // `Map.size` → `map-size`(36) then extend the i32 count to Int64.
            Intrinsic::MapSize => {
                self.emit(&args[0], out)?;
                out.push(Lir::Call(himport::MAP_SIZE));
                out.push(Lir::I64ExtendI32U);
                Ok(())
            }
            // `Set.size`/`Set.len` → `set-size`(46+…) then extend to Int64.
            Intrinsic::SetSize => {
                self.emit(&args[0], out)?;
                out.push(Lir::Call(himport::SET_SIZE));
                out.push(Lir::I64ExtendI32U);
                Ok(())
            }
            // Set algebra → `set-union`/`-intersection`/`-difference`: both set handles, each CONSUMED.
            Intrinsic::SetUnion | Intrinsic::SetIntersection | Intrinsic::SetDifference => {
                self.emit_consuming_operand(&args[0], out)?;
                self.emit_consuming_operand(&args[1], out)?;
                out.push(Lir::Call(match op {
                    Intrinsic::SetUnion => himport::SET_UNION,
                    Intrinsic::SetIntersection => himport::SET_INTERSECTION,
                    Intrinsic::SetDifference => himport::SET_DIFFERENCE,
                    _ => unreachable!("outer arm guarantees a set-algebra op"),
                }));
                Ok(())
            }
            // A `Heap` op is lowered to `Mir::HeapOp` (carrying solved arg types) by `lower`, so it
            // never reaches here as an `Apply(Intrinsic)`. A bare/partially-applied one cannot box (no
            // solved types) — decline rather than shape-guess.
            Intrinsic::Heap(_) => Err(
                "a Heap intrinsic must be lowered to Mir::HeapOp (a bare heap-op value is a later phase)"
                    .to_string(),
            ),
        }
    }

    /// Emit `String.from-bytes b` → `Option String`: stash the Bytes handle, call the fixed
    /// `utf8-valid` helper on it; `if valid { Some(buf) } else { None(unit) }`. Some=disc 0, None=disc 1
    /// (Option declaration order). The String payload IS the validated Bytes leaf — no copy.
    fn emit_str_from_bytes(&mut self, seq: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        // The String's payload IS the validated Bytes leaf — `from-bytes` CONSUMES it. Dup a shared
        // `Local` operand so a caller keeping the original Bytes sees it unchanged.
        self.emit_consuming_operand(seq, out)?;
        let h = self.alloc(ValType::I32);
        out.push(Lir::LocalSet(h));
        // valid = utf8-valid(buf)
        out.push(Lir::LocalGet(h));
        out.push(Lir::Call(crate::heap::RT_UTF8_VALID));
        out.push(Lir::If(BlockType::Val(ValType::I32)));
        // then: Some(buf) — disc 0, payload = the validated Bytes leaf.
        out.push(Lir::ConstI32(0));
        out.push(Lir::LocalGet(h));
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::Else);
        // else: None(unit) — disc 1, payload = the empty array (unit).
        out.push(Lir::ConstI32(1));
        out.push(Lir::ConstI32(0));
        out.push(Lir::Call(himport::ARR_ALLOC));
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::End);
        Ok(())
    }

    /// Emit `Bytes.slice b start length` → `Option Bytes`: guard `start>=0 & length>=0 & start+length <=
    /// bytes-len(b)`, then `Some(bytes-slice(b, start, length))` else `None(unit)`. Args arrive i64 →
    /// wrapped to i32. Some=disc 0, None=disc 1.
    fn emit_bytes_slice(&mut self, seq: &Mir, start: &Mir, length: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        // `bytes-slice` CONSUMES its buffer operand — dup a shared `Local` seq so the other use sees it
        // unchanged (the runtime would otherwise reuse the rc==1 backing).
        self.emit_consuming_operand(seq, out)?;
        let h = self.alloc(ValType::I32);
        out.push(Lir::LocalSet(h));
        // start (i64) → keep as i64 for the non-negativity + sum checks, and an i32 copy for the slice.
        self.emit(start, out)?;
        let s = self.alloc(ValType::I64);
        out.push(Lir::LocalSet(s));
        self.emit(length, out)?;
        let n = self.alloc(ValType::I64);
        out.push(Lir::LocalSet(n));
        // in_bounds = (s >= 0) & (n >= 0) & (s + n <= len(h))   — all as signed i64 (len extended).
        out.push(Lir::LocalGet(s));
        out.push(Lir::ConstI64(0));
        out.push(Lir::I64GeS);
        out.push(Lir::LocalGet(n));
        out.push(Lir::ConstI64(0));
        out.push(Lir::I64GeS);
        out.push(Lir::I32And);
        out.push(Lir::LocalGet(s));
        out.push(Lir::LocalGet(n));
        out.push(Lir::I64Add);
        out.push(Lir::LocalGet(h));
        out.push(Lir::Call(himport::BYTES_LEN));
        out.push(Lir::I64ExtendI32U); // len i32 → i64
        out.push(Lir::I64LeS); // (s+n) <= len
        out.push(Lir::I32And);
        out.push(Lir::If(BlockType::Val(ValType::I32)));
        // then: Some(bytes-slice(h, wrap(s), wrap(n)))
        out.push(Lir::ConstI32(0));
        out.push(Lir::LocalGet(h));
        out.push(Lir::LocalGet(s));
        out.push(Lir::I32WrapI64);
        out.push(Lir::LocalGet(n));
        out.push(Lir::I32WrapI64);
        out.push(Lir::Call(himport::BYTES_SLICE));
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::Else);
        // else: None(unit)
        out.push(Lir::ConstI32(1));
        out.push(Lir::ConstI32(0));
        out.push(Lir::Call(himport::ARR_ALLOC));
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::End);
        Ok(())
    }

    /// Emit a fallible indexed read as an `Option` sum. `seq` is the Bytes/List handle, `idx` the i64
    /// index. `len_op`/`get_op` are the runtime length/get himports. `box_byte` = the got value is a raw
    /// i32 byte to `box-int`+extend into the payload (Bytes); false = the got value is already a boxed
    /// element handle (List). Builds: idx→i32; `if (idx>=0) & (idx < len(seq)) { sum-new(0, <payload>) }
    /// else { sum-new(1, arr-alloc(0)) }`. Some=disc 0, None=disc 1 (Option declaration order).
    fn emit_fallible_at(&mut self, seq: &Mir, idx: &Mir, len_op: u32, get_op: u32, box_byte: bool, out: &mut Vec<Lir>) -> Result<(), String> {
        // Stash the sequence handle and the i32 index (the index arrives as an i64; wrap it).
        self.emit(seq, out)?;
        let h = self.alloc(ValType::I32);
        out.push(Lir::LocalSet(h));
        self.emit(idx, out)?;
        out.push(Lir::I32WrapI64); // index → i32
        let i = self.alloc(ValType::I32);
        out.push(Lir::LocalSet(i));
        // in_bounds = (u32) i < len(h) — an UNSIGNED compare, so a negative index (a huge unsigned) is
        // out of range in one test (fallible-access: a negative index yields None, never wraps).
        out.push(Lir::LocalGet(i));
        out.push(Lir::LocalGet(h));
        out.push(Lir::Call(len_op));
        out.push(Lir::I32LtU);
        out.push(Lir::If(BlockType::Val(ValType::I32)));
        // then: Some(payload) — disc 0.
        out.push(Lir::ConstI32(0));
        out.push(Lir::LocalGet(h));
        out.push(Lir::LocalGet(i));
        out.push(Lir::Call(get_op));
        if box_byte {
            out.push(Lir::I64ExtendI32U); // byte i32 → i64
            out.push(Lir::Call(himport::BOX_INT));
        }
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::Else);
        // else: None(unit) — disc 1, payload = the empty array (unit).
        out.push(Lir::ConstI32(1));
        out.push(Lir::ConstI32(0));
        out.push(Lir::Call(himport::ARR_ALLOC));
        out.push(Lir::Call(himport::SUM_NEW));
        out.push(Lir::End);
        Ok(())
    }

    /// Emit a string literal as a Bytes-backed UTF-8 heap leaf: `buf = bytes-alloc(len)`, then per NFC
    /// byte `bytes-set(buf, i, byte)` (byte statically 0..=255 — no range guard), leaving `buf`. The
    /// value is identical to a `Bytes` build; only its static `Ty::String` distinguishes it (renders `"…"`).
    fn emit_str(&mut self, s: &str, out: &mut Vec<Lir>) -> Result<(), String> {
        let bytes = s.as_bytes();
        let buf = self.alloc(ValType::I32);
        out.push(Lir::ConstI32(bytes.len() as i32));
        out.push(Lir::Call(himport::BYTES_ALLOC));
        out.push(Lir::LocalSet(buf));
        for (i, b) in bytes.iter().enumerate() {
            out.push(Lir::LocalGet(buf));
            out.push(Lir::ConstI32(i as i32));
            out.push(Lir::ConstI32(*b as i32));
            out.push(Lir::Call(himport::BYTES_SET));
            out.push(Lir::Drop);
        }
        out.push(Lir::LocalGet(buf));
        Ok(())
    }

    /// Emit `Bytes.of` for a LITERAL list of byte values: `buf = bytes-alloc(len)`, then per element
    /// `bytes-set(buf, i, i32.wrap(range-check(v)))` (dropping the returned buffer copy), leaving `buf`.
    /// The range check traps if the byte value is < 0 or > 255 (`bytes value out of range`).
    fn emit_bytes_of(&mut self, elems: &[(Ty, Mir)], out: &mut Vec<Lir>) -> Result<(), String> {
        let buf = self.alloc(ValType::I32); // the byte buffer handle
        out.push(Lir::ConstI32(elems.len() as i32));
        out.push(Lir::Call(himport::BYTES_ALLOC));
        out.push(Lir::LocalSet(buf));
        for (i, (_ety, elem)) in elems.iter().enumerate() {
            // Stash the byte value (an i64) in a scratch local so the range guard reads it without
            // re-emitting the (possibly effectful) element.
            self.emit(elem, out)?;
            let val = self.alloc(ValType::I64);
            out.push(Lir::LocalSet(val));
            // Guard: (v < 0) → trap ; (v > 255) → trap.
            out.push(Lir::LocalGet(val));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64LtS);
            out.extend([Lir::If(BlockType::Empty), Lir::Unreachable, Lir::End]);
            out.push(Lir::LocalGet(val));
            out.push(Lir::ConstI64(255));
            out.push(Lir::I64GtS);
            out.extend([Lir::If(BlockType::Empty), Lir::Unreachable, Lir::End]);
            // bytes-set(buf, i, i32.wrap(v)) — returns the buffer; drop the returned copy, thread `buf`.
            out.push(Lir::LocalGet(buf));
            out.push(Lir::ConstI32(i as i32));
            out.push(Lir::LocalGet(val));
            out.push(Lir::I32WrapI64);
            out.push(Lir::Call(himport::BYTES_SET));
            out.push(Lir::Drop);
        }
        out.push(Lir::LocalGet(buf));
        Ok(())
    }

    /// Emit checked Int64 arithmetic — the IDEAL trapping sequences (numeric-model.md #Overflow Is
    /// Defined). Operand subtrees emit first (nested arithmetic gets its OWN scratch, no collision).
    fn emit_checked_arith(
        &mut self,
        op: ArithOp,
        a: &Mir,
        b: &Mir,
        out: &mut Vec<Lir>,
    ) -> Result<(), String> {
        self.emit(a, out)?;
        let la = self.alloc(ValType::I64); // i64
        out.push(Lir::LocalSet(la));
        self.emit(b, out)?;
        let lb = self.alloc(ValType::I64);
        out.push(Lir::LocalSet(lb));
        let lr = self.alloc(ValType::I64);

        match op {
            ArithOp::Add => {
                out.extend([Lir::LocalGet(la), Lir::LocalGet(lb), Lir::I64Add, Lir::LocalSet(lr)]);
                out.extend([Lir::LocalGet(la), Lir::LocalGet(lr), Lir::I64Xor]);
                out.extend([Lir::LocalGet(lb), Lir::LocalGet(lr), Lir::I64Xor]);
                out.push(Lir::I64And);
                out.extend([Lir::ConstI64(0), Lir::I64LtS]);
                out.extend([Lir::If(BlockType::Empty), Lir::Unreachable, Lir::End]);
                out.push(Lir::LocalGet(lr));
            }
            ArithOp::Sub => {
                out.extend([Lir::LocalGet(la), Lir::LocalGet(lb), Lir::I64Sub, Lir::LocalSet(lr)]);
                out.extend([Lir::LocalGet(la), Lir::LocalGet(lb), Lir::I64Xor]);
                out.extend([Lir::LocalGet(la), Lir::LocalGet(lr), Lir::I64Xor]);
                out.push(Lir::I64And);
                out.extend([Lir::ConstI64(0), Lir::I64LtS]);
                out.extend([Lir::If(BlockType::Empty), Lir::Unreachable, Lir::End]);
                out.push(Lir::LocalGet(lr));
            }
            ArithOp::Mul => {
                out.extend([Lir::LocalGet(la), Lir::LocalGet(lb), Lir::I64Mul, Lir::LocalSet(lr)]);
                out.push(Lir::LocalGet(la));
                out.push(Lir::I64Eqz);
                out.push(Lir::If(BlockType::Val(ValType::I64))); // i64-typed block
                out.push(Lir::LocalGet(lr)); // then: r (== 0)
                out.push(Lir::Else);
                out.extend([Lir::LocalGet(lr), Lir::LocalGet(la), Lir::I64DivS, Lir::LocalGet(lb), Lir::I64Ne]);
                out.extend([Lir::If(BlockType::Empty), Lir::Unreachable, Lir::End]);
                out.push(Lir::LocalGet(lr)); // else: r
                out.push(Lir::End);
            }
        }
        Ok(())
    }

    /// Emit a guarded shift `(<< a b)` / `(>> a b)` (numeric-model.md #Overflow Is Defined): the
    /// count is guarded — `(u64)count >= 64` (which also catches a negative count, a huge unsigned)
    /// traps — and a LEFT shift additionally traps on overflow (a bit shifted past the sign, detected
    /// by arithmetic-shifting the result back and comparing to `a`). A right shift is arithmetic
    /// (`i64.shr_s`). Matches the old compiler's `gen_shift` exactly, over fresh scratch locals.
    fn emit_shift(&mut self, op: ShiftOp, a: &Mir, b: &Mir, out: &mut Vec<Lir>) -> Result<(), String> {
        self.emit(a, out)?;
        let la = self.alloc(ValType::I64); // the value being shifted
        out.push(Lir::LocalSet(la));
        self.emit(b, out)?;
        let lb = self.alloc(ValType::I64); // the shift count
        out.push(Lir::LocalSet(lb));

        // Count guard: (u64)count >= 64 → trap.
        out.extend([Lir::LocalGet(lb), Lir::ConstI64(64), Lir::I64GeU]);
        out.extend([Lir::If(BlockType::Empty), Lir::Unreachable, Lir::End]);

        // Compute a <op> count.
        out.extend([Lir::LocalGet(la), Lir::LocalGet(lb)]);
        match op {
            ShiftOp::Right => {
                out.push(Lir::I64ShrS);
            }
            ShiftOp::Left => {
                out.push(Lir::I64Shl);
                let lr = self.alloc(ValType::I64);
                out.push(Lir::LocalSet(lr));
                // Overflow guard: (result >> count) != a → trap; else result.
                out.extend([Lir::LocalGet(lr), Lir::LocalGet(lb), Lir::I64ShrS, Lir::LocalGet(la), Lir::I64Ne]);
                out.extend([Lir::If(BlockType::Empty), Lir::Unreachable, Lir::End]);
                out.push(Lir::LocalGet(lr));
            }
        }
        Ok(())
    }
}
