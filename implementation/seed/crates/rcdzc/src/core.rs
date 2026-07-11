//! The core node form — one entry of the core column, the A-normal rung the one evaluator runs over
//! and a backend consumes.
//!
//! Like the resolved column, this is a *column* over the AST's own `StructId`, not a second arena:
//! `core_of(id)` fills one node's core form, referencing children by their AST `StructId`. It is in
//! **A-normal form** — every operand of an operation is an atom (a constant or, later, a bound name)
//! — so value flow is explicit (`reference-compiler.md` §The Core Representation Is In A-Normal
//! Form). It retains *structured* control (`If`); a linearizing backend flattens that itself
//! (`intermediate-representations.md` §The Fully-Linearized Block Form Is A Linearizing Backend's
//! Representation).
//!
//! **On administrative bindings.** A-normalization names a non-trivial subexpression with a binding
//! (`Let`/`LocalRef`) so its value flow is explicit rather than implicit in an expression's nesting
//! (`reference-compiler.md` §The Core Representation Is In A-Normal Form). This rung realizes the
//! FIRST case where naming earns its keep: a source `let` whose bound value is a RUNTIME computation
//! (not a compile-time constant) used MORE THAN ONCE. Today such a binding would be followed through
//! at each reference — recomputing the value per use — so [`Core::Let`] names it once and each use is
//! a [`Core::LocalRef`] to that name. A binding used at most once, or one whose value folds to a
//! constant, is still copy-propagated / erased at lowering (the ADMIN-REDEX ELIMINATION the one
//! evaluator owes — `reference-compiler.md` ¶3), so naming every intermediate adds no runtime cost
//! and the emitted bytes are unchanged for a program that has no multi-use runtime binding.
//!
//! A `Let` binding is keyed by its INITIALIZER's AST `StructId` — the stable identity a reference to
//! the binding already resolves to (`Resolved::Ref { value }`), so no fresh id space is needed for
//! this case: the slot a binding occupies and the `LocalRef`s that read it share that one occurrence.
//! (Admin bindings with NO source occurrence — the ones a general A-normalization of every operand
//! synthesizes — arrive with the core's own fresh-id space in a later stage; this rung names only the
//! binding a source `let` already gives an occurrence to.)

use crate::ast::{IntValue, StructId};
use crate::diag::Reject;
use crate::resolved::{Prim, Symbol};
use std::collections::BTreeMap;

/// The core (A-normal) form of one node.
#[derive(Clone, PartialEq, Debug)]
pub enum Core {
    /// An integer constant at exact arbitrary precision. The narrowing to the machine width its
    /// solved type fixes is the backend's job at selection.
    ConstInt(IntValue),
    /// A boolean constant.
    ConstBool(bool),
    /// The unit value.
    Unit,
    /// A record value — a fixed set of named fields, each field's value referenced by its AST
    /// occurrence. Stage 1 FOLDS a record at a field read (`core_of` of a member projects the field's
    /// core directly), so a `Record` that SURVIVES to selection is one used as a runtime value (e.g.
    /// returned) — which needs the value heap and therefore DECLINES for now. Carrying the variant
    /// lets member-access fold read the field set.
    Record { fields: BTreeMap<Symbol, StructId> },
    /// A two-way conditional over atoms; structured control retained. Children are AST `StructId`s.
    If {
        cond: StructId,
        then_: StructId,
        else_: StructId,
    },
    /// An A-normal binding sequence: name each `(binder, value)` — its VALUE computed once — then the
    /// `body` uses each name by a [`Core::LocalRef`]. The `binder` is the initializer's AST `StructId`
    /// (the identity a reference to the binding resolves to), so the slot a binding occupies and the
    /// refs that read it agree without a fresh id space. Present ONLY for a source `let` binding whose
    /// value is a runtime computation used more than once — the case where naming avoids recomputing;
    /// a single-use or constant binding is copy-propagated / erased at lowering, leaving no `Let`.
    /// Bindings are sequential (a later one may reference an earlier's name), matching `let*`.
    Let {
        bindings: Vec<(StructId, StructId)>,
        body: StructId,
    },
    /// A reference to a [`Core::Let`] binding — read the value named by `binder` (the initializer
    /// occurrence the binding was keyed by). The backend maps it to a `local.get` of the binding's
    /// slot, exactly as a `Param` reads a parameter slot. Present only when the referenced binding was
    /// KEPT as a `Core::Let` (a multi-use runtime value); a reference to a propagated binding lowers to
    /// the value's own core instead.
    LocalRef { binder: StructId },
    /// A runtime arithmetic operation on two operands (children by AST `StructId`). Present only when
    /// the fold could NOT reduce the operation to a constant (an operand is not compile-time-known — a
    /// FUNCTION PARAMETER). Constant arithmetic folds to `ConstInt`/`Poison` in `lower`. The machine op
    /// the backend emits is selected from the operands' solved width.
    Arith {
        op: Prim,
        lhs: StructId,
        rhs: StructId,
    },
    /// A runtime comparison on two operands (children by AST `StructId`) — result is a `Bool` (an i32
    /// at the machine level). Present only when the fold could not decide it (a runtime operand); two
    /// constants fold to `ConstBool` in `lower`. The machine op is selected from the operands' width
    /// and signedness.
    Compare {
        op: Prim,
        lhs: StructId,
        rhs: StructId,
    },
    /// A runtime integer CONVERSION on one operand (child by AST `StructId`) — present only when the
    /// fold could not reduce it (the operand is a runtime value). `Prim::Wrap` truncates the operand to
    /// the node's solved TARGET width/signedness (read off at selection); a constant operand folds to a
    /// `ConstInt` in `lower`. The target is the conversion node's OWN solved type, not the operand's.
    Convert { op: Prim, operand: StructId },
    /// A reference to a FUNCTION PARAMETER — the `binder` is the parameter's name occurrence (its
    /// identity, matching what `resolve` binds a reference to). The backend maps it to a `local.get` of
    /// the parameter's slot. This is the runtime value a bare literal is not: a parameter's value is
    /// unknown at compile time, so a `Param` reaching selection lowers to a local read rather than a
    /// constant. (Only present when a function body is lowered STANDALONE — i.e. it is emitted as a real
    /// wasm function, not inlined-and-folded at a constant call site.)
    Param { binder: StructId },
    /// A produced "no" carried into the core.
    Poison(Reject),
}
