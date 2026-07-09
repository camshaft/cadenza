//! `lower : typed-Hir → Mir`, then `eval : MirModule → MirModule` (the compile-time evaluator).
//!
//! Mirrors `cdzc/30-lower.cdz`. Lowering reads the SOLVED type on each typed node (never re-derives
//! one) and carries the types `select` needs — a `let`'s bound-value type, an `if`'s result type, and
//! each function's param/return types. It is a shape-preserving map.
//!
//! `eval` is the ONE compile-time tier (metaprogramming.md §Compile-Time Evaluation Is One Tier):
//! constant folding today, and — the same reduction — generic monomorphization / macro expansion
//! later. It runs at MODULE scope (after every function is lowered) so a `Call` to a constant-argument
//! non-recursive callee can β-reduce against the callee's body. See [`crate::fold`].

use crate::ir::{Intrinsic, Mir, MirFunc, MirModule, Typed, TypedFunc, TypedModule, TypedNode};
use crate::ty::Ty;

/// Lower a typed module to `Mir`, then run the compile-time evaluator over the whole module.
pub fn lower_module(module: TypedModule) -> MirModule {
    let funcs = module.funcs.into_iter().map(lower_func).collect();
    let lowered = MirModule {
        funcs,
        exports: module.exports,
    };
    crate::fold::eval_module(lowered)
}

/// Lower one function body (no folding here — folding is a module-scope pass so it can β-reduce calls).
fn lower_func(f: TypedFunc) -> MirFunc {
    MirFunc {
        params: f.params,
        ret: f.ret,
        body: lower(f.body),
    }
}

/// Lower a typed-Hir tree to `Mir`, threading the solved types forward.
pub fn lower(typed: Typed) -> Mir {
    match typed.node {
        TypedNode::Int(n) => Mir::Int(n),
        TypedNode::Bool(b) => Mir::Bool(b),
        TypedNode::Str(s) => Mir::Str(s),
        TypedNode::Unit => Mir::Unit,
        TypedNode::Local(id) => Mir::Local(id),
        TypedNode::Call { func, args } => Mir::Call {
            func,
            args: args.into_iter().map(lower).collect(),
        },
        TypedNode::FuncRef(func) => Mir::FuncRef(func),
        TypedNode::Intrinsic(op) => Mir::Intrinsic(op),
        // A bare constructor. A NULLARY variant is the nullary sum VALUE (`NNil` ≡ `(NNil unit)`) —
        // infer typed it as the `Sum`, so lower it to the built `Mir::Sum` (disc, unit payload) directly.
        // A UNARY ctor bare is a transient function value (only survives if applied via `Apply` below, or
        // reduced by the fold); `select` declines a residual unary ctor value.
        TypedNode::Ctor { def, index } => {
            if def.variants()[index].payload.is_none() {
                Mir::Sum {
                    def,
                    disc: index as u32,
                    payload_ty: Ty::Unit,
                    payload: Box::new(Mir::Unit),
                }
            } else {
                Mir::Ctor { def, index }
            }
        }
        // Applying a CONSTRUCTOR builds a heap sum `(disc, payload)` directly — the payload's solved
        // type comes from the single argument (Unit for a nullary variant applied to `unit`). Any other
        // `Apply` (a function value / intrinsic) stays a `Mir::Apply` for the fold.
        TypedNode::Apply { func, args } => match func.node {
            TypedNode::Ctor { def, index } => {
                let payload = args.into_iter().next();
                let (payload_ty, payload_mir) = match payload {
                    Some(a) => (a.ty.clone(), lower(a)),
                    // A ctor applied to zero args is a type error at infer; if one slips through, use a
                    // unit payload so lowering stays total.
                    None => (Ty::Unit, Mir::Unit),
                };
                Mir::Sum {
                    def,
                    disc: index as u32,
                    payload_ty,
                    payload: Box::new(payload_mir),
                }
            }
            // A `Heap` intrinsic BOXES a scalar arg (a list element / map key+value / set element) →
            // `Mir::HeapOp` carrying each arg's SOLVED type, so `select` boxes by the right accessor (a
            // runtime `Local` arg's box need is not recoverable from the `Mir` shape alone). The func is
            // a direct `Intrinsic` node now that member access resolves a prelude-module field straight to
            // its `Hir`. Every other intrinsic stays a `Mir::Apply` for the fold.
            TypedNode::Intrinsic(Intrinsic::Heap(op)) => Mir::HeapOp {
                op,
                args: args.into_iter().map(|a| (a.ty.clone(), lower(a))).collect(),
            },
            _ => Mir::Apply {
                func: Box::new(lower(*func)),
                args: args.into_iter().map(lower).collect(),
            },
        },
        TypedNode::Wildcard => Mir::Wildcard,
        TypedNode::Match { scrutinee, arms } => {
            let scrut_ty = scrutinee.ty.clone();
            Mir::Match {
                scrutinee: Box::new(lower(*scrutinee)),
                scrut_ty,
                // A pattern is a `Typed` tree — lower it exactly like an expression (`Apply(Ctor)` →
                // `Mir::Sum{disc, payload}`, `Tuple` → `Mir::Tuple`, `Local`/`Wildcard`/literals as is),
                // so `select` reads a uniform lowered pattern tree.
                arms: arms.into_iter().map(|(p, b)| (lower(p), lower(b))).collect(),
                ty: typed.ty,
            }
        }
        TypedNode::Trap(msg) => Mir::Trap(msg),
        TypedNode::Tuple(elems) => {
            // Carry each element's solved type so `select` boxes it by the right accessor.
            Mir::Tuple(
                elems
                    .into_iter()
                    .map(|e| (e.ty.clone(), lower(e)))
                    .collect(),
            )
        }
        TypedNode::Record(fields) => {
            // A record lowers to a heap PRODUCT (a `Mir::Tuple`), its fields SORTED by name — the
            // canonical value-heap slot order every consumer (ctor/proj/render) agrees on. The named
            // structure is gone by this rung; the sort is the whole of what "record" means at runtime.
            let mut sorted = fields;
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            Mir::Tuple(
                sorted
                    .into_iter()
                    .map(|(_, e)| (e.ty.clone(), lower(e)))
                    .collect(),
            )
        }
        TypedNode::List(elems) => {
            // A list lowers to `Mir::List`, carrying each element's solved type (all the same — the
            // list's element type) so `select` boxes each by the right accessor before `vec-push`.
            Mir::List(
                elems
                    .into_iter()
                    .map(|e| (e.ty.clone(), lower(e)))
                    .collect(),
            )
        }
        TypedNode::Map(entries) => {
            // A `(map …)` lowers to `Mir::Map`, carrying each key's + value's solved type so `select`
            // boxes each scalar by the right accessor before `map-insert`.
            Mir::Map(
                entries
                    .into_iter()
                    .map(|(k, v)| ((k.ty.clone(), lower(k)), (v.ty.clone(), lower(v))))
                    .collect(),
            )
        }
        TypedNode::Set(elems) => {
            // A `(set …)` lowers to `Mir::Set`, carrying each element's solved type for boxing before
            // `set-insert`.
            Mir::Set(
                elems
                    .into_iter()
                    .map(|e| (e.ty.clone(), lower(e)))
                    .collect(),
            )
        }
        TypedNode::TupleProj(index, operand) => Mir::Proj {
            slot: index,               // positional: the literal index is the slot
            elem_ty: typed.ty.clone(), // the projected element's solved type (this node's type)
            operand: Box::new(lower(*operand)),
        },
        TypedNode::RecordProj(field, operand) => {
            // Resolve the field NAME to its SLOT — the field's position in the operand's sorted record
            // type (the canonical slot order). Inference already proved the field is present.
            let slot = match &operand.ty {
                crate::ty::Ty::Record(fields) => fields
                    .iter()
                    .position(|(n, _)| *n == field)
                    .expect("field present (checked by infer)"),
                // Inference guarantees the operand is a record carrying `field`; any other shape is a
                // compiler bug. Fall back to slot 0 only to keep lowering total (unreachable in practice).
                _ => 0,
            };
            Mir::Proj {
                slot,
                elem_ty: typed.ty.clone(), // the projected field's solved type
                operand: Box::new(lower(*operand)),
            }
        }
        TypedNode::Arith(op, a, b) => Mir::Arith(op, Box::new(lower(*a)), Box::new(lower(*b))),
        TypedNode::Bit(op, a, b) => Mir::Bit(op, Box::new(lower(*a)), Box::new(lower(*b))),
        TypedNode::Shift(op, a, b) => Mir::Shift(op, Box::new(lower(*a)), Box::new(lower(*b))),
        TypedNode::Cmp(op, a, b) => {
            // Carry the operand type (both operands share it — inference unified them) so selection
            // can pick the i64 (Int) vs i32 (Bool) machine comparison.
            let operand_ty = a.ty.clone();
            Mir::Cmp {
                op,
                operand_ty,
                a: Box::new(lower(*a)),
                b: Box::new(lower(*b)),
            }
        }
        TypedNode::If(c, t, e) => Mir::If {
            cond: Box::new(lower(*c)),
            then_: Box::new(lower(*t)),
            else_: Box::new(lower(*e)),
            ty: typed.ty,
        },
        TypedNode::Let { id, value, body } => Mir::Let {
            id,
            value_ty: value.ty.clone(),
            value: Box::new(lower(*value)),
            body: Box::new(lower(*body)),
        },
        // Lambda — `(fn (p…) body)` lowers shape-preserving. The node's `ty` is `Ty::Fn(params, ret)`.
        // A transient compile-time value: the fold β-reduces `Apply(Lambda, args)`; a survivor declines.
        TypedNode::Lambda { params, body } => Mir::Lambda {
            params,
            body: Box::new(lower(*body)),
        },
        // TypeVal — a type-value (compile-time-only). Lowered to `Mir::TypeVal` so the post-fold
        // erasure fence can point at it when rejecting a leak (CDZ0305). It should never survive fold.
        TypedNode::TypeVal(ty) => Mir::TypeVal(ty),
        // Annot — `(: e T)` lowers to just `e` (transparent — the constraint already happened at infer).
        TypedNode::Annot(e, _t) => lower(*e),
        // Const — `(const e)` lowers to `fold(e)` (the fold + fence will check it reduced fully).
        TypedNode::Const(e) => lower(*e),
    }
}
