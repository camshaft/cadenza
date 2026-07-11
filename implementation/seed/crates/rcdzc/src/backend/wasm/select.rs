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
//! Reserves Bounded Scratch Or Declines). Stage 0's slice — a constant, a two-way `if` — lowers to a
//! constant push and a structured `if`/`else`/`end`, so nothing declines here yet.

use crate::ast::StructId;
use crate::backend::wasm::lir::{BlockType, Lir, ValType, valtype_of};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::{IntTy, Ty};
use std::collections::HashMap;

/// A selected function body: its flat instruction sequence, the value types of its declared (non-
/// parameter) locals in slot order, its parameter value types, and its solved return type (for the
/// type section). Stage 0 bodies are nullary with no locals.
pub struct SelectedFunc {
    pub params: Vec<ValType>,
    pub ret: Ty,
    pub code: Vec<Lir>,
    pub declared: Vec<ValType>,
}

/// Select one NULLARY definition body (rooted at AST occurrence `body`) into its flat instruction
/// sequence. The return type is the body's solved type. Reads the core + type columns lazily.
pub fn select_body(db: &mut Db, body: StructId) -> Result<SelectedFunc, Reject> {
    select_function(db, body, &[])
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
) -> Result<SelectedFunc, Reject> {
    // Assign each parameter a local slot in order, and its wasm value type (its machine rep).
    let mut slot_of: HashMap<StructId, u32> = HashMap::new();
    let mut param_vts: Vec<ValType> = Vec::new();
    for (i, (binder, ty)) in params.iter().enumerate() {
        let vt = valtype_of(ty).ok_or_else(|| {
            Reject::decline("a function parameter's type has no machine representation")
        })?;
        slot_of.insert(*binder, i as u32);
        param_vts.push(vt);
    }
    let ret = type_of(db, body);
    let mut code = Vec::new();
    // Scratch locals start PAST the parameters (slots `0..n` are the params); the checked-arithmetic
    // guard claims scratch slots from `base` up. `high` tracks the highest scratch slot used, so we
    // declare exactly that many i64 scratch locals (`code_entry` run-length-encodes them).
    let base = param_vts.len() as u32;
    let mut high = base;
    emit(db, body, &slot_of, base, &mut high, &mut code)?;
    let scratch = (high - base) as usize;
    let declared = vec![ValType::I64; scratch];
    Ok(SelectedFunc {
        params: param_vts,
        ret,
        code,
        declared,
    })
}

/// Emit the flat instructions for the node at `id`, appending to `out`. `slots` maps a parameter's
/// name occurrence to its wasm local slot; `base` is the next free SCRATCH slot (the checked guard
/// claims `[base, base+1, base+2]` and recurses operands at `base+3`); `high` is the running high-water
/// mark of scratch slots used (so `select_function` declares exactly that many). Exhaustive over `Core`.
fn emit(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
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
            if !v.fits_width(it.ground_signed(), it.ground_width()) {
                return Err(Reject::coded(
                    Code::IntOutOfRange,
                    "integer literal does not fit its width",
                ));
            }
            if it.ground_width() <= 32 {
                out.push(Lir::ConstI32(v.to_i32_bits(it.ground_width())));
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
        // A record that SURVIVED folding to selection is used as a runtime value (not just to read a
        // field, which folds away). Constructing a compound at run time needs the value-heap runtime,
        // a later stage — so decline cleanly rather than emit a plausible-but-wrong sequence.
        Core::Record { .. } => Err(Reject::decline(
            "constructing a record at run time needs the value heap (not yet built)",
        )),
        Core::If { cond, then_, else_ } => {
            // Selection order matches wasm's structured `if`: push the condition, open the block with
            // the RESULT type (read off the node's solved type), then the two arms.
            emit(db, cond, slots, base, high, out)?;
            let block_ty = match type_of(db, id) {
                Ty::Unit => BlockType::Empty,
                other => match valtype_of(&other) {
                    Some(vt) => BlockType::Val(vt),
                    None => {
                        return Err(Reject::decline(
                            "if result type has no machine representation",
                        ));
                    }
                },
            };
            out.push(Lir::If(block_ty));
            emit(db, then_, slots, base, high, out)?;
            out.push(Lir::Else);
            emit(db, else_, slots, base, high, out)?;
            out.push(Lir::End);
            Ok(())
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
        // A runtime comparison: emit both operands, then the machine comparison selected from the
        // operands' width/signedness. The result is an i32 boolean. A comparison never overflows, so
        // both operands share the same scratch base (each is dead once pushed).
        Core::Compare { op, lhs, rhs } => {
            emit(db, lhs, slots, base, high, out)?;
            emit(db, rhs, slots, base, high, out)?;
            let it = operand_int_ty(db, lhs, rhs);
            out.push(compare_op(op, it));
            Ok(())
        }
        // A runtime arithmetic op. `+`/`-`/`*` are CHECKED — the numeric model requires the unqualified
        // operator to TRAP on overflow, not wrap (`numeric-model.md` §Overflow Is Defined; the trapping
        // default). A bare `i64.add` WRAPS (wasm two's-complement), which is a miscompile — so the op is
        // emitted as a guarded sequence over scratch locals (`emit_checked_arith`) that computes the
        // result, tests for signed overflow, and `unreachable`s if it overflowed. `/ % << >>` still
        // decline at runtime (they need signed/unsigned selection — the widths stage). Const arithmetic
        // folds in `lower` (with the SAME trap → CDZ0304); this is the genuine runtime-operand path.
        Core::Arith { op, lhs, rhs } => {
            let narrow = matches!(type_of(db, id), Ty::Int(it) if it.ground_width() <= 32);
            if narrow {
                // A ≤32-bit checked op needs the i32 guard variant — deferred to the widths stage.
                return Err(Reject::decline(
                    "a runtime narrow (≤32-bit) integer operation is not yet emitted",
                ));
            }
            match op {
                Prim::Add | Prim::Sub | Prim::Mul => {
                    emit_checked_arith(db, op, lhs, rhs, slots, base, high, out)
                }
                // Not yet emittable as a runtime op (needs signed/unsigned selection) — decline.
                _ => Err(Reject::decline(
                    "a runtime integer operation of this kind is not yet emitted",
                )),
            }
        }
        // A poison that reached selection is an unconditionally-reached fault; the poison collector
        // surfaces it before emission, so reaching here is a decline rather than emitted code.
        Core::Poison(reject) => Err(reject),
    }
}

/// Emit a CHECKED 64-bit `+`/`-`/`*` that TRAPS on signed overflow (the numeric-model default), over
/// three scratch locals `$a=base`, `$b=base+1`, `$r=base+2`. The sequence:
///
///   <A> local.set $a ; <B> local.set $b ; get$a get$b <op> local.set $r ; <overflow guard> ; get$r
///
/// The overflow guard leaves nothing on the stack unless it TRAPS (`if (empty) unreachable end`) — the
/// signed-overflow test differs by op (all VALIDATED against exact arithmetic in the seed compiler, mul
/// over 172k random cases):
///   - add `r=a+b`: overflow iff `((r^a) & (r^b)) < 0`
///   - sub `r=a-b`: overflow iff `((a^b) & (a^r)) < 0`
///   - mul `r=a*b`: overflow iff `a≠0 && r/a≠b` (the one case `r/a` can't detect — `MIN/-1` — is where
///     `i64.div_s` itself traps, which IS the overflow, so no extra check is needed)
///
/// LIVENESS / minimal locals: both operands recurse at `base+3` (NOT disjoint ranges) — operand A is
/// stored into `$a` before B's code runs, so A's scratch `[base+3..]` is DEAD during B and B safely
/// reuses it. The declared-locals count is therefore `max(A-scratch, B-scratch)+3`, not the sum — the
/// high-water mark in `high` captures exactly that. A's OWN result must not sit in `$a`/`$b`/`$r`
/// (those are this op's), so A/B start at `base+3`, above this op's three slots.
#[allow(clippy::too_many_arguments)]
fn emit_checked_arith(
    db: &mut Db,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    out: &mut Vec<Lir>,
) -> Result<(), Reject> {
    let (sa, sb, sr) = (base, base + 1, base + 2);
    // This op claims slots up to `sr` — record the high-water mark (declared locals reach at least here).
    if sr + 1 > *high {
        *high = sr + 1;
    }
    let operand_base = base + 3;
    // <A> local.set $a
    emit(db, lhs, slots, operand_base, high, out)?;
    out.push(Lir::LocalSet(sa));
    // <B> local.set $b  (B reuses A's dead scratch, from operand_base — minimal locals)
    emit(db, rhs, slots, operand_base, high, out)?;
    out.push(Lir::LocalSet(sb));
    // get$a get$b <op> local.set $r
    out.push(Lir::LocalGet(sa));
    out.push(Lir::LocalGet(sb));
    out.push(match op {
        Prim::Add => Lir::I64Add,
        Prim::Sub => Lir::I64Sub,
        Prim::Mul => Lir::I64Mul,
        _ => return Err(Reject::decline("not a checked arithmetic op")),
    });
    out.push(Lir::LocalSet(sr));
    // The signed-overflow guard — traps via `if (empty) unreachable end` when it detects overflow.
    emit_overflow_guard(op, sa, sb, sr, out);
    // The result.
    out.push(Lir::LocalGet(sr));
    Ok(())
}

/// The signed-overflow guard instruction run for `(op a b)` with result in `$r` — leaves nothing on
/// the stack except via a trap. See `emit_checked_arith` for the per-op test derivations.
fn emit_overflow_guard(op: Prim, sa: u32, sb: u32, sr: u32, out: &mut Vec<Lir>) {
    match op {
        // add: `((r^a) & (r^b)) < 0` → trap.
        Prim::Add => {
            out.push(Lir::LocalGet(sr));
            out.push(Lir::LocalGet(sa));
            out.push(Lir::I64Xor);
            out.push(Lir::LocalGet(sr));
            out.push(Lir::LocalGet(sb));
            out.push(Lir::I64Xor);
            out.push(Lir::I64And);
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64LtS);
            out.push(Lir::IfUnreachableEnd);
        }
        // sub: `((a^b) & (a^r)) < 0` → trap.
        Prim::Sub => {
            out.push(Lir::LocalGet(sa));
            out.push(Lir::LocalGet(sb));
            out.push(Lir::I64Xor);
            out.push(Lir::LocalGet(sa));
            out.push(Lir::LocalGet(sr));
            out.push(Lir::I64Xor);
            out.push(Lir::I64And);
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64LtS);
            out.push(Lir::IfUnreachableEnd);
        }
        // mul: `if a≠0 { if r/a ≠ b { unreachable } }` — nested if guards div against a=0 (a=0 can't
        // overflow); the inner `i64.div_s` traps on MIN/-1 itself, the sole case `r/a` can't detect.
        Prim::Mul => {
            out.push(Lir::LocalGet(sa));
            out.push(Lir::ConstI64(0));
            out.push(Lir::I64Ne);
            out.push(Lir::If(BlockType::Empty)); // if a != 0 {
            out.push(Lir::LocalGet(sr));
            out.push(Lir::LocalGet(sa));
            out.push(Lir::I64DivS);
            out.push(Lir::LocalGet(sb));
            out.push(Lir::I64Ne);
            out.push(Lir::IfUnreachableEnd); //   if (r/a) != b { unreachable }
            out.push(Lir::End); // }
        }
        _ => {}
    }
}

/// The flat wasm comparison op for a relational prim over an operand integer type — the width chooses
/// i32 (≤32-bit, or a boolean operand) vs i64, and the signedness chooses `_s` vs (later) `_u`.
/// Equality is sign- and (for the same width) representation-agnostic. Width-64 signed integers and
/// booleans are what this increment produces, so only signed + eq variants are needed now.
fn compare_op(op: Prim, it: IntTy) -> Lir {
    let narrow = it.ground_width() <= 32;
    match (op, narrow) {
        (Prim::Eq, false) => Lir::I64Eq,
        (Prim::Lt, false) => Lir::I64LtS,
        (Prim::Gt, false) => Lir::I64GtS,
        (Prim::Le, false) => Lir::I64LeS,
        (Prim::Ge, false) => Lir::I64GeS,
        (Prim::Eq, true) => Lir::I32Eq,
        (Prim::Lt, true) => Lir::I32LtS,
        (Prim::Gt, true) => Lir::I32GtS,
        (Prim::Le, true) => Lir::I32LeS,
        (Prim::Ge, true) => Lir::I32GeS,
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
    use crate::testkit::{if_program, scalar_program};

    #[test]
    fn selects_a_literal_to_i64_const() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        let f = select_body(&mut db, body).expect("select");
        assert_eq!(f.code, vec![Lir::ConstI64(42)]);
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn selects_an_if_to_a_structured_block() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        let f = select_body(&mut db, if_node).expect("select");
        // (if false 1 2) → i32.const 0 ; if (result i64) ; i64.const 1 ; else ; i64.const 2 ; end
        assert_eq!(
            f.code,
            vec![
                Lir::ConstI32(0),
                Lir::If(BlockType::Val(ValType::I64)),
                Lir::ConstI64(1),
                Lir::Else,
                Lir::ConstI64(2),
                Lir::End,
            ]
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
        // the numeric model requires it to TRAP on overflow, so it selects to the CHECKED sequence:
        // params in slots 0,1; scratch $a=2 $b=3 $r=4. A→set$a; B→set$b; get$a get$b add set$r;
        // signed-overflow guard `((r^a)&(r^b))<0 → if unreachable`; get$r.
        let ast = crate::testkit::parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (params, body) = function_of(&mut db, "add");
        let f = select_function(&mut db, body, &params).expect("select");
        assert_eq!(f.params, vec![ValType::I64, ValType::I64]);
        assert_eq!(
            f.code,
            vec![
                // A → $a, B → $b
                Lir::LocalGet(0),
                Lir::LocalSet(2),
                Lir::LocalGet(1),
                Lir::LocalSet(3),
                // r = a + b
                Lir::LocalGet(2),
                Lir::LocalGet(3),
                Lir::I64Add,
                Lir::LocalSet(4),
                // overflow guard: ((r^a) & (r^b)) < 0 → trap
                Lir::LocalGet(4),
                Lir::LocalGet(2),
                Lir::I64Xor,
                Lir::LocalGet(4),
                Lir::LocalGet(3),
                Lir::I64Xor,
                Lir::I64And,
                Lir::ConstI64(0),
                Lir::I64LtS,
                Lir::IfUnreachableEnd,
                // result
                Lir::LocalGet(4),
            ]
        );
        // Three i64 scratch locals declared ($a $b $r).
        assert_eq!(f.declared, vec![ValType::I64; 3]);
        assert!(f.ret.agrees_with(&Ty::int64()));
    }

    #[test]
    fn a_parameterized_comparison_selects_to_a_signed_compare() {
        // (def (lt (: a Int64) (: b Int64)) (< a b)) — a runtime signed comparison, result Bool (i32).
        // A comparison never overflows, so no scratch/guard — just push both and compare.
        let ast = crate::testkit::parse(
            "(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (params, body) = function_of(&mut db, "lt");
        let f = select_function(&mut db, body, &params).expect("select");
        assert_eq!(
            f.code,
            vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64LtS]
        );
        assert!(f.declared.is_empty());
        assert_eq!(f.ret, Ty::Bool);
    }

    #[test]
    fn a_nested_checked_op_shares_scratch_minimally() {
        // (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) — a nested checked op. The inner
        // `(+ a b)` fully drains its scratch (leaves only its result on the stack → stored to the
        // outer's $a) before the outer `*` uses its slots, so the two share the scratch pool: the
        // declared-locals count is the MAX depth, not the sum. Inner add at operand_base=6 uses 6,7,8;
        // outer mul uses 3,4,5 — high-water 9, so 6 scratch locals (base 3 → 9), NOT 3+3+3.
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let (params, body) = function_of(&mut db, "f");
        let f = select_function(&mut db, body, &params).expect("select");
        // 3 params (slots 0,1,2); scratch base 3. Outer mul: $a3 $b4 $r5, operands at 6. Inner add:
        // $a6 $b7 $r8, operands (params) need no scratch. High-water 9 → 6 scratch locals.
        assert_eq!(f.declared, vec![ValType::I64; 6]);
    }
}
