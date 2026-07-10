//! The resolved node form — one entry of the resolved column, keyed by AST `StructId`.
//!
//! This is the tree-above-the-core rung, but it is NOT a separate arena: a node's resolved form is a
//! *column* over the AST's own identity (`query-engine.md` §The Compiler's State Is Columns Indexed
//! By Node Identity). `resolved_of(id)` fills the slot for one node; the node references its children
//! by their AST `StructId`, so descending into a child is the same lazy column read on a different
//! id. The source's nesting is therefore preserved (a child is reached through the parent) without
//! copying the tree into a second arena.
//!
//! Resolving a node is per-node and does NOT recurse: it classifies the AST occurrence at `id` and
//! records what it denotes, leaving the children as ids for a later demand to resolve. A "no" is a
//! value here — an unrecognized or malformed construct is [`Resolved::Poison`] — so the resolved
//! column is total over every node a query reaches.

use crate::ast::{IntValue, StructId};
use crate::diag::Reject;

/// The resolved meaning of one AST node. Frozen for Stage 0 at the slice's constructs. Children are
/// referenced by AST `StructId`; a query descends by reading their slots on demand.
#[derive(Clone, PartialEq, Debug)]
pub enum Resolved {
    /// An integer literal at its exact arbitrary precision. Its machine width is a downstream type
    /// decision; the narrowing (and any out-of-range decline) happens at selection.
    Int(IntValue),
    /// A boolean literal.
    Bool(bool),
    /// The unit value (`()`).
    Unit,
    /// A two-way conditional. The three children are AST occurrences resolved on demand.
    If { cond: StructId, then_: StructId, else_: StructId },
    /// A produced "no": an unrecognized head, a malformed form, or an unmodeled literal. Carries its
    /// reject/decline so the fault is reported at the node it was found rather than reconstructed.
    Poison(Reject),
}
