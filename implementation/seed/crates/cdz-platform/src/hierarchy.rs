//! The reducer hierarchy — the spawn-tree parent links (`design/cadenza-platform.md` §3/§7).
//!
//! When a reducer is spawned, its parent is recorded, and that link is immutable on both sides (§7). The
//! links form a tree rooted at the reducers created at genesis. Together with the handler chains
//! ([`HandlerRegistry`](crate::HandlerRegistry)), the hierarchy is the **routing substrate** the kernel
//! maintains as sessions spawn: the system reducer reads it through the privileged API (§3/§4) to assemble
//! a handler chain across generations — the emitting reducer's own segment first, then its parent's, then
//! the grandparents' — and to reason about authority down the tree, since a child's effects pass through
//! its ancestors' middleware before reaching an edge (§5).
//!
//! It holds only the **active set** — the reducers currently alive. A reducer is added when it spawns and
//! removed when it terminates ([`remove`](Hierarchy::remove)), so the structure stays bounded to what is
//! running rather than growing with every reducer that ever existed. Removal is active-set cleanup, not a
//! mutation of the historical record: the immutable parent link a session was born with lives in its log
//! (§7), which is retained and queryable after it ends; this structure just stops tracking it once it is no
//! longer live.
//!
//! The links are **bidirectional**: a reducer's parent walks toward the root (chain assembly, authority),
//! and a reducer's children walk down (a parent deciding which descendants to propagate a new handler to,
//! §3).
//!
//! This is the hierarchy as a plain data structure. It is reached by direct read, not an effect, so the
//! operations are plain synchronous methods; the mutator takes `&mut self` because the owner records a link
//! as each reducer spawns and drops it when the reducer ends.

use crate::Hash;
use std::collections::{BTreeSet, HashMap};

/// One reducer in the active tree: its parent and its live children. A **root** (a reducer created at
/// genesis, with no parent) is recorded as its own parent, so every node in the set has a `parent` and the
/// roots are exactly the self-parented nodes.
#[derive(Debug, Clone)]
struct Node {
    parent: Hash,
    children: BTreeSet<Hash>,
}

/// The spawn tree over the active set: for each live reducer, its parent and its children (§7). A parent
/// link is **immutable** while the reducer lives — recorded once, never changed — and a spawn only ever adds
/// a fresh child under a live parent, so the structure is always a cycle-free forest. A reducer leaves the
/// set only by [`remove`](Hierarchy::remove) when it terminates.
#[derive(Debug, Default, Clone)]
pub struct Hierarchy {
    nodes: HashMap<Hash, Node>,
}

impl Hierarchy {
    /// An empty hierarchy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a **root** — a reducer created at genesis, with no parent. Returns `false` if the id is already
    /// in the set. A root is stored as its own parent, which is how [`parent`](Self::parent) and
    /// [`ancestors`](Self::ancestors) recognize the top of the tree.
    pub fn insert_root(&mut self, id: Hash) -> bool {
        if self.nodes.contains_key(&id) {
            return false;
        }
        self.nodes.insert(
            id,
            Node {
                parent: id,
                children: BTreeSet::new(),
            },
        );
        true
    }

    /// Record that `child` was spawned by `parent`, returning whether it was recorded.
    ///
    /// `parent` must already be in the active set (you spawn under a live reducer) and `child` must be new —
    /// its parent link is immutable while it lives (§7). Because a spawn only ever adds a fresh child under
    /// an existing parent, the child can never already be an ancestor of the parent, so no cycle can form and
    /// none is checked for. Returns `false` if `parent` is absent, `child` is already present, or the two are
    /// equal (a root is added with [`insert_root`](Self::insert_root), not here).
    pub fn record_spawn(&mut self, child: Hash, parent: Hash) -> bool {
        if child == parent || self.nodes.contains_key(&child) || !self.nodes.contains_key(&parent) {
            return false;
        }
        self.nodes
            .get_mut(&parent)
            .expect("parent present, just checked")
            .children
            .insert(child);
        self.nodes.insert(
            child,
            Node {
                parent,
                children: BTreeSet::new(),
            },
        );
        true
    }

    /// Remove `reducer` from the active set when it terminates, returning whether it was removed. It must be
    /// a **leaf** — a reducer with live children is refused (`false`) so that teardown proceeds bottom-up and
    /// no descendant is left pointing at a parent that is gone. On success it is detached from its parent's
    /// children and its node dropped.
    pub fn remove(&mut self, reducer: Hash) -> bool {
        match self.nodes.get(&reducer) {
            Some(node) if node.children.is_empty() => {
                let parent = node.parent;
                self.nodes.remove(&reducer);
                // Detach from the parent's children (a root is its own parent — nothing to detach).
                if parent != reducer
                    && let Some(parent_node) = self.nodes.get_mut(&parent)
                {
                    parent_node.children.remove(&reducer);
                }
                true
            }
            // Absent, or still has live children — refuse.
            _ => false,
        }
    }

    /// Whether `reducer` is in the active set.
    #[must_use]
    pub fn contains(&self, reducer: Hash) -> bool {
        self.nodes.contains_key(&reducer)
    }

    /// The parent of `reducer`, or `None` if it is a root or not in the active set.
    #[must_use]
    pub fn parent(&self, reducer: Hash) -> Option<Hash> {
        self.nodes
            .get(&reducer)
            .and_then(|node| (node.parent != reducer).then_some(node.parent))
    }

    /// The live children of `reducer`, in ascending id order. Empty for a leaf or a reducer not in the set.
    /// This is the direction a parent walks to decide which descendants to propagate a new handler to (§3).
    pub fn children(&self, reducer: Hash) -> impl Iterator<Item = Hash> + '_ {
        self.nodes
            .get(&reducer)
            .into_iter()
            .flat_map(|node| node.children.iter().copied())
    }

    /// The ancestors of `reducer`, nearest first: its parent, then grandparent, up to and including the root.
    /// Empty for a root or an unknown reducer. This is the order the system reducer walks to build a chain
    /// across generations (§3): the reducer's own segment first, then each ancestor's in turn.
    #[must_use]
    pub fn ancestors(&self, reducer: Hash) -> Ancestors<'_> {
        Ancestors {
            hierarchy: self,
            next: self.parent(reducer),
        }
    }

    /// Whether `ancestor` lies on the path from `reducer` up to the root — the authority relation the spawn
    /// tree encodes (an ancestor's middleware governs everything a descendant does, §5).
    #[must_use]
    pub fn is_ancestor(&self, ancestor: Hash, of: Hash) -> bool {
        self.ancestors(of).any(|a| a == ancestor)
    }
}

/// Iterator over a reducer's ancestors, nearest first — see [`Hierarchy::ancestors`].
pub struct Ancestors<'a> {
    hierarchy: &'a Hierarchy,
    next: Option<Hash>,
}

impl Iterator for Ancestors<'_> {
    type Item = Hash;

    fn next(&mut self) -> Option<Hash> {
        let current = self.next?;
        self.next = self.hierarchy.parent(current);
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::Hierarchy;
    use crate::Hash;

    // Distinct hashes standing in for reducer ids.
    fn r(tag: &str) -> Hash {
        Hash::of(tag.as_bytes())
    }

    #[test]
    fn a_root_has_no_parent_and_no_ancestors() {
        let mut h = Hierarchy::new();
        assert!(h.insert_root(r("root")));
        assert!(h.contains(r("root")));
        assert_eq!(h.parent(r("root")), None);
        assert_eq!(h.ancestors(r("root")).count(), 0);
        // inserting the same root twice is refused.
        assert!(!h.insert_root(r("root")));
    }

    #[test]
    fn spawn_requires_a_live_parent_and_a_fresh_child() {
        let mut h = Hierarchy::new();
        // no parent in the set yet — refused.
        assert!(!h.record_spawn(r("child"), r("ghost")));
        h.insert_root(r("root"));
        assert!(h.record_spawn(r("child"), r("root")));
        // the same child cannot be spawned again (its link is immutable while it lives).
        assert!(!h.record_spawn(r("child"), r("root")));
        // a reducer cannot spawn itself.
        assert!(!h.record_spawn(r("x"), r("x")));
    }

    #[test]
    fn links_are_bidirectional_parent_up_and_children_down() {
        let mut h = Hierarchy::new();
        h.insert_root(r("root"));
        h.record_spawn(r("a"), r("root"));
        h.record_spawn(r("b"), r("root"));
        h.record_spawn(r("a1"), r("a"));
        // parent, up.
        assert_eq!(h.parent(r("a1")), Some(r("a")));
        assert_eq!(h.parent(r("a")), Some(r("root")));
        // children, down (ascending id order, and only the direct children).
        let mut root_children: Vec<Hash> = h.children(r("root")).collect();
        let mut expected = vec![r("a"), r("b")];
        root_children.sort_unstable();
        expected.sort_unstable();
        assert_eq!(root_children, expected);
        assert_eq!(h.children(r("a")).collect::<Vec<_>>(), vec![r("a1")]);
        assert_eq!(h.children(r("a1")).count(), 0, "a leaf has no children");
    }

    #[test]
    fn ancestors_walk_from_the_reducer_up_to_and_including_the_root() {
        let mut h = Hierarchy::new();
        h.insert_root(r("root"));
        h.record_spawn(r("parent"), r("root"));
        h.record_spawn(r("child"), r("parent"));
        let chain: Vec<Hash> = h.ancestors(r("child")).collect();
        assert_eq!(chain, vec![r("parent"), r("root")]);
        assert!(h.is_ancestor(r("root"), r("child")));
        assert!(h.is_ancestor(r("parent"), r("child")));
        assert!(!h.is_ancestor(r("child"), r("root")));
    }

    #[test]
    fn remove_takes_a_leaf_out_of_the_active_set_and_off_its_parent() {
        let mut h = Hierarchy::new();
        h.insert_root(r("root"));
        h.record_spawn(r("a"), r("root"));
        h.record_spawn(r("a1"), r("a"));
        // a still has a live child, so it cannot be removed yet — teardown is bottom-up.
        assert!(!h.remove(r("a")));
        // removing the leaf detaches it from a's children and drops it from the set.
        assert!(h.remove(r("a1")));
        assert!(!h.contains(r("a1")));
        assert_eq!(h.children(r("a")).count(), 0);
        // now a is a leaf and can be removed, detaching it from root.
        assert!(h.remove(r("a")));
        assert_eq!(h.children(r("root")).count(), 0);
        // removing something absent is a no-op.
        assert!(!h.remove(r("a")));
    }

    #[test]
    fn a_removed_id_can_be_spawned_again() {
        // Removal clears the mapping, so the id is free to reappear (a fresh reducer, in practice a new id).
        let mut h = Hierarchy::new();
        h.insert_root(r("root"));
        h.record_spawn(r("c"), r("root"));
        assert!(h.remove(r("c")));
        assert!(h.record_spawn(r("c"), r("root")), "the slot was cleared");
        assert_eq!(h.parent(r("c")), Some(r("root")));
    }
}
