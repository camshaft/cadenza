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
//! **On administrative bindings.** A-normalization in general introduces FRESH bindings that name a
//! non-atomic subexpression (`Let`/`LocalRef`). Those cannot be keyed by an AST `StructId`, because
//! they have no source occurrence — so they will arrive together with the core's own fresh-id space
//! in the stage that first needs them. The Stage-0 slice has nothing non-atomic to name (every
//! operand is already an atom), so the core column keys cleanly by `StructId` now, and this rung
//! carries only the variants that map 1:1 to a source node. Shipping a `Let` we could neither
//! construct nor key would be dead shape.

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
    If { cond: StructId, then_: StructId, else_: StructId },
    /// A runtime arithmetic operation on two operands (children by AST `StructId`). Present only when
    /// the fold could NOT reduce the operation to a constant (an operand is not compile-time-known —
    /// which in this increment means it declines, since there are no runtime integer operands yet
    /// without functions). Constant arithmetic folds to `ConstInt`/`Poison` in `lower`. The machine op
    /// the backend emits is selected from the operands' solved width.
    Arith { op: Prim, lhs: StructId, rhs: StructId },
    /// A produced "no" carried into the core.
    Poison(Reject),
}
