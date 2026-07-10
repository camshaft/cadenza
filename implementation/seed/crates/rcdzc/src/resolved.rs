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
//!
//! A bare name resolves — by the lexical-scope walk (`resolve::scope`) then the prelude map — to what
//! it denotes: a [`Resolved::Ref`] to the value occurrence it is bound to, or a `Poison` if unbound.
//! A member KEY is never resolved this way: it is a [`Symbol`] label (its spelling), read without any
//! scope/prelude lookup (`prelude-and-resolution.md` §A Member Key Is A Label, Not A Value).

use crate::ast::{IntValue, StructId};
use crate::diag::Reject;
use std::collections::BTreeMap;

/// A field/variant/member label — a name taken as data, NOT resolved to a value. A member access's
/// key and a record literal's field names are symbols: the projection finds a field BY this label and
/// never inspects a bound value for it.
///
/// A symbol carries an optional NAMESPACE so a name the language defines and a name a macro introduces
/// cannot collide (`contracts/ast-encoding.md` §A Prelude Symbol Is Namespaced). Ordering is by
/// (namespace, name), so a `BTreeMap` keyed by `Symbol` has a canonical field order — which is what
/// makes record equality and projection order-independent (a record's fields are a SET).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Symbol {
    /// The namespace this label belongs to, or `None` for an unqualified source name. Stage 0 source
    /// names are unqualified; the field exists so a macro-introduced label carries its origin.
    pub namespace: Option<String>,
    pub name: String,
}

impl Symbol {
    /// An unqualified label from a source spelling (the Stage-0 case — no namespace).
    pub fn plain(name: impl Into<String>) -> Symbol {
        Symbol { namespace: None, name: name.into() }
    }
}

/// The resolved meaning of one AST node. Children are referenced by AST `StructId`; a query descends
/// by reading their slots on demand.
#[derive(Clone, PartialEq, Debug)]
pub enum Resolved {
    /// An integer literal at its exact arbitrary precision. Its machine width is a downstream type
    /// decision; the narrowing (and any out-of-range decline) happens at selection.
    Int(IntValue),
    /// A boolean literal.
    Bool(bool),
    /// The unit value (`()`).
    Unit,
    /// A reference to a binding: the name at this occurrence denotes the value at `value` (the
    /// initializer of the nearest enclosing `let`/`def`-parameter binding of that name). `type_of` and
    /// `core_of` follow the ref — a bare name IS its bound value's fact.
    Ref { value: StructId },
    /// A `let` binding form: each `(name init)` pair binds `name` to `init` for the initializers and
    /// body that follow (sequential; a later binding sees earlier ones; a repeat shadows). The whole
    /// form's value is `body`'s value. Bindings are carried as `(binder-name-occ, init-occ)` pairs so
    /// scope resolution finds them by walking here from a reference.
    Let { bindings: Vec<(StructId, StructId)>, body: StructId },
    /// A two-way conditional. The three children are AST occurrences resolved on demand.
    If { cond: StructId, then_: StructId, else_: StructId },
    /// A record literal: a fixed SET of named fields, each label mapping to its value occurrence. Held
    /// as a `BTreeMap` so the fields are canonically ordered (order-independent equality/projection)
    /// and a field lookup is O(log n), not a linear scan. The labels are symbols (never resolved); the
    /// values resolve on demand. A duplicate label is a `Poison` before construction (a record's field
    /// names are a set — `core-semantics.md` §A Record Has A Fixed Set Of Named Fields).
    Record { fields: BTreeMap<Symbol, StructId> },
    /// Member access `(. operand key)` — the ONE generic projection. `key` is a label read from the
    /// key occurrence's spelling, NOT resolved (`prelude-and-resolution.md` §Member Access Is One
    /// Generic Projection That Does Not Inspect Its Key). The projection resolves the field against
    /// the operand's type/value downstream.
    Member { operand: StructId, key: Symbol },
    /// A produced "no": an unrecognized head, a malformed form, an unbound name, or an unmodeled
    /// literal. Carries its reject/decline so the fault is reported at the node it was found.
    Poison(Reject),
}
