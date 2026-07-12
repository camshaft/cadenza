//! `select` — instruction selection for the wasm backend: the core (A-normal, structured) form of a
//! definition body linearized into a flat `Vec<Lir>`.
//!
//! This is the wasm backend's linearization of the core (`backends-and-targets.md` §A Backend
//! Linearizes The Core Only If Its Target Is Linear). It reads a node's core form (via
//! [`crate::lower::core_of`]) and its solved type (via [`crate::infer::type_of`]) — the machine
//! representation is a READ-OFF of the solved type (`reference-compiler.md` §A Value's Machine
//! Representation Follows Its Solved Type At Selection), not a guess from the node's shape. It is
//! where a deferred integer width GROUNDS to its machine width, and where a literal that does not fit
//! its solved width DECLINES rather than emitting a truncated value.
//!
//! A construct the flat rung cannot express declines (`reference-compiler.md` §A Guarded Operation
//! Reserves Bounded Scratch Or Declines). What is selected: constant pushes, a structured
//! `if`/`else`/`end`, checked arithmetic and comparisons (guarded scratch locals), truncating
//! conversions, a `match` as a probe chain, a runtime `Core::Call`, and value-heap construction/
//! projection for tuples, records, and sums. A construct without a machine form here declines (e.g. a
//! runtime compound of a type that cannot yet cross the boundary).

use crate::ast::StructId;
use crate::backend::wasm::lir::{BlockType, Lir, ValType, valtype_of};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::{IntTy, Ty};
use std::collections::HashMap;
use tracing::trace;

// The value-heap runtime ops the tuple path emits, referenced by their WIT names (the same names the
// generated `runtime_abi` table + the import section resolve by). Named here so the emit reads clearly
// and `collect_used_ops` and `emit` agree on exactly one spelling per op.
const OP_ARR_ALLOC: &str = "arr-alloc";
const OP_ARR_SET: &str = "arr-set";
const OP_ARR_GET: &str = "arr-get";
const OP_BOX_INT: &str = "box-int";
const OP_GET_INT: &str = "get-int";
const OP_BOX_BOOL: &str = "box-bool";
const OP_GET_BOOL: &str = "get-bool";
/// `sum-new(disc, payload) -> handle` — build a sum value from its discriminant and a single payload
/// handle (`value-heap-runtime.md` §Sum). The payload is: an empty array for a nullary variant, the
/// boxed value for a one-payload variant, or a tuple handle for a multi-payload variant.
const OP_SUM_NEW: &str = "sum-new";
/// `sum-disc(handle) -> u32` — read a sum value's discriminant (which variant), driving a match's
/// dispatch. `sum-payload(handle) -> u32` — the sum's payload handle, unboxed to the bound value.
const OP_SUM_DISC: &str = "sum-disc";
const OP_SUM_PAYLOAD: &str = "sum-payload";
/// `drop` — release a reference to a heap handle (the Perceus calling convention). At refcount 0 the
/// runtime frees the node and recursively releases its children (the boxed elements), so a single
/// `drop` of a dead tuple reclaims the whole value.
const OP_DROP: &str = "drop";

/// Whether a solved type is a HEAP VALUE — one held as an owned runtime handle that the Perceus
/// contract reclaims (a tuple; later a record/sum/collection). A scalar (integer/bool/unit) owns no
/// heap cell, so it is never dup'd/drop'd. This is what decides which `let` bindings get a closing
/// `drop`.
fn is_heap_type(ty: &Ty) -> bool {
    matches!(ty, Ty::Tuple(_) | Ty::Record(_) | Ty::Sum { .. })
}

/// Whether the reference to the `let` binding `binder` ESCAPES the node at `id` — i.e. its reference
/// flows into a value that OUTLIVES the `let` (the returned result, an element of a constructed tuple,
/// or a call argument — all CONSUMING positions), as opposed to being used only to BORROW (a
/// `Core::Proj` operand: `arr-get` borrows and does not transfer ownership). An escaped binding must
/// NOT be dropped — its ownership transfers to the consumer (ownership-transfer-on-return). This is a
/// CONSERVATIVE analysis: any occurrence that is not provably a borrow is treated as an escape, so a
/// value is never wrongly reclaimed (a false "escapes" only leaks in a case we do not yet emit; a false
/// "does not escape" would be a use-after-free, which this avoids). `tail` marks whether `id` is in the
/// body's TAIL (result) position — a bare `LocalRef` in tail position is the return, an escape.
fn binding_escapes(db: &mut Db, id: StructId, binder: StructId, tail_borrowed: bool) -> bool {
    match core_of(db, id) {
        // A reference to the binding: it escapes UNLESS this occurrence is a borrow (the operand of a
        // `Proj`, which `arr-get`-borrows). `tail_borrowed` is set by the `Proj` arm below for its
        // operand; every other occurrence (the result, a tuple element, a call arg) is consuming.
        Core::LocalRef { binder: b } => b == binder && !tail_borrowed,
        // A projection BORROWS its operand — so a `LocalRef` directly under a `Proj` does not escape
        // through it. Recurse with the borrow flag set for the operand.
        Core::Proj { operand, .. } => binding_escapes(db, operand, binder, true),
        // A constructed tuple CONSUMES each element — a binding used as an element escapes into it.
        Core::Tuple { elems } => elems.iter().any(|&e| binding_escapes(db, e, binder, false)),
        // A call CONSUMES its arguments.
        Core::Call { args, .. } => args.iter().any(|&a| binding_escapes(db, a, binder, false)),
        // Control flow: the binding escapes if it escapes any reachable sub-position.
        Core::If { cond, then_, else_ } => {
            binding_escapes(db, cond, binder, false)
                || binding_escapes(db, then_, binder, false)
                || binding_escapes(db, else_, binder, false)
        }
        Core::Match { scrutinee, arms } => {
            binding_escapes(db, scrutinee, binder, false)
                || arms.iter().any(|a| {
                    a.guard
                        .is_some_and(|g| binding_escapes(db, g, binder, false))
                        || binding_escapes(db, a.body, binder, false)
                })
        }
        Core::Let { bindings, body } => {
            bindings
                .iter()
                .any(|(_, v)| binding_escapes(db, *v, binder, false))
                || binding_escapes(db, body, binder, false)
        }
        Core::Arith { lhs, rhs, .. } | Core::Compare { lhs, rhs, .. } => {
            binding_escapes(db, lhs, binder, false) || binding_escapes(db, rhs, binder, false)
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            binding_escapes(db, operand, binder, false)
        }
        Core::Record { fields } => fields
            .values()
            .any(|&v| binding_escapes(db, v, binder, false)),
        // A sum construction CONSUMES each payload (it becomes part of the heap sum value).
        Core::SumNew { payloads, .. } => payloads
            .iter()
            .any(|&p| binding_escapes(db, p, binder, false)),
        // A sum match: the binding escapes if it escapes the scrutinee or any arm's continuation (a leaf
        // body, or a nested switch's arms — recursed via `cont_binding_escapes`).
        Core::MatchSum {
            scrutinee, arms, ..
        } => {
            binding_escapes(db, scrutinee, binder, false)
                || arms
                    .iter()
                    .any(|a| cont_binding_escapes(db, &a.cont, binder))
        }
        // A sum-payload read BORROWS the scrutinee (`sum-payload` reads without consuming), like a
        // projection operand — so a `LocalRef` reached through it does not escape.
        Core::SumPayload { scrutinee, .. } => binding_escapes(db, scrutinee, binder, true),
        // Leaves reference no binding.
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::Unit
        | Core::Param { .. }
        | Core::Poison(_) => false,
    }
}

/// Whether `binder` escapes through a sum-match CONTINUATION — a leaf's body, or a nested switch's arms
/// (each recursed). The `Payload`/`Elem` path steps are heap reads that carry no binding, so only the arm
/// continuations matter (mirrors the `MatchSum` arm walk in `binding_escapes`).
fn cont_binding_escapes(db: &mut Db, cont: &crate::core::SumCont, binder: StructId) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => binding_escapes(db, *body, binder, false),
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| cont_binding_escapes(db, &a.cont, binder)),
    }
}

/// The runtime op that BOXES the node at `id` (a tuple/record element) into a u32 heap handle, by its
/// solved type: an integer → `box-int` (an i64 payload), a boolean → `box-bool`. A COMPOUND element (a
/// nested tuple/record) is ALREADY a u32 handle — it is `arr-set` into the parent array as-is, with no
/// box op — so this returns `Ok(None)` for a compound (the caller skips the box). A type with no heap
/// representation at all (a function/type-value) DECLINES. Reads the solved type.
fn box_op(db: &mut Db, id: StructId) -> Result<Option<&'static str>, Reject> {
    match type_of(db, id) {
        Ty::Int(_) => Ok(Some(OP_BOX_INT)),
        Ty::Bool => Ok(Some(OP_BOX_BOOL)),
        // A nested compound — a tuple/record, or a SUM (its `sum-new` handle) — is already a u32 handle,
        // so it is `arr-set` into the parent array as-is, no box op.
        Ty::Tuple(_) | Ty::Record(_) | Ty::Sum { .. } => Ok(None),
        other => Err(Reject::decline(format!(
            "a tuple element of type {} needs the value heap (not yet built)",
            other.render_name()
        ))),
    }
}

/// The runtime op that UNBOXES a u32 heap handle back to the value the node at `id` projects — the dual
/// of [`box_op`], keyed by this projection's solved type: an integer → `get-int`, a boolean →
/// `get-bool`. A COMPOUND projection (the element is itself a nested tuple/record) needs NO unbox — the
/// handle `arr-get` yields IS the nested compound — so this returns `Ok(None)` (the caller uses the
/// handle as-is). A projection of a type with no heap representation declines.
fn get_op(db: &mut Db, id: StructId) -> Result<Option<&'static str>, Reject> {
    match type_of(db, id) {
        Ty::Int(_) => Ok(Some(OP_GET_INT)),
        Ty::Bool => Ok(Some(OP_GET_BOOL)),
        // A nested compound / SUM handle `arr-get` yields is used as-is — no unbox.
        Ty::Tuple(_) | Ty::Record(_) | Ty::Sum { .. } => Ok(None),
        other => Err(Reject::decline(format!(
            "projecting a tuple element of type {} needs the value heap (not yet built)",
            other.render_name()
        ))),
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
fn is_narrow_int(db: &mut Db, id: StructId) -> Option<Machine> {
    match type_of(db, id) {
        Ty::Int(it) => {
            let m = Machine::of(it);
            m.slot32.then_some(m)
        }
        _ => None,
    }
}

/// A selected function body: its flat instruction sequence, the value types of its declared (non-
/// parameter) locals in slot order, its parameter value types, and its solved return type (for the
/// type section). A body may take parameters and declare locals (the scratch a guarded operation
/// reserves, and any persistent slot a kept `let` binding holds).
pub struct SelectedFunc {
    pub params: Vec<ValType>,
    pub ret: Ty,
    pub code: Vec<Lir>,
    pub declared: Vec<ValType>,
}

/// Select one NULLARY definition body (rooted at AST occurrence `body`) into its flat instruction
/// sequence. The return type is the body's solved type. Reads the core + type columns lazily.
pub fn select_body(db: &mut Db, body: StructId, layout: &Layout) -> Result<SelectedFunc, Reject> {
    select_function(db, body, &[], layout)
}

/// Collect the value-heap runtime OP NAMES the body (rooted at core node `id`) will emit, into `out`.
/// This mirrors `emit`'s op choices EXACTLY (the same `box_op`/`get_op` per element/projection type), so
/// the program's per-program import set is precisely the ops it calls — no more, no less. Run over every
/// reachable body BEFORE selection, so the used-set (hence `layout.import_base` and the import section)
/// is fixed before a `Lir::CallImport` is resolved to an index. Descends every sub-position (both `if`
/// branches, every arm body — an op used only under a branch is still imported, since the branch may
/// run). A box/get op that would decline (a non-scalar element) is simply not added here; the decline
/// surfaces at `emit`.
pub fn collect_used_ops(
    db: &mut Db,
    id: StructId,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    match core_of(db, id) {
        Core::Tuple { elems } => {
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            for elem in &elems {
                // A scalar element boxes (`box-int`/`box-bool`); a nested compound is already a handle
                // (`box_op` → `Ok(None)`), stored as-is. Recurse either way so a nested compound's own
                // construction ops (`arr-alloc`/`arr-set`/its element boxes) are collected.
                if let Ok(Some(op)) = box_op(db, *elem) {
                    out.insert(op);
                }
                collect_used_ops(db, *elem, out);
            }
        }
        Core::Proj { operand, .. } => {
            out.insert(OP_ARR_GET);
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
            collect_used_ops(db, operand, out);
        }
        Core::If { cond, then_, else_ } => {
            collect_used_ops(db, cond, out);
            collect_used_ops(db, then_, out);
            collect_used_ops(db, else_, out);
        }
        Core::Match { scrutinee, arms } => {
            collect_used_ops(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_used_ops(db, g, out);
                }
                collect_used_ops(db, arm.body, out);
            }
        }
        Core::Let { bindings, body } => {
            for (binder, value) in &bindings {
                // A HEAP-typed binding is `drop`'d after the body (Perceus) — so the program imports
                // `drop`. (A scalar binding owns no heap cell → no drop, matching `emit`.)
                if is_heap_type(&type_of(db, *binder)) {
                    out.insert(OP_DROP);
                }
                collect_used_ops(db, *value, out);
            }
            collect_used_ops(db, body, out);
        }
        Core::Arith { lhs, rhs, .. } | Core::Compare { lhs, rhs, .. } => {
            collect_used_ops(db, lhs, out);
            collect_used_ops(db, rhs, out);
        }
        Core::Convert { operand, .. } | Core::Not { operand } => collect_used_ops(db, operand, out),
        Core::Call { args, .. } => {
            for arg in args {
                collect_used_ops(db, arg, out);
            }
        }
        Core::Record { fields } => {
            // A runtime record builds on the heap exactly as a tuple — `arr-alloc` + per-field
            // `box-*`/`arr-set` (the same ops `emit`'s `Core::Record` arm lays down), so the used-set
            // must include them or the import section would omit an op the body calls.
            out.insert(OP_ARR_ALLOC);
            out.insert(OP_ARR_SET);
            for value in fields.values() {
                if let Ok(Some(op)) = box_op(db, *value) {
                    out.insert(op);
                }
                collect_used_ops(db, *value, out);
            }
        }
        // A sum construction always calls `sum-new`; the payload build mirrors `emit`'s `Core::SumNew`:
        //  - nullary → an empty array `arr-alloc(0)` (the unit payload);
        //  - single → `box-*` the one payload (a compound payload is already a handle, no box);
        //  - multi → a tuple handle (`arr-alloc` + per-payload `box-*`/`arr-set`).
        Core::SumNew { payloads, .. } => {
            out.insert(OP_SUM_NEW);
            match payloads.len() {
                0 => {
                    out.insert(OP_ARR_ALLOC);
                }
                1 => {
                    if let Ok(Some(op)) = box_op(db, payloads[0]) {
                        out.insert(op);
                    }
                    collect_used_ops(db, payloads[0], out);
                }
                _ => {
                    out.insert(OP_ARR_ALLOC);
                    out.insert(OP_ARR_SET);
                    for p in &payloads {
                        if let Ok(Some(op)) = box_op(db, *p) {
                            out.insert(op);
                        }
                        collect_used_ops(db, *p, out);
                    }
                }
            }
        }
        // A sum match calls `sum-disc` to dispatch at each switch; a switch on a deeper sub-value (a
        // non-empty `path`) first WALKS there (`sum-payload`/`arr-get` per step) before the disc. The
        // scrutinee + every arm's continuation are emitted (any op reachable in the tree must be
        // imported) — `collect_cont_ops` recurses nested switches, inserting their walk ops too.
        Core::MatchSum {
            scrutinee,
            path,
            arms,
        } => {
            out.insert(OP_SUM_DISC);
            for step in &path {
                match step {
                    crate::core::PathStep::Payload => out.insert(OP_SUM_PAYLOAD),
                    crate::core::PathStep::Elem(_) => out.insert(OP_ARR_GET),
                };
            }
            collect_used_ops(db, scrutinee, out);
            for arm in &arms {
                collect_cont_ops(db, &arm.cont, out);
            }
        }
        // A sum-payload read walks its `path` (`sum-payload`/`arr-get` per step) then unboxes the leaf
        // by THIS node's solved type (`get-*`).
        Core::SumPayload { scrutinee, path } => {
            for step in &path {
                match step {
                    crate::core::PathStep::Payload => out.insert(OP_SUM_PAYLOAD),
                    crate::core::PathStep::Elem(_) => out.insert(OP_ARR_GET),
                };
            }
            if let Ok(Some(op)) = get_op(db, id) {
                out.insert(op);
            }
            collect_used_ops(db, scrutinee, out);
        }
        // Leaves and references emit no runtime op.
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::Unit
        | Core::Param { .. }
        | Core::LocalRef { .. }
        | Core::Poison(_) => {}
    }
}

/// Collect the runtime ops a sum-match CONTINUATION uses — a leaf's body, or a nested switch (its own
/// `sum-disc` + path walk ops + its arms, recursed). Mirrors the `MatchSum` arm walk in `collect_used_ops`
/// so an op used only deep in the tree is still imported.
fn collect_cont_ops(
    db: &mut Db,
    cont: &crate::core::SumCont,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_used_ops(db, *body, out),
        crate::core::SumCont::Switch { path, arms } => {
            out.insert(OP_SUM_DISC);
            for step in path {
                match step {
                    crate::core::PathStep::Payload => out.insert(OP_SUM_PAYLOAD),
                    crate::core::PathStep::Elem(_) => out.insert(OP_ARR_GET),
                };
            }
            for arm in arms {
                collect_cont_ops(db, &arm.cont, out);
            }
        }
    }
}

/// Select a function body with `params` — each a `(name-occurrence, solved-type)`, in signature order.
/// The parameters occupy wasm local slots `0..n` in order; a `Core::Param` reference to a parameter
/// emits `local.get <slot>`. The return type is the body's solved type. A parameter whose type has no
/// machine representation (an unresolved/compound type) DECLINES here — an exported parameter needs a
/// definite scalar type (which an annotation supplies).
pub fn select_function(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    layout: &Layout,
) -> Result<SelectedFunc, Reject> {
    select_function_of(db, body, params, layout, None)
}

/// [`select_function`] plus the emitting function's OWN `db.defs` index (`self_def`) when known — used
/// to compile a SELF-tail-recursive function as a `loop` (its self-tail-calls iterate in place rather
/// than `return_call`). `None` (the `select_function` entry, and `select_body`) disables the loop
/// transform, so a self-call stays a `return_call`. A nullary or unknown-index function never loops.
pub fn select_function_of(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    layout: &Layout,
    self_def: Option<usize>,
) -> Result<SelectedFunc, Reject> {
    // Assign each parameter a local slot in order, and its wasm value type (its machine rep).
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_vts: Vec<ValType> = Vec::new();
    let mut param_slots: Vec<u32> = Vec::new();
    for (i, (binder, ty)) in params.iter().enumerate() {
        let vt = valtype_of(ty).ok_or_else(|| {
            Reject::decline("a function parameter's type has no machine representation")
        })?;
        slot_of.insert(*binder, i as u32);
        param_vts.push(vt);
        param_slots.push(i as u32);
    }
    let ret = type_of(db, body);
    let mut code = Vec::new();
    // Scratch locals start PAST the parameters (slots `0..n` are the params); a guarded op claims scratch
    // slots from `base` up. `high` tracks the highest scratch slot used, and `scratch_ty` records each
    // scratch slot's VALUE TYPE (i32 for a ≤32-bit op, i64 otherwise) — a slot must be DECLARED at the
    // type it is `local.set` with, or wasm rejects the module. (A given scratch slot is used at one
    // width within one op's guarded sequence: arithmetic preserves type and a width conversion `emit_wrap`
    // moves through the value stack rather than stashing across widths — so the map records the slot's
    // type rather than assuming i64.)
    let base = param_vts.len() as u32;
    let mut high = base;
    let mut scratch_ty: HashMap<u32, ValType> = HashMap::new();
    // If this function tail-calls itself (or a mutually-recursive PEER of the same signature) through
    // `if`/`let`/`match` result positions, and has parameters, compile it as a LOOP: a member tail-call
    // updates the parameter locals and `br`s to the loop top instead of a `return_call` — no wasm call
    // frame per iteration. `loop_members` is the tail-recursive group this function belongs to (just
    // `[self_def]` for plain self-recursion; `even`,`odd` for a mutual pair). Detection is conservative
    // — see `body_has_member_tail_call` (only the `if`/`let`/`match` tail positions the transform handles).
    let loop_members: Vec<usize> = match self_def {
        Some(d) if !param_slots.is_empty() => mutual_loop_group(db, d),
        _ => Vec::new(),
    };
    let loops = !loop_members.is_empty();
    // A MUTUAL group (more than one member) dispatches on a `which` state local: the first scratch slot
    // (i32, holding a member discriminant). A plain self-loop needs no dispatch (`which = None`). The
    // `which` slot is claimed above `base`, so scratch for the bodies starts one higher.
    let mutual = loop_members.len() > 1;
    let which_slot = base;
    let body_base = if mutual { base + 1 } else { base };
    if mutual {
        scratch_ty.insert(which_slot, ValType::I32);
        high = high.max(body_base);
    }
    // Every member's body references ITS OWN parameter occurrences; since the signatures are identical,
    // member `m`'s parameter at position `i` shares slot `i` with this function's. Map each member's
    // param binders onto the shared slots so `Core::Param` in a peer's body resolves (a peer body is
    // emitted inline under the dispatch below).
    let mut shared_slots = slot_of.clone();
    if mutual {
        for &m in &loop_members {
            for (i, p) in db.defs[m].params.clone().into_iter().enumerate() {
                let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                    Some(name_occ) => name_occ,
                    None => p,
                };
                shared_slots.insert(binder, i as u32);
            }
        }
    }
    let tl = loops.then(|| TailLoop {
        members: &loop_members,
        param_slots: &param_slots,
        which: mutual.then_some(which_slot),
        depth: 0,
    });
    // Initialize `which` to this function's OWN discriminant BEFORE the loop opens — it selects which
    // member body runs on the FIRST iteration (this function's own). A member cross-call updates `which`
    // for the next iteration; putting the init inside the loop would re-run it every iteration and
    // clobber that update (the entry would be re-selected forever — a correctness bug). So it is a
    // one-time setup outside the loop.
    if mutual {
        let self_which = loop_members
            .iter()
            .position(|&m| m == self_def.unwrap())
            .expect("self is a member of its own loop group") as i32;
        code.push(Lir::ConstI32(self_which));
        code.push(Lir::LocalSet(which_slot));
    }
    if loops {
        let block_ty = match &ret {
            Ty::Unit => BlockType::Empty,
            other => match valtype_of(other) {
                Some(vt) => BlockType::Val(vt),
                None => return Err(Reject::decline("looped function result has no machine rep")),
            },
        };
        code.push(Lir::Loop(block_ty));
    }
    // The body is emitted in TAIL position: a `Core::Call` in the body's result position becomes a
    // `return_call` (or, in a looped function, a member call becomes a loop iteration). `emit_tail`
    // propagates tail-ness through `if`/`match`/`let` result positions and delegates every non-tail
    // position to `emit`.
    if mutual {
        // Dispatch on `which`: an if-chain over the members runs the one whose discriminant is current.
        // Each member's body runs at `depth = dispatch-if-nesting + 1` (the extra +1 is the loop).
        emit_mutual_dispatch(
            db,
            &loop_members,
            which_slot,
            &shared_slots,
            body_base,
            &mut high,
            &mut scratch_ty,
            layout,
            &mut code,
            tl.unwrap(),
        )?;
    } else {
        emit_tail(
            db,
            body,
            &slot_of,
            body_base,
            &mut high,
            &mut scratch_ty,
            layout,
            &mut code,
            tl,
        )?;
    }
    if loops {
        // Close the loop block. Control reaches here only via a non-looping tail leaf, which left the
        // result value on the stack — that value is the loop's (and the function's) result.
        code.push(Lir::End);
    }
    // Declare scratch slots `base..high` in slot order, each at its recorded type (default i64 for a slot
    // that was counted in the high-water mark but never explicitly typed — a defensive fallback).
    let declared: Vec<ValType> = (base..high)
        .map(|s| scratch_ty.get(&s).copied().unwrap_or(ValType::I64))
        .collect();
    peephole(&mut code);
    Ok(SelectedFunc {
        params: param_vts,
        ret,
        code,
        declared,
    })
}

/// A local peephole pass over the linearized body: fold `local.set N ; local.get N` (store then
/// immediately re-read the SAME local) into a single `local.tee N` (store AND leave the value on the
/// stack, one opcode). This is ALWAYS valid — `local.tee` is defined as exactly that set-then-leave —
/// so no liveness analysis is needed; the two forms have identical stack and local effects. The pattern
/// is emitted wherever a value is stashed into a scratch slot and read back immediately: a nested
/// checked op's result flowing into the enclosing op's operand slot (`… local.set $r_inner ;
/// local.get $r_inner ; local.set $a`), and a runtime `let` value stored then used. Block markers
/// (`If`/`Else`/`End`) are their own `Lir` entries, so "adjacent in the vec" means adjacent WITHIN a
/// block — a `local.get` that opens a different block never fuses with a `local.set` closing another.
fn peephole(code: &mut Vec<Lir>) {
    let mut out: Vec<Lir> = Vec::with_capacity(code.len());
    let mut i = 0;
    while i < code.len() {
        if let Lir::LocalSet(n) = code[i]
            && let Some(Lir::LocalGet(m)) = code.get(i + 1)
            && n == *m
        {
            out.push(Lir::LocalTee(n));
            i += 2;
            continue;
        }
        out.push(code[i].clone());
        i += 1;
    }
    *code = out;
}

/// Whether the body at `id` makes a tail call to any def in `members` through the tail positions the
/// loop transform HANDLES — the body itself, an `if`'s two branches, a `let`'s body, or a `match`'s arm
/// bodies. NOT a non-tail position (an operand — that is a non-tail call). Mirrors `emit_tail`'s
/// propagation for exactly the `Call`/`If`/`Let`/`Match` cases so detection and emission agree. For a
/// plain self-loop `members = [self_def]`; for a mutual group it is every member (a tail call to any of
/// them iterates the shared loop).
fn body_has_member_tail_call(db: &mut Db, id: StructId, members: &[usize]) -> bool {
    match core_of(db, id) {
        Core::Call { callee, .. } => members.contains(&callee),
        Core::If { then_, else_, .. } => {
            body_has_member_tail_call(db, then_, members)
                || body_has_member_tail_call(db, else_, members)
        }
        Core::Let { bindings, body } => {
            // Match `emit_tail`: a `let` keeps its body's tail position only when no heap drop is pending
            // (a drop after the body would fall back to non-tail `emit`). A scalar-only `let` (the loop
            // shapes) has no drop, so this simply recurses the body.
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            !any_drop && body_has_member_tail_call(db, body, members)
        }
        // A `match`'s arm bodies are tail positions (the probe chain threads the loop context into each),
        // so a member tail-call in any arm makes the function loopable. (A guard is NOT a tail position —
        // it is a predicate evaluated before the body, so it is not considered here.)
        Core::Match { arms, .. } => arms
            .iter()
            .any(|a| body_has_member_tail_call(db, a.body, members)),
        _ => false,
    }
}

/// The def indices called in TAIL position from the body at `id` — the recursion edges the loop
/// transform can turn into a `br`. Descends exactly the tail positions `emit_tail` propagates through
/// (`if` branches, `let` body without a pending drop, `match` arms); a call in a NON-tail position (an
/// operand) is NOT a tail edge (it must stay a real call) and is skipped. This is the tail-call analogue
/// of `body_has_member_tail_call`, collecting the callees rather than testing one set.
fn tail_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    match core_of(db, id) {
        Core::Call { callee, .. } if !out.contains(&callee) => out.push(callee),
        Core::Call { .. } => {}
        Core::If { then_, else_, .. } => {
            tail_callees(db, then_, out);
            tail_callees(db, else_, out);
        }
        Core::Let { bindings, body } => {
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            if !any_drop {
                tail_callees(db, body, out);
            }
        }
        Core::Match { arms, .. } => {
            for arm in arms {
                tail_callees(db, arm.body, out);
            }
        }
        _ => {}
    }
}

/// The wasm value types of def `d`'s parameters, in order — its machine SIGNATURE. `None` if any
/// parameter type has no machine representation (that def can't be a loop member). Two defs share a
/// signature (the requirement for a shared mutual loop, which reuses one set of parameter slots) iff
/// their `sig_valtypes` are equal.
fn sig_valtypes(db: &mut Db, d: usize) -> Option<Vec<ValType>> {
    crate::layout::def_params(db, d)
        .iter()
        .map(|(_, ty)| valtype_of(ty))
        .collect()
}

/// The TAIL-RECURSIVE LOOP GROUP that def `self_def` belongs to — the set of defs compiled into ONE
/// shared `loop`. Returns `[self_def]` for plain self-recursion (a single-member loop, no dispatch), a
/// LARGER set for a mutually-tail-recursive group of SAME-SIGNATURE functions (`even`/`odd`), or empty
/// when `self_def` is not tail-recursive at all (so it stays ordinary `return_call`s).
///
/// The group is the strongly-connected component of `self_def` in the TAIL-call graph, restricted to
/// members that (a) share `self_def`'s machine signature — the shared loop reuses one set of parameter
/// slots, so members must agree on arity and per-slot type — and (b) are reachable in a tail cycle back
/// to `self_def`. A def whose signature differs, or that only calls `self_def` NON-tail, is excluded (a
/// non-tail call must stay a real call; a differing signature can't share the frame). Deterministic:
/// members are returned with `self_def` first, the rest in ascending def order, so the emitted `which`
/// discriminants are stable across runs.
///
/// MEMOIZED across a whole GROUP: `select_function_of` calls this for EVERY def, and the body is a
/// double BFS over the tail-call graph (forward reach + a reach-back-to-self per member), so a group of
/// N mutually tail-recursive same-signature defs cost O(N²) per def → O(N³) over the group (measured:
/// 200 mutual defs = 687ms before this). Every member of one SCC produces the SAME member SET (differing
/// only in the `self_def`-first ordering), so the expensive set is computed ONCE and cached by the
/// group's canonical representative (its minimum member index) — the N members of a group then share
/// that one computation, and each derives its self-first order cheaply. Keying by `self_def` directly
/// would MISS (each def is queried once), so the cache keys on the SORTED set's min element.
fn mutual_loop_group(db: &mut Db, self_def: usize) -> Vec<usize> {
    let sorted = mutual_loop_members_sorted(db, self_def);
    // Reorder to this member's view: `self_def` first (it enters the loop at its own discriminant), the
    // rest ascending. `sorted` is already ascending, so this is a cheap rotate of `self_def` to front.
    if sorted.len() <= 1 {
        return sorted; // a plain self-loop (or empty) needs no reorder
    }
    let mut members = Vec::with_capacity(sorted.len());
    members.push(self_def);
    members.extend(sorted.iter().copied().filter(|&d| d != self_def));
    members
}

/// The SORTED member set of `self_def`'s tail-recursive SCC (ascending; empty if not a loop). Cached
/// PER MEMBER: since every member of one group produces the SAME sorted set, the first member to be
/// queried computes it (the O(N²) BFS) and then caches it for EVERY member of the group at once — so
/// the other N-1 members hit the cache and never recompute. That collapses the group's total cost from
/// O(N³) to O(N²) (one compute) + O(N) lookups. A non-loop def caches its own empty set.
fn mutual_loop_members_sorted(db: &mut Db, self_def: usize) -> Vec<usize> {
    if let Some(cached) = db.mutual_loop_cache.get(&self_def) {
        return cached.clone();
    }
    let sorted = mutual_loop_group_uncached(db, self_def);
    // Cache for EVERY member of the discovered group (they all share this set) — so a co-member queried
    // later is an O(1) hit, not another O(N²) BFS. A non-loop def (empty set) caches just itself.
    if sorted.is_empty() {
        db.mutual_loop_cache.insert(self_def, Vec::new());
    } else {
        for &m in &sorted {
            db.mutual_loop_cache.insert(m, sorted.clone());
        }
    }
    sorted
}

/// The uncached core — computes the SORTED SCC member set (ascending), see [`mutual_loop_group`] docs.
fn mutual_loop_group_uncached(db: &mut Db, self_def: usize) -> Vec<usize> {
    let Some(self_sig) = sig_valtypes(db, self_def) else {
        return Vec::new();
    };
    // Forward tail-reachability from `self_def`, staying within same-signature defs. A def enters the
    // frontier only if it shares the signature (else the edge can't be a shared-loop iteration).
    let mut reach: Vec<usize> = vec![self_def];
    let mut i = 0;
    while i < reach.len() {
        let d = reach[i];
        i += 1;
        let Some(body) = db.defs[d].body else {
            continue;
        };
        let mut callees = Vec::new();
        tail_callees(db, body, &mut callees);
        for c in callees {
            if !reach.contains(&c) && sig_valtypes(db, c).as_ref() == Some(&self_sig) {
                reach.push(c);
            }
        }
    }
    // Keep only the members that tail-reach BACK to `self_def` (a genuine cycle) — the SCC. A def in
    // `reach` that never tail-calls back is a one-way tail callee (a helper `self_def` tail-calls but
    // which does not recurse into the group); it is not part of the loop and stays a `return_call`.
    // `self_def` is always in (it seeds the group; a lone `self_def` with a self-edge loops as before,
    // and even without one an empty group falls through to no-loop via the `loops` check upstream).
    let mut members: Vec<usize> = reach
        .iter()
        .copied()
        .filter(|&d| d == self_def || tail_reaches(db, d, self_def, &reach))
        .collect();
    // Deterministic order: `self_def` first (this function enters the loop at its own discriminant),
    // the rest ascending — so the emitted `which` discriminants are stable. (Discriminants are LOCAL to
    // each member function's own loop, so `self`-first differing per function is fine — control never
    // crosses between the two functions' loops.)
    members.sort_unstable();
    members.retain(|&d| d != self_def);
    members.insert(0, self_def);
    // A single member is a plain self-loop ONLY if it actually self-tail-calls; otherwise no loop.
    if members.len() == 1 {
        let body = match db.defs[self_def].body {
            Some(b) => b,
            None => return Vec::new(),
        };
        if body_has_member_tail_call(db, body, &members) {
            return members;
        }
        return Vec::new();
    }
    members
}

/// Whether def `from` tail-reaches `target` within the candidate set `within` (a path of tail calls,
/// each hop staying inside `within`). Used to keep only the SCC members in `mutual_loop_group`.
fn tail_reaches(db: &mut Db, from: usize, target: usize, within: &[usize]) -> bool {
    let mut seen: Vec<usize> = vec![from];
    let mut i = 0;
    while i < seen.len() {
        let d = seen[i];
        i += 1;
        let Some(body) = db.defs[d].body else {
            continue;
        };
        let mut callees = Vec::new();
        tail_callees(db, body, &mut callees);
        for c in callees {
            if c == target {
                return true;
            }
            if within.contains(&c) && !seen.contains(&c) {
                seen.push(c);
            }
        }
    }
    false
}

/// The context for a SELF-TAIL-RECURSIVE function being compiled as a `loop`: which def index a tail
/// call must recognize as a loop iteration (`members` — the def indices compiled into this shared
/// loop), the SHARED parameter slots a tail call updates in place, the `which` local's slot (the state
/// variable a mutual group dispatches on — `None` for a plain SELF-loop, which needs no dispatch), and
/// the current branch `depth` from the loop (how many `if`/loop blocks enclose this position — the `br`
/// target). Threaded through `emit_tail`; `None` when the function neither self- nor mutually-loops, so
/// a tail call stays a `return_call`.
///
/// A plain self-tail-recursive function is the degenerate case `members = [self_def]`, `which = None`.
/// A mutually-tail-recursive group of same-signature functions (`even`/`odd`) shares ONE loop: each
/// member's function runs the loop entered at its own discriminant, and a tail call to ANY member sets
/// the shared params, sets `which` to that member's discriminant (its index in `members`), and `br`s to
/// the loop top — a branch, not a wasm call. A tail call to a def OUTSIDE `members` stays `return_call`.
#[derive(Clone, Copy)]
struct TailLoop<'a> {
    members: &'a [usize],
    param_slots: &'a [u32],
    which: Option<u32>,
    depth: u32,
}

impl TailLoop<'_> {
    /// The discriminant (index in `members`) of a tail-call callee that is a loop member, or `None` if
    /// the callee is not in this loop's group (so the call stays a `return_call`).
    fn member_which(&self, callee: usize) -> Option<usize> {
        self.members.iter().position(|&m| m == callee)
    }
}

/// Whether a match's arm bodies are in TAIL position (and, if so, the enclosing self-loop context so a
/// self-tail-call in an arm iterates the loop rather than emitting `return_call`). `NonTail` = an
/// ordinary value match (arm bodies emit via `emit`); `Tail(tl)` = a match in tail position (arm bodies
/// via `emit_tail`, threading `tl` — `None` inside `tl` means tail-but-not-self-recursive).
#[derive(Clone, Copy)]
enum TailPos<'a> {
    NonTail,
    Tail(Option<TailLoop<'a>>),
}

/// Emit the node at `id` in TAIL position — the body's result, whose value becomes the function's
/// return. A `Core::Call` here is emitted as `return_call` (a TAIL call: it replaces the caller's frame
/// rather than pushing a new one), so a tail-recursive loop runs in O(1) stack instead of trapping by
/// stack exhaustion at ~35k frames. When `tl` marks this function self-recursive, a SELF tail call is
/// instead compiled as an in-place LOOP iteration (update the parameter locals, `br` to the loop top) —
/// no wasm call frame per step. Tail-ness PROPAGATES through the result-producing sub-positions: an
/// `if`'s two branches, a `let`'s body (only when no heap `drop` must run AFTER it — a drop is code that
/// executes on return, so the call can't be the last instruction), and a `match`'s arm bodies. Every
/// other node (an operand, an operation, a plain value) is not a tail call, so it delegates to `emit`.
/// This mirrors `emit`'s structure for exactly the propagating cases; everything else is one delegation.
#[allow(clippy::too_many_arguments)]
fn emit_tail(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
    tl: Option<TailLoop>,
) -> Result<(), Reject> {
    match core_of(db, id) {
        // A tail call. When it targets a MEMBER of the loop group being compiled, iterate in place:
        // evaluate the new argument values, move them into the parameter locals, set `which` to the
        // callee's discriminant (mutual group only), and `br` back to the loop top — no call frame.
        // Otherwise it is a `return_call` (a real tail call: recursion to a def outside this loop group,
        // or a function not compiled as a loop at all).
        Core::Call { callee, args } => {
            if let Some(tl) = tl
                && let Some(which) = tl.member_which(callee)
                && args.len() == tl.param_slots.len()
            {
                emit_loop_iteration(
                    db, which, &args, tl, slots, base, high, scratch_ty, layout, out,
                )?;
                return Ok(());
            }
            for &arg in &args {
                emit(db, arg, slots, base, high, scratch_ty, layout, out)?;
            }
            match layout.abs(callee) {
                Some(idx) => {
                    trace!(target: "rcdzc::select", callee, idx, args = args.len(), "emit TAIL call (return_call)");
                    out.push(Lir::ReturnCall(idx));
                    Ok(())
                }
                None => Err(Reject::decline(
                    "tail call to a definition with no emission index",
                )),
            }
        }
        // An `if` in tail position: its condition is not tail (a value the branch selects on), but BOTH
        // branches are — a tail call in either branch is the function's result.
        Core::If { cond, then_, else_ } => {
            let result = type_of(db, id);
            // BRANCHLESS SELECT (see the non-tail `emit` arm for the full rationale): when both branches
            // are cheap trap-free leaves and the result is a non-heap scalar, a `select` beats an `if`.
            // A leaf branch is never a tail call, so dropping the tail context here loses no `return_call`
            // /loop-`br` — the whole `if` becomes one value expression the caller consumes. (An exported
            // body emitted in tail position — `(def (f p a b) (if p a b))` — reaches HERE, not the
            // non-tail arm, so the select must be handled in both places.)
            if !matches!(result, Ty::Unit)
                && !is_heap_type(&result)
                && valtype_of(&result).is_some()
                && is_select_leaf(db, then_)
                && is_select_leaf(db, else_)
            {
                emit_branch(
                    db, then_, &result, slots, base, high, scratch_ty, layout, out,
                )?;
                emit_branch(
                    db, else_, &result, slots, base, high, scratch_ty, layout, out,
                )?;
                emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
                out.push(Lir::Select);
                return Ok(());
            }
            emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
            let block_ty = match &result {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "if result type has no machine representation",
                        ));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            // Inside the `if` block a self-loop `br` must jump one MORE level out to reach the loop top.
            let inner_tl = tl.map(|t| TailLoop {
                depth: t.depth + 1,
                ..t
            });
            // Each branch is TAIL (a tail call becomes `return_call`, a self-call a loop `br`), EXCEPT a
            // bare-literal branch, which must be GROUNDED to the `if`'s result width (a bare literal is
            // never a tail call, so grounding is safe): a default-Int64 literal opposite a narrow branch
            // would push a mismatched machine slot into the block. Ground via `emit_operand`, else emit
            // in tail pos.
            let emit_tail_branch = |db: &mut Db,
                                    b: StructId,
                                    high: &mut u32,
                                    st: &mut HashMap<u32, ValType>,
                                    out: &mut Vec<Lir>|
             -> Result<(), Reject> {
                if matches!(core_of(db, b), Core::ConstInt(_))
                    && let Ty::Int(rit) = &result
                {
                    emit_operand(db, b, *rit, slots, base, high, st, layout, out)
                } else {
                    emit_tail(db, b, slots, base, high, st, layout, out, inner_tl)
                }
            };
            emit_tail_branch(db, then_, high, scratch_ty, out)?;
            out.push(Lir::Else);
            emit_tail_branch(db, else_, high, scratch_ty, out)?;
            out.push(Lir::End);
            Ok(())
        }
        // A `let` in tail position: its body is tail — BUT only if no heap binding must be `drop`ped
        // AFTER the body. A drop is code that runs on the way out, so a `return_call` (which does not
        // return here) would skip it; when a drop is pending, fall back to the non-tail `emit` (the
        // body's call pushes an ordinary frame that returns, then the drops run). A body with no
        // pending drop (every heap binding escapes, or there are none) keeps the tail position.
        Core::Let { bindings, body } => {
            let any_drop = bindings.iter().any(|(binder, _)| {
                is_heap_type(&type_of(db, *binder)) && !binding_escapes(db, body, *binder, false)
            });
            if any_drop {
                return emit(db, id, slots, base, high, scratch_ty, layout, out);
            }
            // Re-emit the bindings exactly as `emit` does, then the body in TAIL position. (No drop
            // epilogue is needed — the `any_drop` check above guaranteed none.)
            let mut extended = slots.clone();
            let mut floor = base;
            for (binder, value) in &bindings {
                let slot = floor;
                let ty = type_of(db, *binder);
                let vt = valtype_of(&ty).ok_or_else(|| {
                    Reject::decline("a let binding's type has no machine representation")
                })?;
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
                scratch_ty.insert(slot, vt);
                if slot + 1 > *high {
                    *high = slot + 1;
                }
                extended.insert(*binder, slot);
                floor = slot + 1;
            }
            // A `let` adds no wasm block (its bindings are plain `local.set`s), so the loop-branch depth
            // is unchanged — the body's tail position is at the same nesting as the `let`.
            emit_tail(
                db, body, &extended, floor, high, scratch_ty, layout, out, tl,
            )
        }
        // A `match` in tail position: each arm body is tail. Delegated with a tail-aware arm emitter.
        Core::Match { scrutinee, arms } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "match result type has no machine representation",
                        ));
                    }
                },
            };
            let it = int_ty_of(db, scrutinee);
            // The match's RESULT integer type (its arms' joined width), so a bare-literal arm body is
            // grounded to it (like an operand of a binary op) — otherwise an arm that is a default-Int64
            // literal beside an arm at a NARROW width would push a mismatched machine slot and wasm
            // rejects the block. `None` for a non-integer result (e.g. Bool arms — a ConstBool is always
            // i32, no width to reconcile).
            let result_it = match type_of(db, id) {
                Ty::Int(rit) => Some(rit),
                _ => None,
            };
            emit_match_arms_tailable(
                db,
                scrutinee,
                &arms,
                it,
                result_it,
                block_ty,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
                TailPos::Tail(tl),
            )
        }
        // Everything else in tail position is an ordinary value (no tail call inside it) — emit normally.
        _ => emit(db, id, slots, base, high, scratch_ty, layout, out),
    }
}

/// Emit a member tail-call as a LOOP iteration: update the parameter locals with the new argument
/// values, set the `which` state local (for a mutual group) to the callee's discriminant, and `br` back
/// to the loop top — no wasm call frame. The new args are ALL evaluated onto the stack FIRST (each
/// reading the OLD parameter values), then popped into the param slots in REVERSE order (the stack is
/// LIFO, so the last-pushed arg is on top and stores into the last param). This is the standard parallel
/// move: it avoids the clobber where storing arg 0 into `$0` would corrupt a later arg that reads `$0`
/// (`sum(n-1, acc+n)` — arg 1 `acc+n` reads the OLD `n`, evaluated before `$0` is written). `which` is
/// set AFTER the params (its slot is above the params, never an arg source, so order is free). `tl.depth`
/// is the number of enclosing `if`/loop blocks, so `br depth` targets the loop top.
#[allow(clippy::too_many_arguments)]
fn emit_loop_iteration(
    db: &mut Db,
    which: usize,
    args: &[StructId],
    tl: TailLoop,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    trace!(target: "rcdzc::select", which, depth = tl.depth, args = args.len(), "emit member tail-call as loop iteration");
    // Evaluate each new argument value onto the stack, grounding a bare-literal arg to its OWN solved
    // width (unification already set it to the parameter's type at the call site — the same
    // reconciliation an operand/branch literal gets, so a default-Int64 literal into a narrow param slot
    // does not mismatch). All args are evaluated BEFORE any store, so each reads the OLD param values.
    for &arg in args {
        if let Core::ConstInt(_) = core_of(db, arg)
            && let Ty::Int(ait) = type_of(db, arg)
        {
            emit_operand(db, arg, ait, slots, base, high, scratch_ty, layout, out)?;
        } else {
            emit(db, arg, slots, base, high, scratch_ty, layout, out)?;
        }
    }
    // Pop the values into the parameter slots, last-arg-first (stack is LIFO).
    for &slot in tl.param_slots.iter().rev() {
        out.push(Lir::LocalSet(slot));
    }
    // For a mutual group, set the `which` state so the next iteration dispatches into the callee's body.
    // (A plain self-loop has one member, `which = None`, and skips this.)
    if let Some(w) = tl.which {
        out.push(Lir::ConstI32(which as i32));
        out.push(Lir::LocalSet(w));
    }
    // Jump to the loop top to iterate.
    out.push(Lir::Br(tl.depth));
    Ok(())
}

/// Emit the mutual-recursion DISPATCH inside the shared loop: an if-chain on the `which` state local
/// that runs the matching member's body in tail position. For k members, `k-1` `if`s test
/// `which == 0, 1, …` and the final `else` is the last member (its discriminant by elimination). Each
/// member body is emitted in TAIL position so a member tail-call inside it iterates the loop; the body
/// sits one `if` deeper than the position handed in, so the threaded `TailLoop.depth` bumps +1 per
/// enclosing dispatch `if` (mirroring how `emit_tail`'s `if` arm bumps depth). `tl.depth` on entry is
/// the loop-relative depth of the dispatch (0 — the loop is the immediately enclosing block).
#[allow(clippy::too_many_arguments)]
fn emit_mutual_dispatch(
    db: &mut Db,
    members: &[usize],
    which_slot: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
    tl: TailLoop,
) -> Result<(), Reject> {
    // Emit member `idx`'s body at branch-depth `depth` (loop-relative), then the rest as the `else` tail.
    fn emit_from(
        db: &mut Db,
        members: &[usize],
        idx: usize,
        which_slot: u32,
        slots: &HashMap<StructId, u32>,
        base: u32,
        high: &mut u32,
        scratch_ty: &mut HashMap<u32, ValType>,
        layout: &Layout,
        out: &mut Vec<Lir>,
        tl: TailLoop,
        block_ty: BlockType,
    ) -> Result<(), Reject> {
        let member = members[idx];
        let body = db.defs[member]
            .body
            .ok_or_else(|| Reject::decline("a loop member has no body"))?;
        if idx + 1 == members.len() {
            // Last member — the unconditional tail (no probe; reached by elimination).
            return emit_tail(
                db,
                body,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
                Some(tl),
            );
        }
        // `which == idx` ? run this member's body : fall through to the next. The body/else sit one `if`
        // deeper, so the loop `br` target grows by one.
        out.push(Lir::LocalGet(which_slot));
        if idx > 0 {
            out.push(Lir::ConstI32(idx as i32));
            out.push(Lir::I32Eq);
        } else {
            // `which == 0` is `i32.eqz` (one instruction; the discriminant 0 is the common entry).
            out.push(Lir::I32Eqz);
        }
        out.push(Lir::If(block_ty));
        let deeper = TailLoop {
            depth: tl.depth + 1,
            ..tl
        };
        emit_tail(
            db,
            body,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            Some(deeper),
        )?;
        out.push(Lir::Else);
        emit_from(
            db,
            members,
            idx + 1,
            which_slot,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            deeper,
            block_ty,
        )?;
        out.push(Lir::End);
        Ok(())
    }
    let ret = type_of(db, tl_body_of(db, members[0])?);
    let block_ty = match &ret {
        Ty::Unit => BlockType::Empty,
        other => match valtype_of(other) {
            Some(vt) => BlockType::Val(vt),
            None => return Err(Reject::decline("looped member result has no machine rep")),
        },
    };
    emit_from(
        db, members, 0, which_slot, slots, base, high, scratch_ty, layout, out, tl, block_ty,
    )
}

/// A loop member's body occurrence (helper for `emit_mutual_dispatch`'s block-type read).
fn tl_body_of(db: &Db, member: usize) -> Result<StructId, Reject> {
    db.defs[member]
        .body
        .ok_or_else(|| Reject::decline("a loop member has no body"))
}

/// Emit the flat instructions for the node at `id`, appending to `out`. `slots` maps a parameter's
/// name occurrence to its wasm local slot; `base` is the next free SCRATCH slot (a guarded op claims
/// `[base, base+1, base+2]` and recurses operands at `base+3`); `high` is the running high-water mark of
/// scratch slots used (so `select_function` declares exactly that many); `scratch_ty` records each
/// scratch slot's value type (so it is declared at the type it is set with). Exhaustive over `Core`.
#[allow(clippy::too_many_arguments)]
fn emit(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    match core_of(db, id) {
        Core::ConstInt(v) => {
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
            out.push(Lir::ConstI32(fields.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            for (i, (_, &value)) in fields.iter().enumerate() {
                // [arr] ; push index ; push (box, if scalar) the field value ; arr-set → [arr]
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit(db, value, slots, base, high, scratch_ty, layout, out)?; // [arr, i, value]
                // A scalar element boxes to a handle (a NARROW int first extends i32→i64, as box-int
                // takes an i64 cell); a nested compound is ALREADY a u32 handle → `arr-set` it directly.
                if let Some(op) = box_op(db, value)? {
                    if let Some(m) = is_narrow_int(db, value) {
                        out.push(if m.signed {
                            Lir::I64ExtendI32S
                        } else {
                            Lir::I64ExtendI32U
                        });
                    }
                    out.push(Lir::CallImport(op)); // [arr, i, handle]
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
            out.push(Lir::ConstI32(elems.len() as i32));
            out.push(Lir::CallImport(OP_ARR_ALLOC)); // → [arr]
            for (i, &elem) in elems.iter().enumerate() {
                // [arr] ; push index ; push (box, if scalar) the element ; arr-set → [arr]
                out.push(Lir::ConstI32(i as i32)); // [arr, i]
                emit(db, elem, slots, base, high, scratch_ty, layout, out)?; // [arr, i, elem]
                // A scalar element boxes (a NARROW int extends i32→i64 first, box-int takes i64); a
                // nested compound is ALREADY a u32 handle → `arr-set` it directly, no box.
                if let Some(op) = box_op(db, elem)? {
                    if let Some(m) = is_narrow_int(db, elem) {
                        out.push(if m.signed {
                            Lir::I64ExtendI32S
                        } else {
                            Lir::I64ExtendI32U
                        });
                    }
                    out.push(Lir::CallImport(op)); // [arr, i, handle]
                }
                out.push(Lir::CallImport(OP_ARR_SET)); // → [arr]
            }
            Ok(()) // leaves [arr] — the tuple handle
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
            out.push(Lir::ConstI32(disc as i32)); // [disc]
            match payloads.len() {
                0 => {
                    // Unit payload: an empty array.
                    out.push(Lir::ConstI32(0)); // [disc, 0]
                    out.push(Lir::CallImport(OP_ARR_ALLOC)); // [disc, payload]
                }
                1 => {
                    let p = payloads[0];
                    emit(db, p, slots, base, high, scratch_ty, layout, out)?; // [disc, value]
                    if let Some(op) = box_op(db, p)? {
                        if let Some(m) = is_narrow_int(db, p) {
                            out.push(if m.signed {
                                Lir::I64ExtendI32S
                            } else {
                                Lir::I64ExtendI32U
                            });
                        }
                        out.push(Lir::CallImport(op)); // [disc, payload-handle]
                    }
                }
                n => {
                    // Multiple payloads: build a tuple `arr` and box each into its position.
                    out.push(Lir::ConstI32(n as i32)); // [disc, n]
                    out.push(Lir::CallImport(OP_ARR_ALLOC)); // [disc, arr]
                    for (i, &p) in payloads.iter().enumerate() {
                        out.push(Lir::ConstI32(i as i32)); // [disc, arr, i]
                        emit(db, p, slots, base, high, scratch_ty, layout, out)?; // [disc, arr, i, value]
                        if let Some(op) = box_op(db, p)? {
                            if let Some(m) = is_narrow_int(db, p) {
                                out.push(if m.signed {
                                    Lir::I64ExtendI32S
                                } else {
                                    Lir::I64ExtendI32U
                                });
                            }
                            out.push(Lir::CallImport(op)); // [disc, arr, i, handle]
                        }
                        out.push(Lir::CallImport(OP_ARR_SET)); // [disc, arr]
                    }
                }
            }
            out.push(Lir::CallImport(OP_SUM_NEW)); // → [sum-handle]
            Ok(())
        }
        // A runtime PROJECTION `(. t i)` — read element `i` off the operand's array handle and UNBOX it
        // to its scalar: `<operand handle> ; i32.const i ; arr-get ; get-<T>`. The result type (this
        // node's solved type) chooses the unbox op.
        Core::Proj { operand, index } => {
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?; // [handle]
            out.push(Lir::ConstI32(index as i32)); // [handle, i]
            out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle]
            // A scalar element unboxes (`get-int`/`get-bool`, then a NARROW int narrows i64→i32); a
            // nested compound: the handle `arr-get` yields IS the nested compound — use it as-is.
            if let Some(op) = get_op(db, id)? {
                out.push(Lir::CallImport(op)); // → [scalar (i64 for an int, i32 for a bool)]
                if is_narrow_int(db, id).is_some() {
                    out.push(Lir::I32WrapI64);
                }
            }
            Ok(())
        }
        // A sum-variant pattern's payload binder — WALK the access `path` from the scrutinee handle
        // (`sum-payload` per `Payload` step, `arr-get i` per `Elem` step), then unbox the leaf by THIS
        // node's solved type. A single `[Payload]` path is the flat `(Some x)` case; `[Payload, Payload]`
        // is the nested `(Some (Some y))` binder.
        Core::SumPayload { scrutinee, path } => {
            emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle]
            for step in &path {
                match step {
                    crate::core::PathStep::Payload => {
                        out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // → [payload-handle]
                    }
                    crate::core::PathStep::Elem(i) => {
                        out.push(Lir::ConstI32(*i as i32));
                        out.push(Lir::CallImport(OP_ARR_GET)); // → [elem-handle]
                    }
                }
            }
            if let Some(op) = get_op(db, id)? {
                out.push(Lir::CallImport(op)); // → [scalar]
                if is_narrow_int(db, id).is_some() {
                    out.push(Lir::I32WrapI64);
                }
            }
            Ok(())
        }
        Core::If { cond, then_, else_ } => {
            let result = type_of(db, id);
            // BRANCHLESS SELECT: when both branches are cheap trap-free leaves (a param/local/constant)
            // and the result is a SCALAR (not unit, not a heap handle), emit wasm's `select` instead of
            // an `if`/`else`/`end` block — one instruction, no branch. `select` pops `[a, b, cond]` and
            // pushes `a` if `cond` is nonzero else `b`, evaluating BOTH unconditionally; that is sound
            // here precisely because each leaf is trap-free, allocation-free, and cheap (so nothing is
            // wasted vs the branch it replaces). A HEAP result is excluded: `select` would evaluate both
            // handles and discard one WITHOUT the Perceus `drop` that owning branch would run, leaking
            // its cell — the `if` (which evaluates only the taken branch) stays for those. This is the
            // classic `min`/`max`/conditional-value idiom `(if (< a b) a b)`.
            if !matches!(result, Ty::Unit)
                && !is_heap_type(&result)
                && valtype_of(&result).is_some()
                && is_select_leaf(db, then_)
                && is_select_leaf(db, else_)
            {
                emit_branch(
                    db, then_, &result, slots, base, high, scratch_ty, layout, out,
                )?;
                emit_branch(
                    db, else_, &result, slots, base, high, scratch_ty, layout, out,
                )?;
                emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
                out.push(Lir::Select);
                return Ok(());
            }
            // Selection order matches wasm's structured `if`: push the condition, open the block with
            // the RESULT type (read off the node's solved type), then the two arms.
            emit(db, cond, slots, base, high, scratch_ty, layout, out)?;
            let block_ty = match &result {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "if result type has no machine representation",
                        ));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            // Both branches must produce the `if`'s RESULT machine slot; a bare-literal branch (default
            // Int64) opposite a NARROW branch would otherwise push a mismatched i64 into a narrow-i32
            // block. Ground a bare-`ConstInt` branch to the result's integer width via `emit_operand`,
            // exactly as an operator operand (`@1a4528f`) and a match arm (`@10f7bdb`) are grounded.
            emit_branch(
                db, then_, &result, slots, base, high, scratch_ty, layout, out,
            )?;
            out.push(Lir::Else);
            emit_branch(
                db, else_, &result, slots, base, high, scratch_ty, layout, out,
            )?;
            out.push(Lir::End);
            Ok(())
        }
        // A scalar MATCH → a chain of `if`s. The match's solved type is each arm's block-result type.
        // Each non-wildcard arm probes `scrutinee == literal` (push scrutinee, push the literal, compare)
        // and takes its body on a match, else falls through to the next arm; the wildcard arm is the
        // unconditional tail (`else`). The scrutinee is a scalar, so re-pushing it per probe is a cheap
        // local reload — no naming needed.
        Core::Match { scrutinee, arms } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
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
            )
        }
        // A sum MATCH → a chain of `if`s over `sum-disc(scrutinee)`. Each variant arm probes
        // `sum-disc(scrutinee) == disc` and takes its body on a match; a wildcard/binder arm (`disc:
        // None`) is the unconditional `else` tail. The scrutinee is a heap handle (an i32 local reload
        // per probe, cheap). A payload binder in a body reads `sum-payload(scrutinee)` on its own
        // (`Core::SumPayload`), so the arm dispatch needs only the disc.
        Core::MatchSum {
            scrutinee,
            path,
            arms,
        } => {
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
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
            emit_sum_match_arms(
                db, scrutinee, &path, &arms, result_it, block_ty, slots, base, high, scratch_ty,
                layout, out,
            )
        }
        // A parameter reference — read its local slot. The slot was assigned in `select_function`; a
        // reference to a binder with no slot is a compiler bug (a param not in the signature), so
        // decline rather than emit a wrong `local.get`.
        Core::Param { binder } => match slots.get(&binder) {
            Some(&slot) => {
                out.push(Lir::LocalGet(slot));
                Ok(())
            }
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
            // Track each HEAP-typed binding as `(binder, slot)`, to `drop` after the body UNLESS it
            // escapes (Perceus). A kept binding is always a genuine runtime value — a constant tuple
            // folds and is never kept (H2c) — so every heap binding here is an owned allocation whose
            // reference is released once its scope ends, or transferred out if it escapes. (A scalar
            // binding owns no heap cell → no drop.)
            let mut heap_bindings: Vec<(StructId, u32)> = Vec::new();
            for (binder, value) in &bindings {
                let slot = floor;
                let ty = type_of(db, *binder);
                // The binding's machine value type — read off its solved type (the value's type). A
                // binding whose type has no machine rep (a compound/unresolved value) declines.
                let vt = valtype_of(&ty).ok_or_else(|| {
                    Reject::decline("a let binding's type has no machine representation")
                })?;
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
                scratch_ty.insert(slot, vt);
                if slot + 1 > *high {
                    *high = slot + 1;
                }
                if is_heap_type(&ty) {
                    heap_bindings.push((*binder, slot));
                }
                extended.insert(*binder, slot);
                floor = slot + 1;
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
            emit(db, body, &extended, floor, high, scratch_ty, layout, out)?;
            for &(binder, slot) in &heap_bindings {
                if !binding_escapes(db, body, binder, false) {
                    out.push(Lir::LocalGet(slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            Ok(())
        }
        // A reference to a kept `let` binding — read its persistent slot, exactly like a parameter. The
        // slot was assigned when the enclosing `Core::Let` was emitted; a `LocalRef` with no slot is a
        // compiler bug (a ref lowered as `LocalRef` without its binding kept), so decline.
        Core::LocalRef { binder } => match slots.get(&binder) {
            Some(&slot) => {
                out.push(Lir::LocalGet(slot));
                Ok(())
            }
            None => Err(Reject::decline("let-binding reference has no local slot")),
        },
        // A runtime CALL — push each argument (in the CALLER's frame, so an arg's own scratch floats
        // above `base`), then `call` the callee's absolute wasm-function index (its position in the
        // layout's emission order). The callee is reachable (`layout` added it), so its index exists; a
        // callee not in the emission order is a compiler bug (reachability missed it) → decline.
        Core::Call { callee, args } => {
            for &arg in &args {
                emit(db, arg, slots, base, high, scratch_ty, layout, out)?;
            }
            match layout.abs(callee) {
                Some(idx) => {
                    trace!(target: "rcdzc::select", callee, idx, args = args.len(), "emit runtime call");
                    out.push(Lir::Call(idx));
                    Ok(())
                }
                None => Err(Reject::decline(
                    "a called function is not in the emission order (reachability gap)",
                )),
            }
        }
        // A runtime comparison: emit both operands, then the machine comparison selected from the
        // operands' width AND signedness (`_s` for a signed type, `_u` for an unsigned one; `eq` is
        // sign-agnostic). The result is an i32 boolean. A comparison never overflows, so both operands
        // share the same scratch base (each is dead once pushed).
        Core::Compare { op, lhs, rhs } => {
            // Both operands must share the comparison's machine slot; ground a bare-literal operand to
            // the shared width so `(> x 50)` over a narrow `x` does not push an i64 beside `x`'s i32.
            let it = operand_int_ty(db, lhs, rhs);
            emit_operand(db, lhs, it, slots, base, high, scratch_ty, layout, out)?;
            emit_operand(db, rhs, it, slots, base, high, scratch_ty, layout, out)?;
            out.push(compare_op(op, it));
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
        Core::Arith { op, lhs, rhs } => {
            let m = Machine::of(int_ty_of(db, id));
            trace!(target: "rcdzc::select", node = id.0, ?op, width = m.width, signed = m.signed, "emit runtime integer op");
            match op {
                Prim::Add | Prim::Sub | Prim::Mul => emit_checked_arith(
                    db, op, m, lhs, rhs, slots, base, high, scratch_ty, layout, out,
                ),
                Prim::BitAnd | Prim::BitOr | Prim::BitXor => {
                    let ot = IntTy::fixed(m.signed, m.width);
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
        // A runtime boolean NEGATION `!operand` — emit the operand (a Bool i32), then `i32.eqz` (1 if 0,
        // else 0 = logical NOT). From the `(if c false true)` fold.
        Core::Not { operand } => {
            emit(db, operand, slots, base, high, scratch_ty, layout, out)?;
            out.push(Lir::I32Eqz);
            Ok(())
        }
        // A poison that reached selection is an unconditionally-reached fault; the poison collector
        // surfaces it before emission, so reaching here is a decline rather than emitted code.
        Core::Poison(reject) => Err(reject),
    }
}

/// Emit a scalar match as a chain of `if`s. `arms` is `[(probe, body)…]` in order; `it` is the
/// scrutinee's integer type (for the comparison op — a boolean scrutinee is compared as an i32). Each
/// LITERAL arm probes `scrutinee == literal` and takes its body on a match, else recurses on the
/// remaining arms in the `else`; a WILDCARD arm is the unconditional tail (emit its body, stop). The
/// scrutinee is re-emitted per probe (a scalar local reload — cheap and correct). `lower` guaranteed a
/// wildcard tail for a runtime match (exhaustiveness), so the chain always terminates in a body.
#[allow(clippy::too_many_arguments)]
fn emit_match_arms(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    emit_match_arms_tailable(
        db,
        scrutinee,
        arms,
        it,
        result_it,
        block_ty,
        slots,
        base,
        high,
        scratch_ty,
        layout,
        out,
        TailPos::NonTail,
    )
}

/// `emit_match_arms`, but with a [`TailPos`]: when the match is in TAIL position, each ARM BODY is a
/// tail position too — a tail call in an arm becomes `return_call`, or, when the enclosing function is
/// self-recursive (`TailPos::Tail(Some(tl))`), a SELF tail-call in an arm iterates the loop. The
/// scrutinee and the probe comparisons are never tail (they are values the dispatch reads).
#[allow(clippy::too_many_arguments)]
fn emit_match_arms_tailable(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
    tail: TailPos,
) -> Result<(), Reject> {
    // Resolve the scrutinee to a SOURCE pushed once per probe. A match dispatches by testing the
    // scrutinee against each arm's literal in turn — so the scrutinee is read once PER PROBE. If it is a
    // reusable value (a parameter/local, or a constant), re-pushing it each time is free. But a COMPUTED
    // scrutinee (`(match (+ a b) …)`) would be fully RE-EVALUATED per probe — recomputing the add AND
    // its overflow guard N times. So a non-reusable scrutinee is evaluated ONCE into a scratch slot here,
    // and every probe reads that slot. A scalar match's scrutinee is Int or Bool (an i32/i64 slot).
    let scrut_vt = match block_scalar_slot(db, scrutinee) {
        Some(vt) => vt,
        None => {
            return Err(Reject::decline(
                "match scrutinee has no machine representation",
            ));
        }
    };
    let (src, chain_base) = match reusable_scalar_src(db, scrutinee, slots) {
        // A reusable scrutinee is pushed in place at each probe — no scratch, the probe chain keeps the
        // full scratch region from `base`.
        Some(src) => (src, base),
        None => {
            // Evaluate the scrutinee ONCE into scratch slot `base`, and run the probe chain from `base+1`
            // so the arm bodies and later probes never clobber that live slot (it must survive every
            // probe). The scrutinee's own emit uses `base+1` too (it is fully consumed into `base` before
            // any probe runs). `slot` is at least `base`, so the high-water covers it.
            let slot = base;
            if slot + 1 > *high {
                *high = slot + 1;
            }
            scratch_ty.insert(slot, scrut_vt);
            emit(
                db,
                scrutinee,
                slots,
                base + 1,
                high,
                scratch_ty,
                layout,
                out,
            )?;
            out.push(Lir::LocalSet(slot));
            (OperandSrc::Slot(slot), base + 1)
        }
    };
    emit_probe_chain(
        db, src, arms, it, result_it, block_ty, slots, chain_base, high, scratch_ty, layout, out,
        tail,
    )
}

/// The wasm slot type of a scalar match scrutinee (Int → its width's slot, Bool → i32), or `None` if
/// it has no machine representation.
fn block_scalar_slot(db: &mut Db, scrutinee: StructId) -> Option<ValType> {
    match type_of(db, scrutinee) {
        Ty::Int(it) => Some(m_slot(it)),
        Ty::Bool => Some(ValType::I32),
        _ => None,
    }
}

/// The reusable [`OperandSrc`] for a match scrutinee that need NOT be stashed — a parameter/kept-local
/// (re-`local.get` is free) or a compile-time constant (re-materialized inline). `None` for a computed
/// scrutinee, which the caller evaluates once into a scratch slot. (A constant scrutinee normally folds
/// away in `lower` before reaching a runtime match, but handling it keeps the source uniform.)
fn reusable_scalar_src(
    db: &mut Db,
    scrutinee: StructId,
    slots: &HashMap<StructId, u32>,
) -> Option<OperandSrc> {
    match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => {
            slots.get(&binder).copied().map(OperandSrc::Slot)
        }
        Core::ConstInt(v) => match type_of(db, scrutinee) {
            Ty::Int(it) if it.ground_width() <= 32 => {
                Some(OperandSrc::ConstI32(v.to_i32_bits(it.ground_width())))
            }
            _ => Some(OperandSrc::ConstI64(v.to_i64_bits())),
        },
        Core::ConstBool(b) => Some(OperandSrc::ConstI32(if b { 1 } else { 0 })),
        _ => None,
    }
}

/// The probe chain over a match's arms, dispatching on a scrutinee already resolved to `src` (pushed
/// once per probe — a local read or an inline constant, never a recomputation). See
/// [`emit_match_arms_tailable`], which resolves `src` and (for a computed scrutinee) evaluates it once.
#[allow(clippy::too_many_arguments)]
fn emit_probe_chain(
    db: &mut Db,
    src: OperandSrc,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
    tail: TailPos,
) -> Result<(), Reject> {
    // An arm body is emitted via `emit_arm_body` (grounds a bare-`ConstInt` body to the match's result
    // width, threads the tail context). The chain dispatches per arm below.
    let Some((arm, rest)) = arms.split_first() else {
        // No arm matched and no wildcard — `lower` forbids this for a runtime match, so it is a
        // compiler bug if reached. Decline rather than emit an undefined fallthrough.
        return Err(Reject::decline(
            "match ran off the end with no wildcard arm",
        ));
    };
    // An UNGUARDED arm whose probe always matches — a wildcard, or the LAST arm of an exhaustive
    // wildcard-less match (its probe redundant since every earlier probe failed) — is the unconditional
    // tail: emit its body at THIS nesting, no `if`. A GUARDED arm is NEVER an unconditional tail (its
    // guard may fail), so it always emits a test; `lower`'s exhaustiveness guarantees a later UNGUARDED
    // cover, so the chain still terminates.
    let probe_redundant = matches!(arm.probe, crate::core::Probe::Wild) || rest.is_empty();
    if arm.guard.is_none() && probe_redundant {
        return emit_arm_body(
            db, arm.body, result_it, slots, base, high, scratch_ty, layout, out, tail,
        );
    }
    // The matched body AND the `else` recursion are both INSIDE this `if` block — so a self-loop `br`
    // from either must jump one MORE level out to reach the loop top (depth + 1).
    let inner = deeper_tail(tail);
    // The arm's TEST: `probe` (scrutinee == literal), AND its `guard` when present. A `Wild` probe has no
    // literal test — the guard alone gates it. To preserve short-circuit trap semantics (the guard is
    // evaluated only when the probe matched — a guard MAY contain a trapping op), a literal-probe-plus-
    // guard nests the guard inside the probe's `if`; the two else-arms both fall through to `rest`.
    let has_literal_probe = !matches!(arm.probe, crate::core::Probe::Wild);
    if has_literal_probe {
        // `if (scrutinee == literal) <guard-gated body> else <rest>`.
        src.push(out);
        match &arm.probe {
            crate::core::Probe::Int(v) => {
                let m = Machine::of(it);
                out.push(m.konst(v.to_i64_bits()));
                out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
            }
            crate::core::Probe::Bool(b) => {
                out.push(Lir::ConstI32(if *b { 1 } else { 0 }));
                out.push(Lir::I32Eq);
            }
            crate::core::Probe::Wild => unreachable!("has_literal_probe"),
        }
        out.push(Lir::If(block_ty));
        emit_arm_guarded_body(
            db, arm, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, inner,
        )?;
        out.push(Lir::Else);
        emit_probe_chain(
            db, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty, layout, out,
            inner,
        )?;
        out.push(Lir::End);
        Ok(())
    } else {
        // A `Wild` probe with a guard: the guard alone gates the arm — `if guard body else rest`.
        emit_arm_guarded_body(
            db, arm, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, inner,
        )
    }
}

/// A [`TailPos`] one `if` block deeper — a self-loop `br` from inside a fresh `if` targets one level
/// further out. Shared by the probe chain and the guarded-body emit (each opens an `if`).
fn deeper_tail(tail: TailPos) -> TailPos {
    match tail {
        TailPos::Tail(tl) => TailPos::Tail(tl.map(|t| TailLoop {
            depth: t.depth + 1,
            ..t
        })),
        TailPos::NonTail => TailPos::NonTail,
    }
}

/// Emit a match-arm BODY at [`TailPos`] `tp`. Every arm produces the match's RESULT type, so a bare
/// `ConstInt` body is grounded to the result's integer width (`result_it`) — else a default-Int64 literal
/// arm beside a narrow arm pushes a mismatched slot and wasm rejects the block. A tail body goes through
/// `emit_tail` (a `ConstInt` is never a tail call); `tp` carries the self-loop context.
#[allow(clippy::too_many_arguments)]
fn emit_arm_body(
    db: &mut Db,
    body: StructId,
    result_it: Option<IntTy>,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
    tp: TailPos,
) -> Result<(), Reject> {
    if let (Some(rit), Core::ConstInt(_)) = (result_it, core_of(db, body)) {
        return emit_operand(db, body, rit, slots, base, high, scratch_ty, layout, out);
    }
    match tp {
        TailPos::Tail(tl) => emit_tail(db, body, slots, base, high, scratch_ty, layout, out, tl),
        TailPos::NonTail => emit(db, body, slots, base, high, scratch_ty, layout, out),
    }
}

/// Emit a GUARDED arm's body gated on its guard (the caller has already established that the arm's
/// PROBE matched — for a literal probe, inside its `if`; for a `Wild` probe, unconditionally). Emits
/// `if guard body else <rest>` — a false guard falls through to the remaining arms, exactly as a
/// non-matching pattern does (`core-semantics.md` §Matching Is Exhaustive Or Rejected). An UNGUARDED arm
/// (reached only via a literal probe whose guard is `None`) emits its body directly. The guard is a
/// boolean value (an i32); it is emitted at `base` (a fresh scratch region, its result consumed by the
/// `if`).
#[allow(clippy::too_many_arguments)]
fn emit_arm_guarded_body(
    db: &mut Db,
    arm: &crate::core::MatchArm,
    src: OperandSrc,
    rest: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
    inner: TailPos,
) -> Result<(), Reject> {
    match arm.guard {
        None => emit_arm_body(
            db, arm.body, result_it, slots, base, high, scratch_ty, layout, out, inner,
        ),
        Some(g) => {
            // `if guard body else <rest>`. The guard is a plain boolean value (never a tail call), so it
            // is emitted with `emit` at `base`; its result is the `if` condition.
            emit(db, g, slots, base, high, scratch_ty, layout, out)?;
            out.push(Lir::If(block_ty));
            // Both the body and the fallthrough are one `if` deeper than this arm's nesting.
            let deeper = deeper_tail(inner);
            emit_arm_body(
                db, arm.body, result_it, slots, base, high, scratch_ty, layout, out, deeper,
            )?;
            out.push(Lir::Else);
            emit_probe_chain(
                db, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty, layout, out,
                deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
    }
}

/// Emit one SWITCH of the decision tree: for each variant arm, `sum-disc(<scrutinee walked to `path`>)
/// == disc`, then `if (block_ty) <continuation> else <rest>`; a default arm (`disc: None`) or the LAST
/// arm (its probe redundant — every earlier disc has been tested and this is the only one left) is the
/// unconditional tail. `path` reaches the sub-value THIS switch dispatches on — empty for the ROOT (the
/// scrutinee itself), a `[Payload…]` path for a NESTED switch. Each arm's CONTINUATION is a leaf body or
/// a deeper switch (`emit_sum_cont`), which is what makes the whole match a decision tree that shares the
/// outer probe. Mirrors `emit_match_arms_tailable` but probes the discriminant.
#[allow(clippy::too_many_arguments)]
fn emit_sum_match_arms(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    arms: &[crate::core::SumArm],
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    match arms.split_first() {
        None => Err(Reject::decline(
            "sum match ran off the end with no covering arm",
        )),
        // A default arm, or the last arm of an exhaustive switch — its probe is redundant, so emit its
        // continuation unconditionally.
        Some((arm, [])) => emit_sum_cont(
            db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out,
        ),
        Some((arm, _)) if arm.disc.is_none() => emit_sum_cont(
            db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out,
        ),
        Some((arm, rest)) => {
            let disc = arm.disc.expect("non-None handled above");
            // sum-disc(<scrutinee walked down `path`>) == disc.
            emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle]
            for step in path {
                match step {
                    crate::core::PathStep::Payload => out.push(Lir::CallImport(OP_SUM_PAYLOAD)),
                    crate::core::PathStep::Elem(i) => {
                        out.push(Lir::ConstI32(*i as i32));
                        out.push(Lir::CallImport(OP_ARR_GET));
                    }
                }
            }
            out.push(Lir::CallImport(OP_SUM_DISC)); // → [disc]
            out.push(Lir::ConstI32(disc as i32));
            out.push(Lir::I32Eq);
            out.push(Lir::If(block_ty));
            emit_sum_cont(
                db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty,
                layout, out,
            )?;
            out.push(Lir::Else);
            emit_sum_match_arms(
                db, scrutinee, path, rest, result_it, block_ty, slots, base, high, scratch_ty,
                layout, out,
            )?;
            out.push(Lir::End);
            Ok(())
        }
    }
}

/// Emit a matched arm's CONTINUATION: a LEAF emits its body (a bare-`ConstInt` body grounded to the
/// match's result width `result_it`, as the scalar-match arms are); a nested SWITCH emits a fresh switch
/// chain on its deeper sub-value (`emit_sum_match_arms`), which is the decision tree recursing to share
/// the outer probe. The nested switch's `if`s reuse the SAME `block_ty` (both branches yield the match's
/// one result type at every depth).
#[allow(clippy::too_many_arguments)]
fn emit_sum_cont(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            if let (Some(rit), Core::ConstInt(_)) = (result_it, core_of(db, *body)) {
                return emit_operand(db, *body, rit, slots, base, high, scratch_ty, layout, out);
            }
            emit(db, *body, slots, base, high, scratch_ty, layout, out)
        }
        crate::core::SumCont::Switch { path, arms } => emit_sum_match_arms(
            db, scrutinee, path, arms, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out,
        ),
    }
}

/// The MACHINE realization of an integer type of width `N` and a signedness — the width-generic engine
/// every runtime op is emitted through. A value of width `N` lives in the smallest wasm slot that holds
/// it: an i32 for `N ≤ 32`, else an i64 (`slot32`). It sits there NORMALIZED — sign-extended if signed,
/// zero-extended if unsigned — which is exactly what the boundary lift and the constant emit produce, so
/// a machine op reads the true value. `Machine` carries the constants and op selectors keyed by the slot
/// width, plus whether `N` is NARROW (`N < slot bits`, so a machine op can produce a value that fits the
/// slot but not the N-bit type — caught by a range-check) versus FULL (`N == slot bits`, where the
/// machine op's own carry/borrow IS the type's overflow). Nothing here hard-codes 64.
#[derive(Clone, Copy)]
struct Machine {
    /// The language width `N` (1..=64).
    width: u32,
    signed: bool,
    /// Whether the value occupies an i32 slot (`N ≤ 32`) rather than an i64.
    slot32: bool,
}

impl Machine {
    fn of(it: IntTy) -> Machine {
        let width = it.ground_width();
        Machine {
            width,
            signed: it.ground_signed(),
            slot32: width <= 32,
        }
    }

    /// The bits of the machine slot (32 or 64).
    fn slot_bits(self) -> u32 {
        if self.slot32 { 32 } else { 64 }
    }

    /// The wasm value type of this machine's slot — the type a scratch local holding its value is
    /// declared at.
    fn slot(self) -> ValType {
        if self.slot32 {
            ValType::I32
        } else {
            ValType::I64
        }
    }

    /// Whether `N` is NARROWER than its slot — the case a range-check is needed after a machine op
    /// (a `FULL` width, `N == slot_bits`, is enforced entirely by the machine op's carry/borrow).
    fn narrow(self) -> bool {
        self.width < self.slot_bits()
    }

    /// A constant in this machine's slot (an i32 or i64 const of the given signed value).
    fn konst(self, v: i64) -> Lir {
        if self.slot32 {
            Lir::ConstI32(v as i32)
        } else {
            Lir::ConstI64(v)
        }
    }

    fn add(self) -> Lir {
        if self.slot32 {
            Lir::I32Add
        } else {
            Lir::I64Add
        }
    }
    fn sub(self) -> Lir {
        if self.slot32 {
            Lir::I32Sub
        } else {
            Lir::I64Sub
        }
    }
    fn mul(self) -> Lir {
        if self.slot32 {
            Lir::I32Mul
        } else {
            Lir::I64Mul
        }
    }
    fn and(self) -> Lir {
        if self.slot32 {
            Lir::I32And
        } else {
            Lir::I64And
        }
    }
    fn xor(self) -> Lir {
        if self.slot32 {
            Lir::I32Xor
        } else {
            Lir::I64Xor
        }
    }
    fn ne(self) -> Lir {
        if self.slot32 { Lir::I32Ne } else { Lir::I64Ne }
    }
    fn lt_s(self) -> Lir {
        if self.slot32 {
            Lir::I32LtS
        } else {
            Lir::I64LtS
        }
    }
    fn lt_u(self) -> Lir {
        if self.slot32 {
            Lir::I32LtU
        } else {
            Lir::I64LtU
        }
    }
    fn ge_u(self) -> Lir {
        if self.slot32 {
            Lir::I32GeU
        } else {
            Lir::I64GeU
        }
    }
    fn gt_s(self) -> Lir {
        if self.slot32 {
            Lir::I32GtS
        } else {
            Lir::I64GtS
        }
    }
    fn shl(self) -> Lir {
        if self.slot32 {
            Lir::I32Shl
        } else {
            Lir::I64Shl
        }
    }
    fn shr(self) -> Lir {
        match (self.slot32, self.signed) {
            (true, true) => Lir::I32ShrS,
            (true, false) => Lir::I32ShrU,
            (false, true) => Lir::I64ShrS,
            (false, false) => Lir::I64ShrU,
        }
    }
    fn div(self) -> Lir {
        match (self.slot32, self.signed) {
            (true, true) => Lir::I32DivS,
            (true, false) => Lir::I32DivU,
            (false, true) => Lir::I64DivS,
            (false, false) => Lir::I64DivU,
        }
    }
    fn rem(self) -> Lir {
        match (self.slot32, self.signed) {
            (true, true) => Lir::I32RemS,
            (true, false) => Lir::I32RemU,
            (false, true) => Lir::I64RemS,
            (false, false) => Lir::I64RemU,
        }
    }

    /// The bitwise op for `&`/`|`/`^` at this machine width.
    fn bitwise(self, op: Prim) -> Lir {
        match (self.slot32, op) {
            (true, Prim::BitAnd) => Lir::I32And,
            (true, Prim::BitOr) => Lir::I32Or,
            (true, _) => Lir::I32Xor,
            (false, Prim::BitAnd) => Lir::I64And,
            (false, Prim::BitOr) => Lir::I64Or,
            (false, _) => Lir::I64Xor,
        }
    }

    /// This width's inclusive bounds `[min_N, max_N]` as machine-slot values. A signed N holds
    /// `-(2^(N-1)) ..= 2^(N-1)-1`; an unsigned N holds `0 ..= 2^N-1`. At `N == slot_bits` (64 or 32) the
    /// bounds ARE the slot extremes, so the range-check is skipped (see `narrow`); this is only consulted
    /// when narrow, so `N < slot_bits ≤ 64` and every bound fits an i64. Computed via `u64` so the shift
    /// never overflows an `i64` (`2^63` as an intermediate would).
    fn bounds(self) -> (i64, i64) {
        if self.signed {
            let half = 1i64 << (self.width - 1); // width ≤ 63 here, so 2^(width-1) ≤ 2^62 fits i64
            (-half, half - 1)
        } else {
            let max = ((1u64 << self.width) - 1) as i64; // width ≤ 63 here, so 2^width - 1 ≤ 2^63 - 1
            (0, max)
        }
    }
}

/// Emit a CHECKED `+`/`-`/`*` that TRAPS when the true result leaves the N-bit type (the numeric-model
/// default). Two composed guards make it correct at ANY width, over scratch locals `$a=base`,
/// `$b=base+1`, `$r=base+2`:
///
///   <A> set$a ; <B> set$b ; get$a get$b <machine-op> set$r ; <M-overflow guard> ; <range-check> ; get$r
///
/// The machine op (`add`/`sub`/`mul` in the i32 or i64 slot) is bit-identical for signed and unsigned.
/// STEP 1, the M-OVERFLOW guard, traps when the true result does not fit the MACHINE slot — needed only
/// when the machine op can overflow it: `+`/`-` at a FULL width (`N == slot bits`), and `*` whenever a
/// full-width product can exceed the slot. After it, `$r` holds the EXACT result as a slot value. STEP 2,
/// the RANGE-CHECK, traps when `$r` fits the slot but not `[min_N, max_N]` — needed when `N` is NARROW.
/// This is what makes a narrow width (Int8's `100+100=200`, a UInt48 `*` past `2^48`) trap. Together they
/// trap iff the true result leaves the N-bit type. The per-op M-overflow tests, SIGNED (validated against
/// exact arithmetic in the seed compiler, mul over 172k random cases) — add `((r^a)&(r^b))<0`, sub
/// `((a^b)&(a^r))<0`, mul `a≠0 && r/a≠b` (`div_s` traps MIN/-1 itself) — and UNSIGNED (carry/borrow out of
/// the slot) — add `r <ᵤ a`, sub `a <ᵤ b`, mul `a≠0 && r/ᵤa≠b`.
///
/// LIVENESS / minimal locals: both operands recurse at `base+3` (NOT disjoint ranges) — operand A is
/// stored into `$a` before B's code runs, so A's scratch `[base+3..]` is DEAD during B and B safely
/// reuses it. The declared-locals count is therefore `max(A-scratch, B-scratch)+3`, not the sum — the
/// high-water mark in `high` captures exactly that.
/// Emit a binary op's OPERAND at the operation's width. A binary integer op's two operands must share
/// one machine slot (i32 for a ≤32-bit op, i64 otherwise) — wasm rejects a mixed `i32`/`i64` op. A
/// bare integer LITERAL is width-polymorphic (it defaults to Int64 = an i64 slot when typed on its
/// own), so a `(+ x 1)` / `(> x 50)` over a NARROW parameter `x` would otherwise push the literal as
/// an i64 beside `x`'s i32 and produce invalid wasm. Ground a bare-literal operand to the OP's width
/// `it` here (the width unification the per-node `type_of` does not thread back to the operand). A
/// non-literal operand carries its own machine width already and is emitted unchanged.
#[allow(clippy::too_many_arguments)]
fn emit_operand(
    db: &mut Db,
    id: StructId,
    it: IntTy,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    if let Core::ConstInt(v) = core_of(db, id) {
        let width = it.ground_width();
        if !v.fits_width(it.ground_signed(), width) {
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
        return Ok(());
    }
    emit(db, id, slots, base, high, scratch_ty, layout, out)
}

/// Emit an `if`/`match` branch (or arm) body producing the construct's RESULT type. Both branches must
/// leave the same machine slot on the stack (the block's result type), so a bare-literal branch — a
/// width-polymorphic `ConstInt` that defaults to Int64 — is GROUNDED to the result's integer width
/// (`emit_operand`), exactly as an operator operand is: else a default-Int64 literal branch opposite a
/// NARROW branch pushes a mismatched i64 into a narrow-i32 block and wasm rejects the function. A
/// non-literal branch, or a non-integer result, emits normally.
#[allow(clippy::too_many_arguments)]
fn emit_branch(
    db: &mut Db,
    id: StructId,
    result: &Ty,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    if let (Ty::Int(rit), Core::ConstInt(_)) = (result, core_of(db, id)) {
        return emit_operand(db, id, *rit, slots, base, high, scratch_ty, layout, out);
    }
    emit(db, id, slots, base, high, scratch_ty, layout, out)
}

/// Whether an `if`'s BRANCH is cheap enough to compute UNCONDITIONALLY for a branchless `select`: a
/// leaf that costs one instruction and can neither trap nor allocate — a parameter, a kept `let`-local,
/// or a compile-time constant. A `select` evaluates BOTH operands (there is no short-circuit), so a
/// heavier branch would waste the work the `if` avoided, and a trapping/allocating branch would change
/// behavior (a trap on the untaken side, a leaked heap cell); a leaf is safe on both counts.
fn is_select_leaf(db: &mut Db, id: StructId) -> bool {
    matches!(
        core_of(db, id),
        Core::Param { .. } | Core::LocalRef { .. } | Core::ConstInt(_) | Core::ConstBool(_)
    )
}

/// How a checked-arith operand is pushed onto the stack at each of its use sites (the machine op AND
/// every guard re-read). An operand read many times need not be copied into a scratch local IF it is
/// cheap and side-effect-free to re-materialize:
///  - `Slot` — the operand already lives in a wasm local (a parameter, a kept `let`-binding, or a
///    scratch slot a non-reusable operand was stored into); push is `local.get`.
///  - `Const` — the operand is a compile-time integer; push is the grounded `i32.const`/`i64.const`
///    directly, so it needs neither a scratch slot nor a `local.set`.
///
/// Deciding the source ONCE (in [`operand_src`]) and pushing it at each site keeps the machine op and
/// the guard in agreement and removes the store+slot for a reusable operand.
#[derive(Clone, Copy)]
enum OperandSrc {
    Slot(u32),
    ConstI32(i32),
    ConstI64(i64),
}

impl OperandSrc {
    /// Push this operand's value onto the stack (`local.get slot`, or the constant push).
    fn push(self, out: &mut Vec<Lir>) {
        match self {
            OperandSrc::Slot(slot) => out.push(Lir::LocalGet(slot)),
            OperandSrc::ConstI32(v) => out.push(Lir::ConstI32(v)),
            OperandSrc::ConstI64(v) => out.push(Lir::ConstI64(v)),
        }
    }
}

/// The reusable operand source for `id` at machine slot type `slot_ty`, or `None` if the operand must
/// be computed and stashed in a scratch slot (a nested computation). A REUSABLE operand is one that is
/// side-effect-free and cheap to re-emit at every use site — so no scratch local and no `local.set`:
///  - a parameter (`Core::Param`) or kept `let`-binding (`Core::LocalRef`) already in a local of the
///    op's machine type (a narrow local feeding a wider op does NOT match — its i32 slot ≠ the i64 op);
///  - a compile-time integer (`Core::ConstInt`) that fits the op width, grounded to the op width `ot`
///    (the same range-check + bit-pattern `emit_operand` applies to an inline literal, so an
///    out-of-range constant still declines — CDZ0302 — rather than silently truncating).
fn operand_src(
    db: &mut Db,
    id: StructId,
    ot: IntTy,
    slots: &HashMap<StructId, u32>,
) -> Result<Option<OperandSrc>, Reject> {
    match core_of(db, id) {
        Core::Param { binder } | Core::LocalRef { binder } => {
            let Some(&slot) = slots.get(&binder) else {
                return Ok(None);
            };
            // The operand must live in a slot of the op's machine type; else reading it would feed a
            // mismatched i32/i64 into the machine op. A same-width operand matches; a narrow operand
            // feeding a wider op does not and takes the copy path (where `emit_operand` widens it).
            if valtype_of(&type_of(db, id)) == Some(m_slot(ot)) {
                Ok(Some(OperandSrc::Slot(slot)))
            } else {
                Ok(None)
            }
        }
        Core::ConstInt(v) => {
            // A constant is re-materializable for free — inline it (grounded to the op width) at each
            // use, so it needs no scratch slot. Same range-check as `emit_operand`: out of range
            // declines, never truncates.
            let width = ot.ground_width();
            if !v.fits_width(ot.ground_signed(), width) {
                return Err(Reject::coded(
                    Code::IntOutOfRange,
                    "integer literal does not fit its width",
                ));
            }
            let src = if width <= 32 {
                OperandSrc::ConstI32(v.to_i32_bits(width))
            } else {
                OperandSrc::ConstI64(v.to_i64_bits())
            };
            Ok(Some(src))
        }
        _ => Ok(None),
    }
}

/// The wasm machine slot (i32 for a ≤32-bit width, i64 otherwise) for an integer op of type `ot` —
/// the same choice [`Machine::slot`] makes, computed straight from the `IntTy` so `operand_src` need
/// not build a `Machine`.
fn m_slot(ot: IntTy) -> ValType {
    if ot.ground_width() <= 32 {
        ValType::I32
    } else {
        ValType::I64
    }
}

/// Whether the nodes at `a` and `b` lower to the STRUCTURALLY IDENTICAL core computation — the basis
/// for common-subexpression elimination. Two nodes are equal iff their core forms are the same operator
/// over recursively-equal operands, bottoming out at the same param/local slot or the same constant.
/// This is used ONLY to decide whether a repeated operand can be computed ONCE and read twice, so it is
/// deliberately CONSERVATIVE: any core kind not enumerated here (a call, a conditional, a heap
/// construct — whose equality would need more than structural matching, or whose sharing is not clearly
/// safe) compares UNEQUAL, so CSE simply does not fire. Every kind that DOES compare equal is a PURE,
/// deterministic scalar computation (arithmetic/comparison/conversion/projection over equal operands,
/// or a leaf) — so computing it once and reusing the value is observably identical to computing it
/// twice, INCLUDING its trap behavior (a trapping subexpression traps at the same first-occurrence
/// point whether shared or not). Effects would break this, but rcdzc has none yet.
fn core_eq(db: &mut Db, a: StructId, b: StructId) -> bool {
    if a == b {
        return true; // the SAME occurrence — trivially identical.
    }
    match (core_of(db, a), core_of(db, b)) {
        (Core::ConstInt(x), Core::ConstInt(y)) => x.eq_value(&y),
        (Core::ConstBool(x), Core::ConstBool(y)) => x == y,
        (Core::Unit, Core::Unit) => true,
        // A leaf reference: equal iff the SAME binder (same param/local slot → same value).
        (Core::Param { binder: x }, Core::Param { binder: y }) => x == y,
        (Core::LocalRef { binder: x }, Core::LocalRef { binder: y }) => x == y,
        // A pure binary op: same operator over recursively-equal operands. (Arithmetic and comparison
        // are the operators whose two runtime operands can be the shared subexpression.)
        (
            Core::Arith {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Arith {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        )
        | (
            Core::Compare {
                op: ox,
                lhs: lx,
                rhs: rx,
            },
            Core::Compare {
                op: oy,
                lhs: ly,
                rhs: ry,
            },
        ) => ox == oy && core_eq(db, lx, ly) && core_eq(db, rx, ry),
        // A pure conversion: same op over an equal operand.
        (
            Core::Convert {
                op: ox,
                operand: px,
            },
            Core::Convert {
                op: oy,
                operand: py,
            },
        ) => ox == oy && core_eq(db, px, py),
        // A tuple projection: same index off an equal (runtime) operand.
        (
            Core::Proj {
                operand: px,
                index: ix,
            },
            Core::Proj {
                operand: py,
                index: iy,
            },
        ) => ix == iy && core_eq(db, px, py),
        _ => false,
    }
}

/// Where a checked op leaves its result:
///  - `Stack` — the usual case: the result is left on the operand stack (via `local.get $r`), for the
///    enclosing expression to consume.
///  - `Slot(d)` — the caller wants the result in local `d` and NOT on the stack. Used when this op is
///    an OPERAND of an enclosing checked op: the enclosing op would otherwise `emit_operand(this) ;
///    local.set d` — computing this result into its own `$r` then COPYING it to `d`. Passing `Slot(d)`
///    makes THIS op use `d` as its `$r` directly, so its final `local.set` IS the store and the copy
///    (`local.get $r_inner ; local.tee d`) vanishes, along with the separate `$r_inner` scratch slot.
#[derive(Clone, Copy)]
enum ResultDest {
    Stack,
    Slot(u32),
}

#[allow(clippy::too_many_arguments)]
fn emit_checked_arith(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    emit_checked_arith_to(
        db,
        op,
        m,
        lhs,
        rhs,
        slots,
        base,
        high,
        scratch_ty,
        layout,
        out,
        ResultDest::Stack,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_checked_arith_to(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
    dest: ResultDest,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // Each operand's SOURCE at every use site (the machine op + the guard's re-reads): a reusable
    // operand — a matching local, or a compile-time constant — is pushed directly (`local.get` / an
    // inline `const`) and needs NO scratch slot; only a nested computation is stashed in a fresh
    // scratch slot (source = that slot). `$r` (the result) always needs its own scratch. Scratch slots
    // are claimed from `base`; the operand recursion floats ABOVE whatever scratch is actually used, so
    // an operand that needs no copy also frees the slot it would have occupied.
    let mut next_scratch = base;
    let mut claim = |high: &mut u32| {
        let s = next_scratch;
        next_scratch += 1;
        if s + 1 > *high {
            *high = s + 1;
        }
        s
    };
    // A reusable source is settled now; a non-reusable operand claims a scratch slot to be stored into.
    let sa_src = operand_src(db, lhs, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    // COMMON-SUBEXPRESSION ELIMINATION: if B is a non-reusable computation STRUCTURALLY IDENTICAL to A,
    // it produces the same value — so compute it ONCE (as A, into `$a`) and read `$a` for B too, rather
    // than emitting the whole computation (and its guards) a second time. `(+ (* a b) (* a b))` becomes
    // one `*` + one guard, read twice. Safe because `core_eq` only matches PURE deterministic scalar
    // computations (see its doc) — no effects, and a trapping operand traps identically. Only fires when
    // A itself was stashed in a slot (`sa` is a Slot): a reusable A (a bare local/const) is already free
    // to re-push, so B just shares that same source with no CSE needed.
    let sb_src = operand_src(db, rhs, ot, slots)?;
    // `sb_shares_a` records that B is the SAME computation as A and reuses A's slot (CSE) — so B is NOT
    // emitted separately below (it would recompute into A's slot). Distinct from `sb_src.is_some()` (a
    // reusable source that also skips the emit but for a different reason).
    let mut sb_shares_a = false;
    let sb = match sb_src {
        Some(src) => src,
        None if matches!(sa, OperandSrc::Slot(_)) && core_eq(db, lhs, rhs) => {
            trace!(target: "rcdzc::select", lhs = lhs.0, rhs = rhs.0, "CSE: identical operands share one computation");
            sb_shares_a = true;
            sa
        }
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    // `$r` (the result slot): the caller-requested destination when this op is an operand of an
    // enclosing op (`Slot(d)`), else a fresh scratch slot. Using `d` directly means this op's final
    // `local.set` IS the store the enclosing op wanted — no copy. `d` is one of the enclosing op's
    // operand slots, claimed BELOW this op's `base`, so this op's own operand scratch (claimed from
    // `base` up) never collides with it.
    let sr = match dest {
        ResultDest::Slot(d) => d,
        ResultDest::Stack => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            s
        }
    };
    // Operands that DO need a copy recurse above the scratch slots claimed so far; A is stored before
    // B runs, so B may reuse A's operand scratch (the liveness the high-water mark captures).
    let operand_base = next_scratch;
    // <A> compute A into $a — only when A is a stashed (non-reusable) operand; a reusable source is
    // pushed in place at each use. `emit_operand_into` writes the result straight into `$a`: a nested
    // checked op targets `$a` as its own result slot (no copy), any other operand is `emit_operand`ed
    // then `local.set $a`. A bare-literal operand is grounded to the OP's width `ot` by `emit_operand`.
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            lhs,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // <B> compute B into $b — only for a stashed operand that is NOT shared with A (CSE). When
    // `sb_shares_a`, B's value already sits in A's slot (computed once), so re-emitting it would both
    // recompute and clobber — skip it.
    if sb_src.is_none()
        && !sb_shares_a
        && let OperandSrc::Slot(sb_slot) = sb
    {
        emit_operand_into(
            db,
            rhs,
            ot,
            sb_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // push$a push$b <machine-op> local.set $r
    sa.push(out);
    sb.push(out);
    out.push(match op {
        Prim::Add => m.add(),
        Prim::Sub => m.sub(),
        Prim::Mul => m.mul(),
        _ => return Err(Reject::decline("not a checked arithmetic op")),
    });
    out.push(Lir::LocalSet(sr));
    // Step 1: the machine-slot overflow guard (only where the machine op can overflow its slot).
    emit_machine_overflow_guard(op, m, sa, sb, sr, out);
    // Step 2: the narrow-width range-check on the exact result in `$r`.
    emit_range_check(m, sr, out);
    // The result. `Stack` leaves it on the operand stack (`local.get $r`) for the enclosing expression;
    // `Slot(d)` means `$r` IS `d` and the caller wants the value only in the slot, so nothing is pushed
    // (the `local.set $r` above already stored it) — this is where the copy-into-the-operand-slot goes
    // away.
    if matches!(dest, ResultDest::Stack) {
        out.push(Lir::LocalGet(sr));
    }
    Ok(())
}

/// Emit operand `id` (at op width `ot`) so its value ends up in local `slot`. When `id` is itself a
/// nested checked `+`/`-`/`*`, it is emitted with `ResultDest::Slot(slot)` so its own result store
/// writes `slot` directly — no `emit_operand` + separate `local.set`, hence no `local.get $r_inner ;
/// local.tee slot` copy and no extra `$r_inner` scratch. Any other operand (a projection, a call, a
/// conversion, a shift/bitwise, a literal, …) is `emit_operand`ed onto the stack then `local.set slot`.
#[allow(clippy::too_many_arguments)]
fn emit_operand_into(
    db: &mut Db,
    id: StructId,
    ot: IntTy,
    slot: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    if let Core::Arith { op, lhs, rhs } = core_of(db, id)
        && matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
    {
        let m = Machine::of(int_ty_of(db, id));
        return emit_checked_arith_to(
            db,
            op,
            m,
            lhs,
            rhs,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            ResultDest::Slot(slot),
        );
    }
    emit_operand(db, id, ot, slots, base, high, scratch_ty, layout, out)?;
    out.push(Lir::LocalSet(slot));
    Ok(())
}

/// The machine-slot overflow guard for `(op a b)` with result in `$r` — traps (`if (empty) unreachable
/// end`) when the true result does not fit the MACHINE slot. For a NARROW `+`/`-` the machine add/sub
/// cannot overflow its slot (operands are far from the slot extremes), so the guard is skipped and the
/// range-check alone bounds the result; `*` always runs the `r/a≠b` guard (a narrow product can still
/// exceed the slot — e.g. two 48-bit values multiply past 2^64). See `emit_checked_arith`.
fn emit_machine_overflow_guard(
    op: Prim,
    m: Machine,
    sa: OperandSrc,
    sb: OperandSrc,
    sr: u32,
    out: &mut Vec<Lir>,
) {
    // `+`/`-` overflow the slot only at a FULL width; a narrow add/sub stays within the slot.
    let addsub_can_overflow = !m.narrow();
    match op {
        Prim::Add if addsub_can_overflow && m.signed => {
            // signed add: `((r^a) & (r^b)) < 0` → trap.
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.xor());
            out.push(Lir::LocalGet(sr));
            sb.push(out);
            out.push(m.xor());
            out.push(m.and());
            out.push(m.konst(0));
            out.push(m.lt_s());
            out.push(Lir::IfUnreachableEnd);
        }
        Prim::Add if addsub_can_overflow => {
            // unsigned add: `r <ᵤ a` → trap (the sum carried out of the slot).
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.lt_u());
            out.push(Lir::IfUnreachableEnd);
        }
        Prim::Sub if addsub_can_overflow && m.signed => {
            // signed sub: `((r^a) & (a^b)) < 0` → trap. Mathematically `((a^b) & (a^r)) < 0`, but `^`
            // and `&` are commutative, so we compute `(r^a)` FIRST — reading `$r` immediately after the
            // `local.set $r` that produced the result, so the peephole fuses that `set ; get` into a
            // `local.tee $r` (one fewer instruction). `(r^a)` ≡ `(a^r)`, `(r^a)&(a^b)` ≡ `(a^b)&(a^r)`.
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.xor());
            sa.push(out);
            sb.push(out);
            out.push(m.xor());
            out.push(m.and());
            out.push(m.konst(0));
            out.push(m.lt_s());
            out.push(Lir::IfUnreachableEnd);
        }
        Prim::Sub if addsub_can_overflow => {
            // unsigned sub: `a <ᵤ b` → trap (an unsigned value cannot go below 0). For a NARROW unsigned
            // width the machine sub CAN go negative in the slot (below 0), which the range-check then
            // catches — but the unsigned-underflow meaning is clearer as this direct test, and it holds
            // at full width where the range-check is a no-op. (A narrow signed/unsigned sub also relies on
            // the range-check for the upper edge, which never trips for sub.)
            sa.push(out);
            sb.push(out);
            out.push(m.lt_u());
            out.push(Lir::IfUnreachableEnd);
        }
        Prim::Mul => {
            // mul: `if a≠0 { if r/a ≠ b { unreachable } }` — guards div against a=0 (a=0 can't overflow);
            // the machine `div_s` traps on MIN/-1 itself (the sole case `r/a` can't detect at full width),
            // `div_u` is total for a≠0. Runs at every width — a narrow product can exceed the slot too.
            sa.push(out);
            out.push(m.konst(0));
            out.push(m.ne());
            out.push(Lir::If(BlockType::Empty)); // if a != 0 {
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.div());
            sb.push(out);
            out.push(m.ne());
            out.push(Lir::IfUnreachableEnd); //   if (r/a) != b { unreachable }
            out.push(Lir::End); // }
        }
        _ => {}
    }
}

/// The narrow-width range-check on an exact result in `$r`: trap unless `min_N <= r <= max_N`. A no-op
/// at a FULL width (`N == slot bits`, where the slot extremes ARE the bounds). Emitted as two guards:
/// `r < min_N → trap` (signed compare — the bound and value are signed slot values) and `r > max_N →
/// trap`. For an unsigned narrow width `min_N = 0`, so the lower guard rejects a machine result that
/// went negative in the slot (an unsigned underflow), and the upper guard rejects `> 2^N-1`.
fn emit_range_check(m: Machine, sr: u32, out: &mut Vec<Lir>) {
    if !m.narrow() {
        return;
    }
    let (min_n, max_n) = m.bounds();
    // r < min_N → trap.
    out.push(Lir::LocalGet(sr));
    out.push(m.konst(min_n));
    out.push(m.lt_s());
    out.push(Lir::IfUnreachableEnd);
    // r > max_N → trap.
    out.push(Lir::LocalGet(sr));
    out.push(m.konst(max_n));
    out.push(m.gt_s());
    out.push(Lir::IfUnreachableEnd);
}

/// Emit a runtime `/`/`%`. The machine `div`/`rem` traps natively on ÷0 (all widths) and, for a FULL
/// signed width, on `MIN/-1` — exactly two of the defined traps. Two extra guards make it correct at any
/// width: a NARROW signed `/` whose `min_N / -1` overflows the type is NOT trapped by the machine op (the
/// quotient `2^(N-1)` fits the wider slot), so it is caught by a range-check on the result; `%` never
/// overflows (its result is bounded by the divisor), so it needs no range-check. Over scratch locals
/// `$a`,`$b`,`$r` when a range-check is required; otherwise a bare `operands; op` suffices.
#[allow(clippy::too_many_arguments)]
fn emit_div_rem(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    // A narrow signed division needs a range-check on the quotient (its `min_N / -1` overflows the type
    // but not the slot). Every other case — `%` (bounded by the divisor), unsigned `/` (magnitude only
    // shrinks), full-width signed `/` (the machine `div_s` traps MIN/-1 itself) — is exact after the
    // native trap, so no scratch is needed.
    let ot = IntTy::fixed(m.signed, m.width);
    let needs_range_check = matches!(op, Prim::Div) && m.signed && m.narrow();
    if !needs_range_check {
        emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
        emit_operand(db, rhs, ot, slots, base, high, scratch_ty, layout, out)?;
        out.push(if matches!(op, Prim::Div) {
            m.div()
        } else {
            m.rem()
        });
        return Ok(());
    }
    // Narrow signed `/`: compute into `$r`, then range-check.
    let sr = base;
    if sr + 1 > *high {
        *high = sr + 1;
    }
    scratch_ty.insert(sr, m.slot());
    let operand_base = base + 1;
    emit_operand(
        db,
        lhs,
        ot,
        slots,
        operand_base,
        high,
        scratch_ty,
        layout,
        out,
    )?;
    emit_operand(
        db,
        rhs,
        ot,
        slots,
        operand_base,
        high,
        scratch_ty,
        layout,
        out,
    )?;
    out.push(m.div()); // traps on ÷0 natively; the machine op does not overflow at a narrow width
    out.push(Lir::LocalSet(sr));
    emit_range_check(m, sr, out);
    out.push(Lir::LocalGet(sr));
    Ok(())
}

/// Emit a runtime `<<`/`>>` that GUARDS the shift count and (for `<<`) tests overflow, over scratch
/// locals `$a=base` (value), `$b=base+1` (count), `$r=base+2` (result). A wasm shift MASKS the count mod
/// the slot width and never traps, so a naive lowering would leak C-style undefined-shift behavior. The
/// numeric model forbids this: a count outside `[0, N)` has no defined value (trap), and a left shift is
/// exact multiplication by `2^count`, so it traps on overflow like `*`. The sequence:
///
///   <A> set$a ; <B> set$b
///   ; count guard: `b >=ᵤ N` → trap           (a negative count read unsigned is huge, so ≥ N too)
///   ; get$a get$b <machine-shift> set$r
///   ; <<-only: <M-overflow round-trip> ; <range-check>
///   ; get$r
///
/// The count is guarded against the LANGUAGE width `N` (not the slot width). `>>` never overflows, so it
/// has only the count guard; it is arithmetic (`shr_s`) for a signed type, logical (`shr_u`) for an
/// unsigned one. `<<`'s overflow has two parts: the round-trip `(r >>[s/u] b) != a` catches bits shifted
/// out of the SLOT, and the range-check catches a result that fits the slot but not the narrower N-bit
/// type — together the exact `2^count`-overflow at any width.
#[allow(clippy::too_many_arguments)]
fn emit_shift(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // The value `$a` and the count `$b` are read several times (count guard, the shift, the round-trip
    // check), so — like `emit_checked_arith` — a reusable operand (a matching local or a constant) is
    // pushed directly at each use (no scratch), and only a nested computation is stashed in a scratch
    // slot. `$r` (the result) always needs its own scratch. Both share the op's machine slot, so a
    // bare-literal value/count is grounded to that width (a mixed i32/i64 shift is invalid wasm).
    let mut next_scratch = base;
    let mut claim = |high: &mut u32| {
        let s = next_scratch;
        next_scratch += 1;
        if s + 1 > *high {
            *high = s + 1;
        }
        s
    };
    let sa_src = operand_src(db, lhs, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    let sb_src = operand_src(db, rhs, ot, slots)?;
    let sb = match sb_src {
        Some(src) => src,
        None => {
            let s = claim(high);
            scratch_ty.insert(s, m.slot());
            OperandSrc::Slot(s)
        }
    };
    let sr = claim(high);
    scratch_ty.insert(sr, m.slot());
    let operand_base = next_scratch;
    // Stash a non-reusable value/count into its scratch slot (a nested op writes it directly).
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            lhs,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    if sb_src.is_none()
        && let OperandSrc::Slot(sb_slot) = sb
    {
        emit_operand_into(
            db,
            rhs,
            ot,
            sb_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // Count guard: `b >=ᵤ N` → trap. A negative count read unsigned is huge (≥ N), so this one test
    // catches both a negative and a too-large count. Bound is the LANGUAGE width N, not the slot width.
    sb.push(out);
    out.push(m.konst(m.width as i64));
    out.push(m.ge_u());
    out.push(Lir::IfUnreachableEnd);
    // push$a push$b <machine-shift> local.set $r
    sa.push(out);
    sb.push(out);
    out.push(match op {
        Prim::Shl => m.shl(),
        Prim::Shr => m.shr(),
        _ => return Err(Reject::decline("not a shift op")),
    });
    out.push(Lir::LocalSet(sr));
    if matches!(op, Prim::Shl) {
        // Round-trip: shifting `$r` back right by `$b` must recover `$a`; else the shift dropped bits out
        // of the SLOT (overflow). The inverse shift matches signedness so the round-trip is exact.
        out.push(Lir::LocalGet(sr));
        sb.push(out);
        out.push(m.shr());
        sa.push(out);
        out.push(m.ne());
        out.push(Lir::IfUnreachableEnd);
        // Range-check: a narrow `<<` result may fit the slot but exceed the N-bit type.
        emit_range_check(m, sr, out);
    }
    out.push(Lir::LocalGet(sr));
    Ok(())
}

/// Emit a runtime `wrap` — TRUNCATE the operand (source machine `src`) to the target `dst`'s width and
/// signedness, keeping the low `dst.width` bits and reinterpreting them at the target sign. NEVER traps
/// (the whole point of `wrap`). Three composed steps, all width-generic:
///
///   1. emit the operand (it lands in the SOURCE slot, normalized to the source width);
///   2. MOVE it to the TARGET slot: `i32.wrap_i64` (i64→i32, drops the high half — which the mask would
///      drop anyway) or `i64.extend_i32_{s,u}` (i32→i64, extended by the SOURCE sign so the source value
///      is preserved before masking); same slot → nothing;
///   3. TRUNCATE to `dst.width` in the target slot when it is narrow (`dst.width < slot bits`): `and` the
///      low-`N`-bits mask, then — if the TARGET is signed — sign-extend from bit `N-1` via
///      `(x << (M-N)) >> (M-N)` (arithmetic shr). An unsigned target stops after the mask (zero-filled).
///
/// A full-width target (`dst.width == slot bits`) needs no truncation after the slot move — the slot IS
/// the width. The result is left normalized in the target slot, exactly as every other value.
#[allow(clippy::too_many_arguments)]
fn emit_wrap(
    db: &mut Db,
    src: Machine,
    dst: Machine,
    operand: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    // 1. The operand, in the source slot.
    emit(db, operand, slots, base, high, scratch_ty, layout, out)?;
    // 2. Move into the target slot (drop/extend the machine width). The extend is by the SOURCE sign so
    //    the source value's bits are preserved into the wider slot before the target mask.
    match (src.slot32, dst.slot32) {
        (false, true) => out.push(Lir::I32WrapI64), // i64 source → i32 target
        (true, false) => out.push(if src.signed {
            Lir::I64ExtendI32S
        } else {
            Lir::I64ExtendI32U
        }),
        _ => {} // same slot width — nothing to move
    }
    // 3. Truncate to the target width within the target slot, when narrower than the slot.
    if dst.narrow() {
        let slot_bits = dst.slot_bits();
        if dst.signed {
            // Sign-extend from bit N-1: `(x << (M-N)) >> (M-N)` with arithmetic (signed) shr. This both
            // masks (the << pushes the high bits out) and sign-fills. `dst.shr()` is arithmetic for a
            // signed dst.
            let shift = (slot_bits - dst.width) as i64;
            out.push(dst.konst(shift));
            out.push(dst.shl());
            out.push(dst.konst(shift));
            out.push(dst.shr());
        } else {
            // Zero-fill: mask to the low N bits.
            let mask = if dst.width >= 64 {
                -1i64 // all ones (unreachable for narrow, but total)
            } else {
                (1i64 << dst.width) - 1
            };
            out.push(dst.konst(mask));
            out.push(dst.and());
        }
    }
    Ok(())
}

/// The flat wasm comparison op for a relational prim over an operand integer type — the width chooses
/// i32 (≤32-bit, or a boolean operand) vs i64, and the SIGNEDNESS chooses `_s` (a signed type orders by
/// two's-complement value) vs `_u` (an unsigned type orders by magnitude). Equality is sign-agnostic
/// (the same bits compare equal either way). A ≤32-bit value is properly sign-/zero-extended in its
/// slot, so the i32 `_s`/`_u` ops compare it correctly.
fn compare_op(op: Prim, it: IntTy) -> Lir {
    let narrow = it.ground_width() <= 32;
    let signed = it.ground_signed();
    match (op, narrow, signed) {
        (Prim::Eq, false, _) => Lir::I64Eq,
        (Prim::Lt, false, true) => Lir::I64LtS,
        (Prim::Gt, false, true) => Lir::I64GtS,
        (Prim::Le, false, true) => Lir::I64LeS,
        (Prim::Ge, false, true) => Lir::I64GeS,
        (Prim::Lt, false, false) => Lir::I64LtU,
        (Prim::Gt, false, false) => Lir::I64GtU,
        (Prim::Le, false, false) => Lir::I64LeU,
        (Prim::Ge, false, false) => Lir::I64GeU,
        (Prim::Eq, true, _) => Lir::I32Eq,
        (Prim::Lt, true, true) => Lir::I32LtS,
        (Prim::Gt, true, true) => Lir::I32GtS,
        (Prim::Le, true, true) => Lir::I32LeS,
        (Prim::Ge, true, true) => Lir::I32GeS,
        (Prim::Lt, true, false) => Lir::I32LtU,
        (Prim::Gt, true, false) => Lir::I32GtU,
        (Prim::Le, true, false) => Lir::I32LeU,
        (Prim::Ge, true, false) => Lir::I32GeU,
        // Not a comparison — `Core::Compare` only ever carries a comparison prim, so unreachable.
        _ => Lir::I64Eq,
    }
}

/// The integer type governing a runtime comparison's operands — read off whichever operand solves to
/// an integer (they unify to one type). A boolean comparison has no integer operand, so it grounds to
/// the ≤32-bit path via the default `i64`… (a bool is compared as an i32 — see `Compare` selection,
/// which reads the operand's own `valtype`). Falls back to signed-64.
fn operand_int_ty(db: &mut Db, lhs: StructId, rhs: StructId) -> IntTy {
    // A boolean operand is an i32; represent that as a signed ≤32-bit width so `compare_op` picks i32.
    let bool_as_i32 = IntTy::fixed(true, 32);
    match type_of(db, lhs) {
        Ty::Int(it) => it,
        Ty::Bool => bool_as_i32,
        _ => match type_of(db, rhs) {
            Ty::Int(it) => it,
            Ty::Bool => bool_as_i32,
            _ => IntTy::i64(),
        },
    }
}

/// The integer type of the node at `id`, if its solved type is an integer — used to ground a literal's
/// width at selection. Defaults to the signed-64 instance when the node is not an integer (a
/// defensive fallback; a `ConstInt` node always types as an integer).
fn int_ty_of(db: &mut Db, id: StructId) -> IntTy {
    match type_of(db, id) {
        Ty::Int(it) => it,
        _ => IntTy::i64(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::scalar_program;

    /// Compute the boundary layout for a test program (all test fixtures have an export). `select_*`
    /// needs it to resolve a `Core::Call` callee's function index; these Lir-level tests exercise no
    /// call, so the layout's contents beyond the exported def are irrelevant — but a real one is passed
    /// so the signature is honest.
    fn layout_of(db: &mut Db) -> Layout {
        crate::layout::compute(db).expect("layout")
    }

    #[test]
    fn selects_a_literal_to_i64_const() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let f = select_body(&mut db, body, &layout).expect("select");
        assert_eq!(f.code, vec![Lir::ConstI64(42)]);
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn selects_a_runtime_if_with_leaf_branches_to_a_branchless_select() {
        // A RUNTIME condition (a bool param `p`) with two CHEAP TRAP-FREE LEAF branches (constants) —
        // the `if` selects to wasm's BRANCHLESS `select`, not a structured block: push the two branch
        // values then the condition, then `select` (which pops `[then, else, cond]` and pushes `then`
        // if `cond` is nonzero). `local.get 0` is the condition `p`. This replaces the old
        // `if (result i64) … else … end` control block — one instruction, no branch. (A CONSTANT
        // condition folds away in `lower`; a NON-leaf/heap/effecting branch keeps the structured `if`,
        // covered by `keeps_the_structured_if_when_a_branch_is_not_a_cheap_leaf`.)
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool)) (if p 1 2)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                Lir::ConstI64(1),
                Lir::ConstI64(2),
                Lir::LocalGet(0),
                Lir::Select,
            ]
        );
    }

    #[test]
    fn keeps_the_structured_if_when_a_branch_is_not_a_cheap_leaf() {
        // A branch that is NOT a cheap trap-free leaf (here `(+ a a)`, a checked add) must keep the
        // structured `if`/`else`/`end`: `select` evaluates BOTH branches unconditionally, so converting
        // a heavier/possibly-trapping branch would waste the work the `if` avoids (and could surface a
        // trap on the untaken side). So the wasm block survives with a real `if`. This pins the
        // eligibility gate `is_select_leaf` alongside the positive case above.
        let ast = crate::testkit::parse(
            "(module m (def (f (: p Bool) (: a Int64)) (if p a (+ a a))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            f.code.contains(&Lir::If(BlockType::Val(ValType::I64))),
            "a non-leaf branch keeps the structured if, got: {:?}",
            f.code
        );
        assert!(
            !f.code.contains(&Lir::Select),
            "a non-leaf branch must NOT use select, got: {:?}",
            f.code
        );
    }

    // ── runtime lowering: a parameterized function body selects to local reads + machine ops ──────
    //
    // These select a FUNCTION body standalone (as `select_function`, the path an exported function
    // takes) — the parameters are runtime values, so their references become `local.get` and the
    // operation is a runtime machine op, NOT folded. Asserted at the Lir level (no export/run yet).

    /// Locate def `name`'s parameter name-occurrences (seeing through `(: a T)`) and body, plus solve
    /// each param's type — the inputs `select_function` takes for an exported parameterized function.
    fn function_of(db: &mut Db, name: &str) -> (Vec<(StructId, Ty)>, StructId) {
        let d = db.def_by_name(name).expect("def present");
        let sig_params = db.defs[d].params.clone();
        let body = db.defs[d].body.expect("body");
        let mut params = Vec::new();
        for p in sig_params {
            // The name occurrence a reference binds to — bare `a` or the inner name of `(: a T)`.
            let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => name_occ,
                None => p,
            };
            let ty = type_of(db, binder);
            params.push((binder, ty));
        }
        (params, body)
    }

    #[test]
    fn a_parameterized_addition_selects_to_a_checked_sequence() {
        // (def (add (: a Int64) (: b Int64)) (+ a b)) — the body is a RUNTIME add over two params, and
        // the numeric model requires it to TRAP on overflow, so it selects to the CHECKED sequence.
        // Both operands are ALREADY in locals (params, slots 0,1), so they are read DIRECTLY — no copy
        // into `$a`/`$b` scratch (see `operand_src`). Only the result needs scratch: $r = slot 2.
        // get0 get1 add set$r; signed-overflow guard `((r^a)&(r^b))<0 → if unreachable` reading the
        // params' own slots; get$r.
        let ast = crate::testkit::parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "add");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(f.params, vec![ValType::I64, ValType::I64]);
        assert_eq!(
            f.code,
            vec![
                // r = a + b — operands read straight from the param slots, no set$a/set$b copies.
                Lir::LocalGet(0),
                Lir::LocalGet(1),
                Lir::I64Add,
                // `local.set 2 ; local.get 2` (store $r, then the guard's first read of $r) is fused by
                // the `peephole` pass into a single `local.tee 2`.
                Lir::LocalTee(2),
                // overflow guard: ((r^a) & (r^b)) < 0 → trap, reading a=slot0, b=slot1 directly.
                Lir::LocalGet(0),
                Lir::I64Xor,
                Lir::LocalGet(2),
                Lir::LocalGet(1),
                Lir::I64Xor,
                Lir::I64And,
                Lir::ConstI64(0),
                Lir::I64LtS,
                Lir::IfUnreachableEnd,
                // result
                Lir::LocalGet(2),
            ]
        );
        // One i64 scratch local declared ($r) — the operand copies ($a,$b) are eliminated.
        assert_eq!(f.declared, vec![ValType::I64; 1]);
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn a_constant_operand_is_inlined_not_stashed_in_scratch() {
        // (def (f (: a Int64)) (+ a 1)) — the RHS is a compile-time constant. `operand_src` returns a
        // `Const` source for it, so it is pushed inline (`i64.const 1`) at BOTH use sites (the add and
        // the guard's `r^b`) rather than stored into a `$b` scratch local and re-read. Only $r needs
        // scratch. Sequence: get$a const1 add set$r ; guard ((r^a)&(r^1))<0 ; get$r — the `set$r ;
        // get$r` pair fuses to `local.tee` via the peephole.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64)) (+ a 1)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![
                // r = a + 1 — `a` from its param slot, `1` inline (no $b scratch).
                Lir::LocalGet(0),
                Lir::ConstI64(1),
                Lir::I64Add,
                Lir::LocalTee(1), // set $r then the guard's first read of $r, fused.
                // guard: ((r ^ a) & (r ^ 1)) < 0 → trap. `1` re-materialized inline again.
                Lir::LocalGet(0),
                Lir::I64Xor,
                Lir::LocalGet(1),
                Lir::ConstI64(1),
                Lir::I64Xor,
                Lir::I64And,
                Lir::ConstI64(0),
                Lir::I64LtS,
                Lir::IfUnreachableEnd,
                Lir::LocalGet(1),
            ]
        );
        // Only $r (slot 1) is declared — the constant operand needs no scratch slot at all.
        assert_eq!(f.declared, vec![ValType::I64; 1]);
    }

    #[test]
    fn identical_operands_are_computed_once_via_cse() {
        // (def (f (: a Int64) (: b Int64)) (+ (* a b) (* a b))) — the two operands are the SAME product.
        // CSE computes `(* a b)` ONCE into a slot; the outer add reads that slot for BOTH operands. So
        // the body contains exactly ONE `i64.mul` (not two), and the add's operands are two reads of the
        // shared slot.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ (* a b) (* a b))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
            1,
            "the shared product is computed exactly once (CSE)"
        );
        // The add reads the product's slot twice as its two operands (a LocalGet of the same slot). Find
        // the mul's result slot (the LocalSet right after the sole I64Mul... or a LocalTee if the guard
        // fused) and confirm the I64Add is preceded by two reads of it.
        assert_eq!(
            f.code.iter().filter(|i| **i == Lir::I64Add).count(),
            1,
            "one add over the shared product"
        );
    }

    #[test]
    fn a_self_tail_recursive_function_compiles_to_a_loop() {
        // (def (f (: n Int64) (: acc Int64)) (if (= n 0) acc (f (- n 1) (+ acc 1)))) — the self-call is
        // in tail position (the `if`'s else branch), so `select_function_of` (given f's own def index)
        // compiles it as a LOOP: the body opens with `Lir::Loop`, the self-call updates the param slots
        // (`local.set`s) and `br`s back, and there is NO `ReturnCall`.
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64) (: acc Int64)) \
               (if (= n 0) acc (f (- n 1) (+ acc 1)))) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            matches!(f.code.first(), Some(Lir::Loop(_))),
            "a self-tail-recursive function body opens with a loop"
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Br(_))),
            "the self-tail-call branches back to the loop top"
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
            "no return_call — the self-call became a loop iteration"
        );
    }

    #[test]
    fn a_mutually_tail_recursive_pair_compiles_to_a_shared_loop() {
        // even/odd tail-call each other (same signature) — each compiles to ONE `loop` with a `which`
        // dispatch: the body opens with `Lir::Loop`, a cross-call sets `which` + `br`s back, and there
        // is NO `ReturnCall` (the mutual tail-call became a loop iteration, not a real call).
        let ast = crate::testkit::parse(
            "(module m (def (even (: n Int64)) (if (= n 0) true (odd (- n 1)))) \
               (def (odd (: n Int64)) (if (= n 0) false (even (- n 1)))) (export even))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("even").expect("def even");
        let (params, body) = function_of(&mut db, "even");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        // The loop is not the FIRST instruction (the `which` init precedes it), but it is present near
        // the top, the cross-calls `br`, and no `return_call` survives.
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
            "a mutually-tail-recursive member compiles to a loop, got: {:?}",
            f.code
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::Br(_))),
            "the mutual tail-call branches back to the loop top"
        );
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
            "no return_call — the mutual tail-call became a loop iteration, got: {:?}",
            f.code
        );
    }

    #[test]
    fn mutual_recursion_with_different_signatures_stays_return_call() {
        // `f(n)` tail-calls `g(n,k)` and vice-versa — DIFFERENT arities, so they can't share one set of
        // parameter slots. The shared-loop transform must decline (signature guard) and leave the mutual
        // tail-calls as `return_call` (still O(1) stack, just a real tail call, not a loop `br`).
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (if (= n 0) 1 (g (- n 1) 2))) \
               (def (g (: n Int64) (: k Int64)) (if (= n 0) k (f (- n 1)))) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
            "heterogeneous-signature mutual recursion is not merged into a loop, got: {:?}",
            f.code
        );
        assert!(
            f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
            "the cross-call to a different-signature peer stays a return_call"
        );
    }

    #[test]
    fn a_non_recursive_function_is_not_wrapped_in_a_loop() {
        // A plain `(+ a b)` — no self-call, so no loop wrapping even when the def index is supplied.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) (+ a b)) (export f))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let d = db.def_by_name("f").expect("def f");
        let (params, body) = function_of(&mut db, "f");
        let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
        assert!(
            !f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
            "a non-recursive function is not wrapped in a loop"
        );
    }

    #[test]
    fn peephole_fuses_set_then_get_of_the_same_local_into_tee() {
        // `local.set N ; local.get N` (store then immediately re-read the SAME local) → `local.tee N`.
        let mut code = vec![
            Lir::I64Add,
            Lir::LocalSet(2),
            Lir::LocalGet(2), // same local as the set → fuses
            Lir::LocalGet(0),
            Lir::I64Xor,
        ];
        peephole(&mut code);
        assert_eq!(
            code,
            vec![Lir::I64Add, Lir::LocalTee(2), Lir::LocalGet(0), Lir::I64Xor]
        );
    }

    #[test]
    fn peephole_leaves_a_set_get_of_different_locals_alone() {
        // A `local.get` of a DIFFERENT local must NOT fuse (it is a genuine read of another value), and
        // a `local.set` not immediately followed by a matching `local.get` is untouched.
        let mut code = vec![
            Lir::LocalSet(3),
            Lir::LocalGet(4), // different local → no fuse
            Lir::LocalSet(5),
            Lir::I64Add, // set not followed by a get → no fuse
        ];
        let before = code.clone();
        peephole(&mut code);
        assert_eq!(code, before);
    }

    #[test]
    fn peephole_does_not_fuse_across_a_block_boundary() {
        // A block marker (`End`) between the set and the get keeps them non-adjacent, so no fuse — a
        // `local.get` opening a different block never merges with a `local.set` closing another.
        let mut code = vec![Lir::LocalSet(2), Lir::End, Lir::LocalGet(2)];
        let before = code.clone();
        peephole(&mut code);
        assert_eq!(code, before);
    }

    #[test]
    fn a_parameterized_comparison_selects_to_a_signed_compare() {
        // (def (lt (: a Int64) (: b Int64)) (< a b)) — a runtime signed comparison, result Bool (i32).
        // A comparison never overflows, so no scratch/guard — just push both and compare.
        let ast = crate::testkit::parse(
            "(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "lt");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64LtS]
        );
        assert!(f.declared.is_empty());
        assert_eq!(f.ret, Ty::Bool);
    }

    #[test]
    fn a_nested_checked_op_shares_scratch_minimally() {
        // (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) — a nested checked op. The outer
        // mul's LHS is the inner add; instead of computing the add into its OWN $r and copying that to
        // the mul's $a, the add is emitted with `ResultDest::Slot($a)` so its result store writes $a
        // directly (no `local.get $r_inner ; local.tee $a` copy, no separate $r_inner slot). Slots:
        // outer mul $a=3 (the inner add writes here), $b=c=slot 2 (a direct param, no scratch), $r=4;
        // the inner add reuses $a=3 as its own $r and its a,b are direct params → no scratch of its own.
        // So only slots 3 and 4 are declared — 2 locals, down from 3 before the dest-threading.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(f.declared, vec![ValType::I64; 2]);
    }

    // ── value-heap H2d: Perceus — a kept heap binding constructs then DROPS ───────────────────────

    #[test]
    fn a_projection_only_tuple_folds_and_builds_no_heap() {
        // (def (f (: a Int64) (: b Int64)) (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) — `t` is ONLY
        // ever projected (never used as a whole value), so it does NOT need to exist on the heap: each
        // projection folds straight through to its element (the param), and the body is just `(+ a b)`.
        // No `arr-alloc`, no `box`/`arr-set`, no `drop` — a projection-only compound emits ZERO heap ops
        // (`should_keep_binding` does not keep a projection-only compound). The GENUINE heap-alloc →
        // escape → walk → drop (Perceus) path is exercised by the recursive-escape resource tests
        // (`a_recursive_runtime_tuple_escapes_to_the_host` + the `live-objects == 0` balance probe),
        // where the compound is returned WHOLE and must actually be built.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64)) \
               (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.contains(&Lir::CallImport("arr-alloc")),
            "a projection-only tuple must not be built on the heap"
        );
        assert!(
            !f.code.contains(&Lir::CallImport("drop")),
            "nothing is built, so nothing is dropped"
        );
        // It is exactly the checked add of the two params — the same code `(+ a b)` emits directly.
        assert!(
            f.code.contains(&Lir::I64Add) && f.code.contains(&Lir::LocalGet(0)),
            "the body folds to `(+ a b)` over the params"
        );
    }

    #[test]
    fn a_scalar_let_binding_is_not_dropped() {
        // A scalar (`Int64`) `let` binding owns no heap cell, so NO drop is emitted for it — reclamation
        // is only for heap values. `(let ((s (+ a b))) (+ s s))` — `s` is a kept i64, never dropped.
        let ast = crate::testkit::parse(
            "(module m (def (g (: a Int64) (: b Int64)) \
               (let ((s (+ a b))) (+ s s))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "g");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert!(
            !f.code.contains(&Lir::CallImport("drop")),
            "a scalar binding owns no heap cell and must not be dropped"
        );
    }
}
