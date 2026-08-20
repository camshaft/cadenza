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
//! This is the hierarchy as a plain data structure. It is reached by direct read, not an effect, so the
//! operations are plain synchronous methods; the mutator takes `&mut self` because the owner records a link
//! as each reducer spawns.

use crate::Hash;
use std::collections::HashMap;

/// The spawn tree: each reducer's parent, recorded when it spawns (§7). A reducer with no entry is a root
/// (created at genesis). A parent link is **immutable** — recorded once and never changed — which, with
/// every child spawned under an already-existing parent, keeps the structure a cycle-free forest.
#[derive(Debug, Default, Clone)]
pub struct Hierarchy {
    /// child -> parent. A key's absence means the reducer is a root (or not yet known).
    parents: HashMap<Hash, Hash>,
}

impl Hierarchy {
    /// An empty hierarchy — every reducer is a root until a parent is recorded for it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `child` was spawned by `parent`, returning whether the link was recorded.
    ///
    /// The link is immutable (§7): if `child` already has a parent it is left untouched and this returns
    /// `false`. It also refuses to form a cycle — a reducer cannot be its own parent, nor be parented to one
    /// of its own descendants — returning `false` in that case too. In a real spawn a child's id derives
    /// from its parent's genesis, so the parent always pre-exists and neither case can arise; the guards
    /// just keep the structure a valid forest under any caller.
    pub fn record_spawn(&mut self, child: Hash, parent: Hash) -> bool {
        if child == parent || self.parents.contains_key(&child) {
            return false;
        }
        // Refuse a cycle: `parent` must not be `child` itself or a descendant of `child` — i.e. `child`
        // must not already be one of `parent`'s ancestors.
        if self.ancestors(parent).any(|ancestor| ancestor == child) {
            return false;
        }
        self.parents.insert(child, parent);
        true
    }

    /// The parent of `reducer`, or `None` if it is a root (or not recorded).
    #[must_use]
    pub fn parent(&self, reducer: Hash) -> Option<Hash> {
        self.parents.get(&reducer).copied()
    }

    /// The ancestors of `reducer`, nearest first: its parent, then grandparent, up to the root. Empty for a
    /// root. This is the order the system reducer walks to build a chain across generations (§3): the
    /// reducer's own segment first, then each ancestor's in turn.
    #[must_use]
    pub fn ancestors(&self, reducer: Hash) -> Ancestors<'_> {
        Ancestors {
            hierarchy: self,
            next: self.parents.get(&reducer).copied(),
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
        self.next = self.hierarchy.parents.get(&current).copied();
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
        let h = Hierarchy::new();
        assert_eq!(h.parent(r("root")), None);
        assert_eq!(h.ancestors(r("root")).count(), 0);
    }

    #[test]
    fn ancestors_walk_from_the_reducer_up_to_the_root_in_order() {
        let mut h = Hierarchy::new();
        // root <- parent <- child (each spawned under an existing reducer)
        h.record_spawn(r("parent"), r("root"));
        h.record_spawn(r("child"), r("parent"));
        assert_eq!(h.parent(r("child")), Some(r("parent")));
        // nearest first: parent, then grandparent (root).
        let chain: Vec<Hash> = h.ancestors(r("child")).collect();
        assert_eq!(chain, vec![r("parent"), r("root")]);
        // and the authority relation the walk encodes.
        assert!(h.is_ancestor(r("root"), r("child")));
        assert!(h.is_ancestor(r("parent"), r("child")));
        assert!(!h.is_ancestor(r("child"), r("root")));
    }

    #[test]
    fn a_parent_link_is_immutable_once_recorded() {
        let mut h = Hierarchy::new();
        assert!(h.record_spawn(r("child"), r("first-parent")));
        // a second spawn record for the same child is refused; the original link stands.
        assert!(!h.record_spawn(r("child"), r("other-parent")));
        assert_eq!(h.parent(r("child")), Some(r("first-parent")));
    }

    #[test]
    fn a_cycle_is_refused() {
        let mut h = Hierarchy::new();
        h.record_spawn(r("b"), r("a"));
        h.record_spawn(r("c"), r("b"));
        // a reducer cannot be its own parent.
        assert!(!h.record_spawn(r("x"), r("x")));
        // and cannot be parented to one of its own descendants: a already has c as a descendant, so making
        // a's parent be c would close a loop. (a is a root here, so it has no parent yet — eligible but for
        // the cycle guard.)
        assert!(!h.record_spawn(r("a"), r("c")));
        assert_eq!(
            h.parent(r("a")),
            None,
            "a stays a root; the cycle was refused"
        );
    }
}
