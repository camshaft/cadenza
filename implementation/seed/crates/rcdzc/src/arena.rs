//! The arena substrate — a rung's nodes held in a contiguous region, addressed by a stable index.
//!
//! This is the storage the columns model rides on
//! (`intermediate-representations.md` §A Rung's Nodes Are Held In A Contiguous Arena, Referenced By
//! Index; `query-engine.md` §The Compiler's State Is Columns Indexed By Node Identity). Two shapes:
//!
//!  - [`Arena`] holds a rung's nodes; pushing one returns its identity (its index), and a node
//!    references another node by that identity rather than by an owning pointer. Node identity is
//!    assigned by a fixed post-order `push`, so it is a deterministic function of the program's
//!    structure — no allocation address or hash-iteration order ever reaches an index.
//!  - [`Column`] holds one *fact* per node identity — the solved type, an origin back-reference —
//!    keyed by the SAME index that addresses the arena. A slot is either filled or absent; absence
//!    means only "no answer was determined here", never a defaulted value.
//!
//! **Strongly typed by index.** Both are parameterized by the id type that addresses them, so the
//! index type is a static tag for *which rung* a node or fact belongs to: an `Arena<HirId, _>` can
//! only be indexed by a `HirId`, and addressing it with a `CoreId` is a compile error. The single
//! Rust-ism here is `PhantomData<Id>`; in the eventual Cadenza port the id type is simply the
//! arena's declared index type, so this stays a faithful mirror.
//!
//! An id is created ONLY by an arena's `push`, so an index a node carries always refers to a node
//! that arena holds — the "an index remains valid for the lifetime of the rung" obligation.

use std::marker::PhantomData;

/// A type usable as an arena/column index: it wraps a `u32` position. Implemented by the per-rung id
/// newtypes (`HirId`, `CoreId`, `LocalId`, …) via [`define_id!`]. Kept deliberately minimal — one
/// number in, one number out — so the Cadenza port is a plain "index is a natural" story.
pub trait Index: Copy {
    /// The position this id addresses.
    fn ix(self) -> usize;
    /// Build an id from a position. Only the owning arena/column calls this.
    fn from_ix(ix: usize) -> Self;
}

/// Define a rung's index newtype implementing [`Index`]. One line per rung id, so every id is the
/// same shape (a `u32` position) and the boilerplate lives in one macro rather than repeated by hand.
#[macro_export]
macro_rules! define_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $crate::arena::Index for $name {
            fn ix(self) -> usize {
                self.0 as usize
            }
            fn from_ix(ix: usize) -> Self {
                $name(ix as u32)
            }
        }
    };
}

// The AST's `StructId` — the node identity every column keys on — is itself an [`Index`]. The impl
// lives here rather than in the copied `ast.rs` so that file stays verbatim (copy-don't-depend): the
// `Index` trait is rcdzc's, so implementing it for a copied type is rcdzc's concern, added at the
// substrate that defines the trait.
impl Index for crate::ast::StructId {
    fn ix(self) -> usize {
        self.0 as usize
    }
    fn from_ix(ix: usize) -> Self {
        crate::ast::StructId(ix as u32)
    }
}

/// A contiguous arena of `Node`s, addressed by index type `Id`. Pushing a node returns its identity.
pub struct Arena<Id: Index, Node> {
    nodes: Vec<Node>,
    _id: PhantomData<Id>,
}

impl<Id: Index, Node> Arena<Id, Node> {
    /// An empty arena.
    pub fn new() -> Arena<Id, Node> {
        Arena { nodes: Vec::new(), _id: PhantomData }
    }

    /// Append `node`, returning its identity (its index). Deterministic: the k-th push is index k.
    pub fn push(&mut self, node: Node) -> Id {
        let ix = self.nodes.len();
        self.nodes.push(node);
        Id::from_ix(ix)
    }

    /// The node at `id`. Panics only on an id this arena did not produce, which cannot arise when ids
    /// come solely from `push` — an internal invariant, not an input error.
    pub fn get(&self, id: Id) -> &Node {
        &self.nodes[id.ix()]
    }

    /// The number of nodes held.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the arena holds no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl<Id: Index, Node> Default for Arena<Id, Node> {
    fn default() -> Arena<Id, Node> {
        Arena::new()
    }
}

/// One slot of a column: an answer, or no answer yet. `Absent` means ONLY that the phase owning the
/// column determined no answer for that node — never a default and never a negative outcome. A
/// negative *decision* (a decline, a rejection, a poison) is carried as a `Filled` value of the
/// column's element type, so it is distinguished from the absence of a decision
/// (`query-engine.md` §An Empty Column Slot Means Only That No Answer Was Determined).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Slot<T> {
    Absent,
    Filled(T),
}

/// A column: one fact per node identity, keyed by index type `Id`. Dense — a `Vec` of slots grown to
/// cover the ids filled so far; unfilled positions read as `Absent`.
pub struct Column<Id: Index, T> {
    slots: Vec<Slot<T>>,
    _id: PhantomData<Id>,
}

impl<Id: Index, T> Column<Id, T> {
    /// An empty column (every slot absent).
    pub fn new() -> Column<Id, T> {
        Column { slots: Vec::new(), _id: PhantomData }
    }

    /// Fill the slot at `id` with `value`, growing the column to reach it. A producer calls this to
    /// record the fact it determined about a node.
    pub fn fill(&mut self, id: Id, value: T) {
        let ix = id.ix();
        while self.slots.len() <= ix {
            self.slots.push(Slot::Absent);
        }
        self.slots[ix] = Slot::Filled(value);
    }

    /// Read the slot at `id`. An id past the filled region reads as `Absent`.
    pub fn get(&self, id: Id) -> &Slot<T> {
        match self.slots.get(id.ix()) {
            Some(slot) => slot,
            None => &Slot::Absent,
        }
    }

    /// Read the value at `id`, or `None` if the slot is absent. A reader that REQUIRES a value uses
    /// this and declines on `None` rather than substituting a default — the sparse model must never
    /// silently read a not-yet-determined fact as a convenient value
    /// (`query-engine.md` §A Reader That Requires A Value And Finds Absence Declines Rather Than
    /// Defaults).
    pub fn require(&self, id: Id) -> Option<&T> {
        match self.get(id) {
            Slot::Filled(v) => Some(v),
            Slot::Absent => None,
        }
    }
}

impl<Id: Index, T> Default for Column<Id, T> {
    fn default() -> Column<Id, T> {
        Column::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway id type for exercising the substrate.
    crate::define_id!(TestId);

    #[test]
    fn push_assigns_sequential_ids() {
        let mut a: Arena<TestId, &str> = Arena::new();
        let x = a.push("x");
        let y = a.push("y");
        assert_eq!(x, TestId(0));
        assert_eq!(y, TestId(1));
        assert_eq!(a.get(x), &"x");
        assert_eq!(a.get(y), &"y");
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn column_absent_until_filled() {
        let mut c: Column<TestId, i64> = Column::new();
        // Absent before any fill — and `require` declines rather than defaults.
        assert_eq!(c.get(TestId(3)), &Slot::Absent);
        assert_eq!(c.require(TestId(3)), None);
        c.fill(TestId(3), 42);
        assert_eq!(c.require(TestId(3)), Some(&42));
        // A gap left by filling a later id stays absent, not zero.
        assert_eq!(c.require(TestId(0)), None);
    }
}
