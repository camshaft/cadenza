//! `lower` — the query that fills the core column: for a node's `StructId`, its A-normal [`Core`]
//! form.
//!
//! One concern: lowering the resolved tree to the A-normal core. [`core_of`] reads a node's resolved
//! form (via [`crate::resolve::resolved_of`]) and produces its core form, memoizing into `db.core`;
//! it is the ONLY module that fills that column. Where a lowering decision needs the solved type it
//! READS it (via [`crate::infer::type_of`]) rather than recomputing one
//! (`reference-compiler.md` §One Pass Owns One Concern) — Stage 0's structural lowering does not yet
//! branch on the type, but the read is available so the read-off discipline holds as constructs grow.
//!
//! For the Stage-0 slice every leaf is already an atom, so lowering is close to a structural map: a
//! literal → a `Const*`, an `if` → a core `If` referencing the same child ids (lowered on their own
//! demand). No administrative binding is introduced yet (nothing non-atomic to name); those, and the
//! core's own fresh-id space, arrive with the stage that first needs them.

use crate::arena::Slot;
use crate::ast::{IntValue, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::resolve::resolved_of;
use crate::resolved::{Prim, Resolved};

/// The core (A-normal) form of the node at `id`, filling the column on demand (memoized). Reads the
/// resolved form; children stay ids, lowered on their own demand.
pub fn core_of(db: &mut Db, id: StructId) -> Core {
    if let Slot::Filled(c) = db.core.get(id) {
        return c.clone();
    }
    let c = compute(db, id);
    db.core.fill(id, c.clone());
    c
}

/// Lower one node's resolved form to its core form. Records fold: a bare name is its bound value's
/// core, a `let` is its body's core, and a member projection is the FIELD'S core read directly — so a
/// record used only to read a field leaves no runtime trace (it folds to the projected scalar). A
/// record used as a runtime value survives as `Core::Record` (which declines at select until the
/// value heap exists). This is the one compile-time reduction tier acting through lowering
/// (`reference-compiler.md` §A Construct Whose Value Is Fully Determined At Compile Time).
fn compute(db: &mut Db, id: StructId) -> Core {
    match resolved_of(db, id) {
        Resolved::Int(v) => Core::ConstInt(v),
        Resolved::Bool(b) => Core::ConstBool(b),
        Resolved::Unit => Core::Unit,
        // A name is its bound value's core — follow the ref (the `let` binding folds away).
        Resolved::Ref { value } => core_of(db, value),
        // A `let`'s value is its body's value (the bindings are compile-time structure).
        Resolved::Let { body, .. } => core_of(db, body),
        // A record value — kept as a compound; folds away only when a member reads a field of it.
        Resolved::Record { fields } => Core::Record { fields },
        // Member access FOLDS: reduce the operand to a record (following refs, reducing a ctor
        // application) and lower the field's value directly, so `(. (record (x 1)) x)` and `(. (Int
        // 64) max)` both fold to the field's value with no record built. The one projection, via the
        // evaluator. A non-record operand or an absent field is a poison so a mis-projection never
        // emits a wrong value.
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(value) => core_of(db, value),
            crate::eval::Member::NoField => Core::Poison(Reject::coded(
                Code::Malformed,
                format!("record has no field `{}`", key.name),
            )),
            // Not a record. If the operand is itself a poison (e.g. a dup-field record), propagate
            // THAT root cause rather than the generic "requires a record".
            crate::eval::Member::NotRecord => match core_of(db, operand) {
                Core::Poison(r) => Core::Poison(r),
                _ => Core::Poison(Reject::coded(Code::Malformed, "member access requires a record")),
            },
        },
        Resolved::If { cond, then_, else_ } => Core::If { cond, then_, else_ },
        // A bare built-in operation value that is not applied has no runtime form yet (no closures) —
        // it declines. Applying it is what lowers.
        Resolved::Prim(_) => Core::Poison(Reject::decline(
            "a built-in operation used as a value needs runtime closures (not yet built)",
        )),
        // Application — the ONE path, dispatched by the head value's `(meta apply)` primitive. An
        // arithmetic prim folds (below); a type-constructor prim reduces via the evaluator to a built
        // value (a module / type-value), which is then lowered — a member projection off it folds, a
        // bare type/module used at runtime declines at the erasure fence.
        Resolved::Apply { head, args } => {
            // A LAMBDA head β-reduces (substitute args for params) and the reduced body lowers — this
            // is how a user function call folds/monomorphizes: `((fn (x) (+ x 1)) 5)` reduces to
            // `(+ 5 1)` → `6`, with no function value emitted. The reduction runs UNDER a guard keyed
            // by the lambda's body, so a recursive call (which re-enters the same body while lowering
            // the reduced result) is detected and DECLINES rather than inlining without end.
            if crate::eval::lambda_body(db, head).is_some() {
                // Reduce and lower under a depth guard: a terminating fold bottoms out; a recursive
                // function inlines past the bound and DECLINES rather than diverging.
                match db.enter_reduction() {
                    Some(mut guard) => {
                        let g = guard.db();
                        return match crate::eval::apply_lambda(g, head, &args) {
                            Ok(Some(reduced)) => core_of(g, reduced),
                            Ok(None) => unreachable!("lambda_body implies a lambda head"),
                            Err(msg) => Core::Poison(Reject::decline(msg)),
                        };
                    }
                    None => {
                        return Core::Poison(Reject::decline(
                            "a recursive function needs runtime specialization (not yet built)",
                        ));
                    }
                }
            }
            match crate::eval::meta_apply_of(db, head) {
                Some(prim) if prim.is_arith() => lower_arith(db, prim, &args),
                Some(prim) if prim.is_comparison() => lower_comparison(db, prim, &args),
                Some(prim) => match crate::eval::reduce_ctor(db, prim, &args) {
                    Ok(built) => core_of(db, built),
                    Err(msg) => Core::Poison(Reject::decline(msg)),
                },
                // Not applyable. If the head itself is a poison (e.g. an unbound name), propagate THAT
                // root cause — an unbound head is a scope error, not merely "not applyable".
                None => match core_of(db, head) {
                    Core::Poison(r) => Core::Poison(r),
                    _ => Core::Poison(Reject::decline("value is not applyable")),
                },
            }
        }
        Resolved::Poison(r) => Core::Poison(r),
        // A lambda parameter used as a value has no runtime core yet (runtime closures deferred).
        Resolved::Param { .. } => Core::Poison(Reject::decline(
            "a lambda parameter has no runtime form yet (runtime closures not built)",
        )),
        // A type value or a compile-time lambda is compile-time-only — no runtime core form (the
        // erasure fence forbids one reaching runtime), so lowering it as a runtime value declines.
        Resolved::TypeVal(_) | Resolved::Lambda { .. } => Core::Poison(Reject::decline(
            "a type value or compile-time lambda has no runtime form",
        )),
    }
}

/// Lower an ARITHMETIC application: FOLD it when its operands fold to constants — evaluate at compile
/// time with a CHECKED operation, so a provable overflow is a build error (CDZ0304 poison) rather than
/// a shipped runtime trap (`reference-compiler.md` §A Compile-Provable Trap Fails The Build). An
/// operand that is not a constant stays a runtime `Arith` (its wasm op selected from the solved width
/// at selection); a poison operand propagates.
fn lower_arith(db: &mut Db, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            format!("{} takes exactly 2 operands", intrinsic_name(op)),
        ));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => fold_arith(op, a, b),
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        _ => Core::Arith { op, lhs: args[0], rhs: args[1] },
    }
}

/// Fold a constant arithmetic operation with a CHECKED evaluation. Both operands are compile-time
/// constants; if the operation's defined outcome on them is a trap (an overflow the checked default
/// forbids, or an operand outside the machine range the fold evaluates over), the result is a poison
/// carrying CDZ0304 — the build fails rather than shipping a runtime trap. On success the result is a
/// `ConstInt`. The evaluation is over `i64` (the Stage default integer); a later width stage
/// generalizes the range the check tests to the operands' solved width.
fn fold_arith(op: Prim, a: IntValue, b: IntValue) -> Core {
    let (x, y) = match (a.to_i64(), b.to_i64()) {
        (Some(x), Some(y)) => (x, y),
        // An operand beyond the machine range the fold evaluates over — a provable width trap.
        _ => {
            return Core::Poison(Reject::coded(
                Code::ConstTrap,
                "constant operand does not fit the integer width",
            ));
        }
    };
    // Each integer op evaluates over `i64` (the Stage default width) with the DEFINED numeric-model
    // semantics; `None` marks a provable trap the checked default forbids (`numeric-model.md` §Overflow
    // Is Defined). A later width stage generalizes the range/count the checks test to the solved width.
    let checked = match op {
        Prim::Add => x.checked_add(y),
        Prim::Sub => x.checked_sub(y),
        Prim::Mul => x.checked_mul(y),
        // Division truncates toward zero; traps on a zero divisor and on `MIN / -1` (Rust's
        // `checked_div` returns `None` for both — exactly the two defined traps).
        Prim::Div => x.checked_div(y),
        // Remainder takes the dividend's sign; traps on a zero divisor. `MIN % -1` is 0 (no overflow),
        // but Rust's `%` panics there — `checked_rem` returns `None`, so special-case it to 0.
        Prim::Rem => {
            if y == -1 {
                Some(0)
            } else {
                x.checked_rem(y)
            }
        }
        // A left shift is exact multiplication by `2^count`: it traps on an out-of-range count
        // (< 0 or ≥ width) AND on overflow past the width — NOT wasm's silent mask-and-wrap.
        Prim::Shl => checked_shl_i64(x, y),
        // Arithmetic (sign-extending) right shift; traps on an out-of-range count, never overflows.
        Prim::Shr => checked_shr_i64(x, y),
        // Bitwise operations are total on the two's-complement value — never trap.
        Prim::BitAnd => Some(x & y),
        Prim::BitOr => Some(x | y),
        Prim::BitXor => Some(x ^ y),
        // A non-integer-binary prim never reaches the fold (`lower_arith` is only called for an
        // `is_arith` prim), so these arms are unreachable in practice; decline rather than panic.
        Prim::Lt
        | Prim::Gt
        | Prim::Le
        | Prim::Ge
        | Prim::Eq
        | Prim::IntCtor
        | Prim::UIntCtor
        | Prim::FnCtor
        | Prim::BoolTy
        | Prim::UnitTy => {
            return Core::Poison(Reject::decline("not an integer binary operation"));
        }
    };
    match checked {
        Some(n) => Core::ConstInt(IntValue::from_i64(n)),
        // A provable trap — the checked default traps, and the compiler can prove it, so the build
        // fails (CDZ0304) rather than emitting a component that traps (`numeric-model.md` §A Constant
        // Operation With No Value Is Rejected At Compile Time).
        None => Core::Poison(Reject::coded(
            Code::ConstTrap,
            format!("constant {} has no defined value (overflow, divide-by-zero, or out-of-range shift)", intrinsic_name(op)),
        )),
    }
}

/// A left shift as EXACT multiplication by `2^count`: `None` (a provable trap) if the count is outside
/// `0..64` or the exact result overflows `i64` — a left shift is not exempt from Overflow Is Defined,
/// so it traps like `*` rather than masking the count and wrapping (`numeric-model.md`).
fn checked_shl_i64(x: i64, count: i64) -> Option<i64> {
    if !(0..64).contains(&count) {
        return None;
    }
    // Multiply by 2^count with overflow checking — the defined meaning of a left shift.
    x.checked_mul(1i64.checked_shl(count as u32)?)
}

/// An ARITHMETIC (sign-extending) right shift: `None` if the count is outside `0..64` (an out-of-range
/// count traps rather than masking). Never overflows. The signed shift preserves the sign bit, so
/// shifting a negative value right fills with ones (e.g. `-256 >> 7 = -2`).
fn checked_shr_i64(x: i64, count: i64) -> Option<i64> {
    if !(0..64).contains(&count) {
        return None;
    }
    Some(x >> count)
}

/// Lower a COMPARISON application (`< > <= >= =`). Folds two constant SCALARS (integers or booleans) to
/// a `ConstBool` — a total ordering on the scalar's value. A compound or runtime operand DECLINES (I1
/// is fold-only; structural comparison over the value heap, and runtime scalar comparison, arrive
/// later) — the operator's type stays fully generic (`∀a. a → a → Bool`), only the fold's coverage is
/// bounded. A poison operand propagates.
fn lower_comparison(db: &mut Db, op: Prim, args: &[StructId]) -> Core {
    if args.len() != 2 {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            format!("{} takes exactly 2 operands", intrinsic_name(op)),
        ));
    }
    let lhs = core_of(db, args[0]);
    let rhs = core_of(db, args[1]);
    match (lhs, rhs) {
        (Core::ConstInt(a), Core::ConstInt(b)) => match (a.to_i64(), b.to_i64()) {
            (Some(x), Some(y)) => Core::ConstBool(compare_ord(op, x.cmp(&y))),
            // An operand beyond the fold's machine range — decline (a wider fold arrives with widths).
            _ => Core::Poison(Reject::decline(
                "comparison of an integer beyond the machine width is not yet folded",
            )),
        },
        (Core::ConstBool(a), Core::ConstBool(b)) => Core::ConstBool(compare_ord(op, a.cmp(&b))),
        (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
        // A compound (record/heap) or runtime operand — structural / runtime comparison is a later
        // stage; decline cleanly rather than emit a wrong or trapping comparison.
        _ => Core::Poison(Reject::decline(
            "comparison of a compound or runtime value is not yet supported",
        )),
    }
}

/// Reduce an `Ordering` to the boolean the comparison `op` asks of it — the one place the relational
/// prims map to their meaning, shared by every scalar the fold compares (integers and booleans agree
/// on the ordering; only the comparison of the ordering differs). Equality is `Ordering::Equal`.
fn compare_ord(op: Prim, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        Prim::Lt => ord == Less,
        Prim::Gt => ord == Greater,
        Prim::Le => ord != Greater,
        Prim::Ge => ord != Less,
        Prim::Eq => ord == Equal,
        _ => false, // not a comparison — unreachable (only called for `is_comparison` prims).
    }
}

/// The source spelling of an intrinsic, for diagnostics.
fn intrinsic_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "+",
        Prim::Sub => "-",
        Prim::Mul => "*",
        Prim::Div => "/",
        Prim::Rem => "%",
        Prim::Shl => "<<",
        Prim::Shr => ">>",
        Prim::BitAnd => "&",
        Prim::BitOr => "|",
        Prim::BitXor => "^",
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "=",
        Prim::IntCtor => "Int",
        Prim::UIntCtor => "UInt",
        Prim::FnCtor => "->",
        Prim::BoolTy => "Bool",
        Prim::UnitTy => "Unit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::IntValue;
    use crate::testkit::{if_program, scalar_program};

    #[test]
    fn lowers_a_literal_to_a_const() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        assert_eq!(core_of(&mut db, body), Core::ConstInt(IntValue::from_i64(42)));
    }

    #[test]
    fn lowers_an_if_referencing_its_child_ids() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        match core_of(&mut db, if_node) {
            Core::If { cond, then_, else_ } => {
                assert_eq!(core_of(&mut db, cond), Core::ConstBool(false));
                assert_eq!(core_of(&mut db, then_), Core::ConstInt(IntValue::from_i64(1)));
                assert_eq!(core_of(&mut db, else_), Core::ConstInt(IntValue::from_i64(2)));
            }
            other => panic!("expected If, got {other:?}"),
        }
    }
}
