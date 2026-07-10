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
use crate::ast::StructId;
use crate::core::Core;
use crate::db::Db;
use crate::resolve::resolved_of;
use crate::resolved::Resolved;

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

/// Lower one node's resolved form to its core form. Stage 0's slice is atomic, so this is a
/// structural map; children remain AST ids.
fn compute(db: &mut Db, id: StructId) -> Core {
    match resolved_of(db, id) {
        Resolved::Int(v) => Core::ConstInt(v),
        Resolved::Bool(b) => Core::ConstBool(b),
        Resolved::Unit => Core::Unit,
        Resolved::If { cond, then_, else_ } => Core::If { cond, then_, else_ },
        Resolved::Poison(r) => Core::Poison(r),
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
