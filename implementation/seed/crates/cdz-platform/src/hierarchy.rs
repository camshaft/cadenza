//! The reducer hierarchy — the spawn-tree parent links (`design/cadenza-platform.md` §3/§7).
//!
//! When a reducer is spawned, its parent is recorded, and that link is immutable on both sides (§7). The
//! links form a tree rooted at the reducers created at genesis. Together with the handler chains
//! ([`HandlerRegistry`](crate::HandlerRegistry)), the hierarchy is the **routing substrate** the kernel
//! maintains as sessions spawn: the system reducer reads it through the privileged API (§3/§4) to assemble a
//! handler chain across generations — the emitting reducer's own segment first, then its parent's, then the
//! grandparents' — and to reason about authority down the tree, since a child's effects pass through its
//! ancestors' middleware before reaching an edge (§5).
//!
//! It holds only the **active set** — the reducers currently alive. A reducer is added when it spawns and
//! removed when it terminates ([`remove`](Hierarchy::remove)), so the structure stays bounded to what is
//! running. Removal is active-set cleanup, not a mutation of the historical record: the immutable parent
//! link a session was born with lives in its log (§7), retained after it ends; this just stops tracking it.
//!
//! Tracking spawns is the running system's job, so [`Hierarchy`] is an **async trait** shared behind an
//! `Arc`: an in-memory build answers from a local map ([`InMemoryHierarchy`]); a distributed build answers
//! from a replicated structure. The queries are async for the same reason the rest of the system's
//! operations are — a replicated read awaits — and the mutators take `&self` (interior mutability), because
//! the system records a link as each reducer spawns and drops it when the reducer ends, concurrently with
//! reads. `ancestors` and `children` return owned `Vec`s (not borrowing iterators) so they cross the async
//! trait boundary.

use crate::ReducerId;
use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// The spawn tree over the active set: for each live reducer, its parent and its children (§7). A parent
/// link is **immutable** while the reducer lives, and a spawn only ever adds a fresh child under a live
/// parent, so the structure is always a cycle-free forest. A reducer leaves the set only by
/// [`remove`](Hierarchy::remove) when it terminates.
#[async_trait]
pub trait Hierarchy: Send + Sync {
    /// Add a **root** — a reducer created at genesis, with no parent. Returns `false` if the id is already in
    /// the set. A root is its own parent, which is how [`parent`](Self::parent) recognizes the top of a tree.
    async fn insert_root(&self, id: ReducerId) -> bool;

    /// Record that `child` was spawned by `parent`, returning whether it was recorded. `parent` must be in
    /// the active set and `child` must be new (its parent link is immutable while it lives, §7). Since a
    /// spawn only adds a fresh child under a live parent, no cycle can form. Returns `false` if `parent` is
    /// absent, `child` is already present, or the two are equal (a root uses [`insert_root`](Self::insert_root)).
    async fn record_spawn(&self, child: ReducerId, parent: ReducerId) -> bool;

    /// Remove `reducer` from the active set when it terminates, returning whether it was removed. It must be a
    /// **leaf** — a reducer with live children is refused — so teardown proceeds bottom-up and no descendant
    /// is left pointing at a parent that is gone.
    async fn remove(&self, reducer: ReducerId) -> bool;

    /// Whether `reducer` is in the active set.
    async fn contains(&self, reducer: ReducerId) -> bool;

    /// The parent of `reducer`, or `None` if it is a root or not in the active set.
    async fn parent(&self, reducer: ReducerId) -> Option<ReducerId>;

    /// The live children of `reducer`, in ascending id order — the direction a parent walks to decide which
    /// descendants to propagate a new handler to (§3). Empty for a leaf or an unknown reducer.
    async fn children(&self, reducer: ReducerId) -> Vec<ReducerId>;

    /// The ancestors of `reducer`, nearest first: its parent, then grandparent, up to and including the root.
    /// The order the system reducer walks to build a chain across generations (§3). Empty for a root or an
    /// unknown reducer.
    async fn ancestors(&self, reducer: ReducerId) -> Vec<ReducerId>;

    /// Whether `ancestor` lies on the path from `of` up to the root — the authority relation the spawn tree
    /// encodes (an ancestor's middleware governs everything a descendant does, §5).
    async fn is_ancestor(&self, ancestor: ReducerId, of: ReducerId) -> bool {
        self.ancestors(of).await.contains(&ancestor)
    }
}

/// One reducer in the active tree: its parent and its live children. A **root** is its own parent, so every
/// node has a `parent` and the roots are exactly the self-parented nodes.
#[derive(Debug, Clone)]
struct Node {
    parent: ReducerId,
    children: BTreeSet<ReducerId>,
}

/// An in-memory [`Hierarchy`] — the active spawn tree as a local map. For tests and single-process use; a
/// distributed build tracks the same tree in a replicated structure. Interior mutability (a `Mutex`), since
/// the system records and drops links behind a shared `Arc`.
#[derive(Debug, Default)]
pub struct InMemoryHierarchy {
    nodes: Mutex<HashMap<ReducerId, Node>>,
}

impl InMemoryHierarchy {
    /// An empty hierarchy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Hierarchy for InMemoryHierarchy {
    async fn insert_root(&self, id: ReducerId) -> bool {
        let mut nodes = self.nodes.lock().expect("hierarchy lock");
        if nodes.contains_key(&id) {
            return false;
        }
        nodes.insert(
            id,
            Node {
                parent: id,
                children: BTreeSet::new(),
            },
        );
        true
    }

    async fn record_spawn(&self, child: ReducerId, parent: ReducerId) -> bool {
        let mut nodes = self.nodes.lock().expect("hierarchy lock");
        if child == parent || nodes.contains_key(&child) || !nodes.contains_key(&parent) {
            return false;
        }
        nodes
            .get_mut(&parent)
            .expect("parent present, just checked")
            .children
            .insert(child);
        nodes.insert(
            child,
            Node {
                parent,
                children: BTreeSet::new(),
            },
        );
        true
    }

    async fn remove(&self, reducer: ReducerId) -> bool {
        let mut nodes = self.nodes.lock().expect("hierarchy lock");
        match nodes.get(&reducer) {
            Some(node) if node.children.is_empty() => {
                let parent = node.parent;
                nodes.remove(&reducer);
                // Detach from the parent's children (a root is its own parent — nothing to detach).
                if parent != reducer
                    && let Some(parent_node) = nodes.get_mut(&parent)
                {
                    parent_node.children.remove(&reducer);
                }
                true
            }
            // Absent, or still has live children — refuse.
            _ => false,
        }
    }

    async fn contains(&self, reducer: ReducerId) -> bool {
        self.nodes
            .lock()
            .expect("hierarchy lock")
            .contains_key(&reducer)
    }

    async fn parent(&self, reducer: ReducerId) -> Option<ReducerId> {
        self.nodes
            .lock()
            .expect("hierarchy lock")
            .get(&reducer)
            .and_then(|node| (node.parent != reducer).then_some(node.parent))
    }

    async fn children(&self, reducer: ReducerId) -> Vec<ReducerId> {
        self.nodes
            .lock()
            .expect("hierarchy lock")
            .get(&reducer)
            .map(|node| node.children.iter().copied().collect())
            .unwrap_or_default()
    }

    async fn ancestors(&self, reducer: ReducerId) -> Vec<ReducerId> {
        // Walk the parent chain to the root under one lock — nearest first, up to and including the root.
        let nodes = self.nodes.lock().expect("hierarchy lock");
        let mut chain = Vec::new();
        let mut next = nodes
            .get(&reducer)
            .and_then(|node| (node.parent != reducer).then_some(node.parent));
        while let Some(current) = next {
            chain.push(current);
            next = nodes
                .get(&current)
                .and_then(|node| (node.parent != current).then_some(node.parent));
        }
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::{Hierarchy, InMemoryHierarchy};
    use crate::{Hash, ReducerId};

    // Distinct reducer ids.
    fn r(tag: &str) -> ReducerId {
        ReducerId::from_hash(Hash::of(tag.as_bytes()))
    }

    #[tokio::test]
    async fn a_root_has_no_parent_and_no_ancestors() {
        let h = InMemoryHierarchy::new();
        assert!(h.insert_root(r("root")).await);
        assert!(h.contains(r("root")).await);
        assert_eq!(h.parent(r("root")).await, None);
        assert!(h.ancestors(r("root")).await.is_empty());
        // inserting the same root twice is refused.
        assert!(!h.insert_root(r("root")).await);
    }

    #[tokio::test]
    async fn spawn_requires_a_live_parent_and_a_fresh_child() {
        let h = InMemoryHierarchy::new();
        // no parent in the set yet — refused.
        assert!(!h.record_spawn(r("child"), r("ghost")).await);
        h.insert_root(r("root")).await;
        assert!(h.record_spawn(r("child"), r("root")).await);
        // the same child cannot be spawned again (its link is immutable while it lives).
        assert!(!h.record_spawn(r("child"), r("root")).await);
        // a reducer cannot spawn itself.
        assert!(!h.record_spawn(r("x"), r("x")).await);
    }

    #[tokio::test]
    async fn links_are_bidirectional_parent_up_and_children_down() {
        let h = InMemoryHierarchy::new();
        h.insert_root(r("root")).await;
        h.record_spawn(r("a"), r("root")).await;
        h.record_spawn(r("b"), r("root")).await;
        h.record_spawn(r("a1"), r("a")).await;
        assert_eq!(h.parent(r("a1")).await, Some(r("a")));
        assert_eq!(h.parent(r("a")).await, Some(r("root")));
        let mut root_children = h.children(r("root")).await;
        let mut expected = vec![r("a"), r("b")];
        root_children.sort_unstable();
        expected.sort_unstable();
        assert_eq!(root_children, expected);
        assert_eq!(h.children(r("a")).await, vec![r("a1")]);
        assert!(
            h.children(r("a1")).await.is_empty(),
            "a leaf has no children"
        );
    }

    #[tokio::test]
    async fn ancestors_walk_from_the_reducer_up_to_and_including_the_root() {
        let h = InMemoryHierarchy::new();
        h.insert_root(r("root")).await;
        h.record_spawn(r("parent"), r("root")).await;
        h.record_spawn(r("child"), r("parent")).await;
        assert_eq!(h.ancestors(r("child")).await, vec![r("parent"), r("root")]);
        assert!(h.is_ancestor(r("root"), r("child")).await);
        assert!(h.is_ancestor(r("parent"), r("child")).await);
        assert!(!h.is_ancestor(r("child"), r("root")).await);
    }

    #[tokio::test]
    async fn remove_takes_a_leaf_out_of_the_active_set_and_off_its_parent() {
        let h = InMemoryHierarchy::new();
        h.insert_root(r("root")).await;
        h.record_spawn(r("a"), r("root")).await;
        h.record_spawn(r("a1"), r("a")).await;
        // a still has a live child, so it cannot be removed yet — teardown is bottom-up.
        assert!(!h.remove(r("a")).await);
        // removing the leaf detaches it from a's children and drops it from the set.
        assert!(h.remove(r("a1")).await);
        assert!(!h.contains(r("a1")).await);
        assert!(h.children(r("a")).await.is_empty());
        // now a is a leaf and can be removed, detaching it from root.
        assert!(h.remove(r("a")).await);
        assert!(h.children(r("root")).await.is_empty());
        // removing something absent is a no-op.
        assert!(!h.remove(r("a")).await);
    }

    #[tokio::test]
    async fn a_removed_id_can_be_spawned_again() {
        let h = InMemoryHierarchy::new();
        h.insert_root(r("root")).await;
        h.record_spawn(r("c"), r("root")).await;
        assert!(h.remove(r("c")).await);
        assert!(
            h.record_spawn(r("c"), r("root")).await,
            "the slot was cleared"
        );
        assert_eq!(h.parent(r("c")).await, Some(r("root")));
    }
}
