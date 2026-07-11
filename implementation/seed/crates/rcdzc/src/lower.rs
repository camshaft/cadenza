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
        // Member access FOLDS: project the field's value occurrence from the operand's record and
        // lower THAT directly, so `(. (record (x 1)) x)` is the constant `1` with no record built. A
        // non-record operand or an absent field is a fault the type-error query reports; here it
        // becomes a poison so a mis-projection never emits a wrong value.
        Resolved::Member { operand, key } => match core_of(db, operand) {
            Core::Record { fields } => match fields.get(&key) {
                Some(&value) => core_of(db, value),
                None => Core::Poison(Reject::coded(
                    Code::Malformed,
                    format!("record has no field `{}`", key.name),
                )),
            },
            Core::Poison(r) => Core::Poison(r),
            _ => Core::Poison(Reject::coded(
                Code::Malformed,
                "member access requires a record",
            )),
        },
        Resolved::If { cond, then_, else_ } => Core::If { cond, then_, else_ },
        // A bare built-in operation value that is not applied has no runtime form yet (no closures) —
        // it declines. Applying it is what lowers.
        Resolved::Prim(_) => Core::Poison(Reject::decline(
            "a built-in operation used as a value needs runtime closures (not yet built)",
        )),
        // An arithmetic application. FOLD it when its operands fold to constants: evaluate at compile
        // time with a CHECKED operation, so a provable overflow is a build error (CDZ0304 poison)
        // rather than a shipped runtime trap (`reference-compiler.md` §A Compile-Provable Trap Fails
        // The Build). An operand that is not a constant stays a runtime `Arith` (which, in this
        // increment, has no runtime integer source yet without functions — so `select` emits the op
        // for a genuine runtime operand once functions land; a poison operand propagates).
        Resolved::Apply { head, args } => {
            let op = match crate::resolve::intrinsic_of(db, head) {
                Some(op) => op,
                None => return Core::Poison(Reject::decline("application of a non-operation not yet supported")),
            };
            if args.len() != 2 {
                return Core::Poison(Reject::coded(
                    Code::Malformed,
                    format!("{} takes exactly 2 operands", intrinsic_name(op)),
                ));
            }
            let lhs = core_of(db, args[0]);
            let rhs = core_of(db, args[1]);
            match (lhs, rhs) {
                // Both constant → fold with a checked operation.
                (Core::ConstInt(a), Core::ConstInt(b)) => fold_arith(op, a, b),
                // A poison operand propagates (its own fault is already recorded).
                (Core::Poison(r), _) | (_, Core::Poison(r)) => Core::Poison(r),
                // A non-constant operand: a runtime arithmetic op over the two argument occurrences.
                _ => Core::Arith { op, lhs: args[0], rhs: args[1] },
            }
        }
        Resolved::Poison(r) => Core::Poison(r),
        // TODO: type constructors handled by the evaluator — a type value or a compile-time lambda has
        // no runtime core form yet (they reduce at compile time), so lowering them declines for now.
        Resolved::TypeVal(_) | Resolved::Lambda { .. } => Core::Poison(Reject::decline(
            "a type value or compile-time lambda has no runtime form yet",
        )),
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
    let checked = match op {
        Prim::Add => x.checked_add(y),
        Prim::Sub => x.checked_sub(y),
        Prim::Mul => x.checked_mul(y),
        // TODO: type constructors handled by the evaluator — only arith prims reach the fold today.
        Prim::IntCtor | Prim::UIntCtor | Prim::FnCtor => {
            return Core::Poison(Reject::decline("type constructor is not an arithmetic operation"));
        }
    };
    match checked {
        Some(n) => Core::ConstInt(IntValue::from_i64(n)),
        // A provable overflow — the checked default traps, and the compiler can prove it, so the build
        // fails (CDZ0304) rather than emitting a component that traps (`numeric-model.md` §A Constant
        // Operation With No Value Is Rejected At Compile Time).
        None => Core::Poison(Reject::coded(
            Code::ConstTrap,
            format!("integer overflow in constant {}", intrinsic_name(op)),
        )),
    }
}

/// The source spelling of an intrinsic, for diagnostics.
fn intrinsic_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "+",
        Prim::Sub => "-",
        Prim::Mul => "*",
        // TODO: type constructors handled by the evaluator — only arith prims reach here today.
        Prim::IntCtor => "Int",
        Prim::UIntCtor => "UInt",
        Prim::FnCtor => "->",
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
