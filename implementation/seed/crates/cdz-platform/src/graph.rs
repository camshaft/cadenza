//! The reducer graph — the typed relationships the running platform maintains over its reducers
//! (`design/cadenza-platform.md` §3/§7).
//!
//! Every relationship between reducers the system reasons about is an edge in one directed graph over the
//! active reducer set, labelled by an [`EdgeKind`]. The spawn tree is the [`EdgeKind::spawn`] edges (each a
//! `child -> parent` link); other relationships — a supervision subscription, a capability grant, a
//! federation trust link — are simply other edge kinds over the same nodes. Because an edge kind is a hash,
//! a new relationship is a new hash a consumer mints, needing no change to this trait or its backends: the
//! graph stores and queries edges without knowing what any kind *means*, and the meaning lives with the
//! consumer that mints the kind.
//!
//! Together with the handler chains ([`HandlerRegistry`](crate::HandlerRegistry)), the spawn edges are the
//! **routing substrate** the kernel maintains as sessions spawn: the system reducer walks a reducer's
//! [`ancestors`](ReducerGraph::ancestors) to assemble a handler chain across generations — own segment
//! first, then parent's, then grandparents' — and to reason about authority down the tree, since a child's
//! effects pass through its ancestors' middleware (§5).
//!
//! It holds only the **active set** — the reducers currently alive. A node is added when it spawns and
//! removed when it terminates ([`remove`](ReducerGraph::remove), which drops every edge incident to it), so
//! the structure stays bounded to what is running. Removal is active-set cleanup, not a mutation of the
//! historical record: the immutable spawn link a session was born with lives in its log (§7), retained
//! after it ends; this just stops tracking it.
//!
//! Tracking the graph is the running system's job, so [`ReducerGraph`] is an **async trait** shared behind
//! an `Arc`: an in-memory build answers from a local map ([`InMemoryReducerGraph`]); a distributed build
//! answers from a replicated structure. The queries are async for the same reason the rest of the system's
//! operations are — a replicated read awaits — and the mutators take `&self` (interior mutability), because
//! the system records an edge as each reducer spawns and drops a node when it ends, concurrently with
//! reads. Queries return owned `Vec`s (not borrowing iterators) so they cross the async trait boundary.

use crate::{Hash, ReducerId};
use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

/// The type of an edge in the reducer graph — a relationship label identified by hash, so a new kind of
/// relationship is a new hash rather than a change to the graph. Unlike a `ContractId` (the hash of a
/// schema), an edge kind is a pure nominal label — it names a relationship and carries no interaction
/// type — so a well-known kind is the hash of its name. It is a distinct type from the other hash-shaped
/// ids, so an edge kind can never be mistaken for a [`ReducerId`] even though both are `Hash`-wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeKind(Hash);

impl EdgeKind {
    /// An edge kind from a raw hash — for a consumer minting its own relationship over the graph.
    #[must_use]
    pub fn from_hash(hash: Hash) -> Self {
        Self(hash)
    }

    /// The underlying hash.
    #[must_use]
    pub fn hash(&self) -> Hash {
        self.0
    }

    /// The spawn-tree edge, `child -> parent` (§7). The system links one as each reducer spawns. It is
    /// **functional** (a reducer has at most one spawn out-edge — its parent) and **acyclic** (a spawn only
    /// ever adds a fresh child under a live parent), so the spawn edges form a forest whose roots are the
    /// reducers with no spawn out-edge. The [`parent`](ReducerGraph::parent) /
    /// [`children`](ReducerGraph::children) / [`ancestors`](ReducerGraph::ancestors) conveniences read this
    /// kind.
    #[must_use]
    pub fn spawn() -> Self {
        Self(Hash::of(b"cdz-platform.edge.spawn"))
    }

    /// The supervision edge, `watcher -> watched` (§7). A one-way subscription: it asks for the watched
    /// reducer's lifecycle events to be delivered to the watcher when it terminates. Free many-to-many, and
    /// need not follow the spawn tree — the two directions of a parent/child supervision link are two
    /// independent edges of this kind. The [`watchers`](ReducerGraph::watchers) convenience reads it.
    #[must_use]
    pub fn watch_exit() -> Self {
        Self(Hash::of(b"cdz-platform.edge.watch-exit"))
    }
}

/// A direction to read a node's edges of a kind: its **out**-edges (`node -> others`) or its **in**-edges
/// (`others -> node`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Edges leaving the node (`node -> other`).
    Out,
    /// Edges entering the node (`other -> node`).
    In,
}

/// A directed graph over the active reducer set with edges labelled by [`EdgeKind`]. The five core
/// operations (`insert` / `link` / `remove` / `contains` / `neighbors`) are what a backend implements;
/// [`reach`](Self::reach) and the spawn-tree conveniences are derived over them.
#[async_trait]
pub trait ReducerGraph: Send + Sync {
    /// Add `node` to the active set with no edges, returning `false` if it is already present.
    async fn insert(&self, node: ReducerId) -> bool;

    /// Add a `kind` edge `from -> to`, returning whether it was newly added. Both endpoints must already be
    /// in the active set; returns `false` if either is absent or the edge already exists.
    async fn link(&self, from: ReducerId, to: ReducerId, kind: EdgeKind) -> bool;

    /// Remove `node` from the active set, dropping every edge incident to it — in either direction and of
    /// any kind. Returns whether it was present. Unlike a spawn-tree teardown, this imposes no ordering: a
    /// node with children is removed too, and those children lose their spawn out-edge to it (they become
    /// roots); a consumer that wants bottom-up teardown sequences its own [`remove`](Self::remove) calls.
    async fn remove(&self, node: ReducerId) -> bool;

    /// Whether `node` is in the active set.
    async fn contains(&self, node: ReducerId) -> bool;

    /// The direct neighbours of `node` along `kind` edges in direction `dir`, in ascending id order. Empty
    /// for an unknown node or one with no such edges.
    async fn neighbors(&self, node: ReducerId, kind: EdgeKind, dir: Dir) -> Vec<ReducerId>;

    /// The nodes reachable from `node` by following `kind` edges in direction `dir`, nearest first and not
    /// including `node` itself. A breadth-first walk carrying a visited set, so a cycle — which a
    /// well-formed spawn tree never has — cannot make it loop. The default walks with repeated
    /// [`neighbors`](Self::neighbors) calls; a backend able to answer a transitive query in one round-trip
    /// may override it.
    async fn reach(&self, node: ReducerId, kind: EdgeKind, dir: Dir) -> Vec<ReducerId> {
        let mut seen: HashSet<ReducerId> = HashSet::new();
        seen.insert(node);
        let mut out = Vec::new();
        let mut frontier = self.neighbors(node, kind, dir).await;
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for n in frontier {
                if seen.insert(n) {
                    out.push(n);
                    next.extend(self.neighbors(n, kind, dir).await);
                }
            }
            frontier = next;
        }
        out
    }

    /// The parent of `reducer` in the spawn tree — its single [`spawn`](EdgeKind::spawn) out-edge — or
    /// `None` if it is a root (no spawn out-edge) or not in the active set.
    async fn parent(&self, reducer: ReducerId) -> Option<ReducerId> {
        self.neighbors(reducer, EdgeKind::spawn(), Dir::Out)
            .await
            .into_iter()
            .next()
    }

    /// The live children of `reducer` — the reducers whose spawn out-edge points at it — in ascending id
    /// order. Empty for a leaf or an unknown reducer.
    async fn children(&self, reducer: ReducerId) -> Vec<ReducerId> {
        self.neighbors(reducer, EdgeKind::spawn(), Dir::In).await
    }

    /// The ancestors of `reducer`, nearest first: its parent, then grandparent, up to and including the
    /// root — the spawn chain the system reducer walks to build a handler chain across generations (§3).
    /// Empty for a root or an unknown reducer.
    async fn ancestors(&self, reducer: ReducerId) -> Vec<ReducerId> {
        self.reach(reducer, EdgeKind::spawn(), Dir::Out).await
    }

    /// Whether `ancestor` lies on the spawn path from `of` up to the root — the authority relation the
    /// spawn tree encodes (an ancestor's middleware governs everything a descendant does, §5).
    async fn is_ancestor(&self, ancestor: ReducerId, of: ReducerId) -> bool {
        self.ancestors(of).await.contains(&ancestor)
    }

    /// The reducers watching `reducer` for its exit — the in-edges of the
    /// [`watch_exit`](EdgeKind::watch_exit) kind, in ascending id order. The system reads these when
    /// `reducer` terminates, to deliver its lifecycle event to each. Empty for an unwatched or unknown
    /// reducer.
    async fn watchers(&self, reducer: ReducerId) -> Vec<ReducerId> {
        self.neighbors(reducer, EdgeKind::watch_exit(), Dir::In)
            .await
    }
}

/// One node in the active graph: its edges grouped by kind, in each direction. A `BTreeSet` per
/// `(kind, direction)` keeps neighbours in ascending id order and deduplicates.
#[derive(Debug, Default)]
struct Node {
    /// Edges leaving this node: for each kind, the nodes it points to.
    out: HashMap<EdgeKind, BTreeSet<ReducerId>>,
    /// Edges entering this node: for each kind, the nodes that point to it.
    into: HashMap<EdgeKind, BTreeSet<ReducerId>>,
}

/// An in-memory [`ReducerGraph`] — the active graph as a local adjacency map. For tests and single-process
/// use; a distributed build tracks the same edges in a replicated structure. Interior mutability (a
/// `Mutex`), since the system records and drops edges behind a shared `Arc`.
#[derive(Debug, Default)]
pub struct InMemoryReducerGraph {
    nodes: Mutex<HashMap<ReducerId, Node>>,
}

impl InMemoryReducerGraph {
    /// An empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ReducerGraph for InMemoryReducerGraph {
    async fn insert(&self, node: ReducerId) -> bool {
        let mut nodes = self.nodes.lock().expect("graph lock");
        if nodes.contains_key(&node) {
            return false;
        }
        nodes.insert(node, Node::default());
        true
    }

    async fn link(&self, from: ReducerId, to: ReducerId, kind: EdgeKind) -> bool {
        let mut nodes = self.nodes.lock().expect("graph lock");
        if !nodes.contains_key(&from) || !nodes.contains_key(&to) {
            return false;
        }
        let added = nodes
            .get_mut(&from)
            .expect("from present, just checked")
            .out
            .entry(kind)
            .or_default()
            .insert(to);
        if added {
            nodes
                .get_mut(&to)
                .expect("to present, just checked")
                .into
                .entry(kind)
                .or_default()
                .insert(from);
        }
        added
    }

    async fn remove(&self, node: ReducerId) -> bool {
        let mut nodes = self.nodes.lock().expect("graph lock");
        let Some(removed) = nodes.remove(&node) else {
            return false;
        };
        // Drop the reverse of every out-edge: for each `node -> to`, remove `node` from `to`'s in-edges.
        for (kind, tos) in &removed.out {
            for to in tos {
                if let Some(target) = nodes.get_mut(to)
                    && let Some(set) = target.into.get_mut(kind)
                {
                    set.remove(&node);
                }
            }
        }
        // Drop the reverse of every in-edge: for each `from -> node`, remove `node` from `from`'s out-edges.
        for (kind, froms) in &removed.into {
            for from in froms {
                if let Some(source) = nodes.get_mut(from)
                    && let Some(set) = source.out.get_mut(kind)
                {
                    set.remove(&node);
                }
            }
        }
        true
    }

    async fn contains(&self, node: ReducerId) -> bool {
        self.nodes.lock().expect("graph lock").contains_key(&node)
    }

    async fn neighbors(&self, node: ReducerId, kind: EdgeKind, dir: Dir) -> Vec<ReducerId> {
        let nodes = self.nodes.lock().expect("graph lock");
        let Some(n) = nodes.get(&node) else {
            return Vec::new();
        };
        let edges = match dir {
            Dir::Out => &n.out,
            Dir::In => &n.into,
        };
        edges
            .get(&kind)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Dir, EdgeKind, InMemoryReducerGraph, ReducerGraph};
    use crate::{Hash, ReducerId};

    // Distinct reducer ids.
    fn r(tag: &str) -> ReducerId {
        ReducerId::from_hash(Hash::of(tag.as_bytes()))
    }

    // Link a spawn edge `child -> parent`, the shape the system establishes at each spawn.
    async fn spawn(g: &InMemoryReducerGraph, child: ReducerId, parent: ReducerId) -> bool {
        g.link(child, parent, EdgeKind::spawn()).await
    }

    #[tokio::test]
    async fn a_root_has_no_parent_and_no_ancestors() {
        let g = InMemoryReducerGraph::new();
        assert!(g.insert(r("root")).await);
        assert!(g.contains(r("root")).await);
        assert_eq!(g.parent(r("root")).await, None);
        assert!(g.ancestors(r("root")).await.is_empty());
        // inserting the same node twice is refused.
        assert!(!g.insert(r("root")).await);
    }

    #[tokio::test]
    async fn a_link_needs_both_endpoints_and_is_idempotent() {
        let g = InMemoryReducerGraph::new();
        // neither endpoint present — refused.
        assert!(!spawn(&g, r("child"), r("ghost")).await);
        g.insert(r("root")).await;
        // child not present yet — refused.
        assert!(!spawn(&g, r("child"), r("root")).await);
        g.insert(r("child")).await;
        assert!(spawn(&g, r("child"), r("root")).await);
        // the same edge again is not newly added.
        assert!(!spawn(&g, r("child"), r("root")).await);
    }

    #[tokio::test]
    async fn spawn_edges_read_both_directions_as_parent_and_children() {
        let g = InMemoryReducerGraph::new();
        for id in ["root", "a", "b", "a1"] {
            g.insert(r(id)).await;
        }
        spawn(&g, r("a"), r("root")).await;
        spawn(&g, r("b"), r("root")).await;
        spawn(&g, r("a1"), r("a")).await;
        assert_eq!(g.parent(r("a1")).await, Some(r("a")));
        assert_eq!(g.parent(r("a")).await, Some(r("root")));
        // children come back in ascending id order and cover every child.
        let mut root_children = g.children(r("root")).await;
        let mut expected = vec![r("a"), r("b")];
        root_children.sort_unstable();
        expected.sort_unstable();
        assert_eq!(root_children, expected);
        assert_eq!(g.children(r("a")).await, vec![r("a1")]);
        assert!(
            g.children(r("a1")).await.is_empty(),
            "a leaf has no children"
        );
    }

    #[tokio::test]
    async fn ancestors_walk_the_spawn_chain_nearest_first() {
        let g = InMemoryReducerGraph::new();
        for id in ["root", "parent", "child"] {
            g.insert(r(id)).await;
        }
        spawn(&g, r("parent"), r("root")).await;
        spawn(&g, r("child"), r("parent")).await;
        assert_eq!(g.ancestors(r("child")).await, vec![r("parent"), r("root")]);
        assert!(g.is_ancestor(r("root"), r("child")).await);
        assert!(g.is_ancestor(r("parent"), r("child")).await);
        assert!(!g.is_ancestor(r("child"), r("root")).await);
    }

    #[tokio::test]
    async fn remove_drops_a_node_and_all_its_incident_edges() {
        let g = InMemoryReducerGraph::new();
        for id in ["root", "a", "a1"] {
            g.insert(r(id)).await;
        }
        spawn(&g, r("a"), r("root")).await;
        spawn(&g, r("a1"), r("a")).await;
        // Removing `a` — a non-leaf — succeeds and clears both its edges: root loses the child, and a1 is
        // left with no parent (the general graph imposes no bottom-up ordering).
        assert!(g.remove(r("a")).await);
        assert!(!g.contains(r("a")).await);
        assert!(g.children(r("root")).await.is_empty());
        assert_eq!(g.parent(r("a1")).await, None);
        // removing something absent is a no-op.
        assert!(!g.remove(r("a")).await);
    }

    #[tokio::test]
    async fn a_removed_node_can_be_reinserted() {
        let g = InMemoryReducerGraph::new();
        g.insert(r("root")).await;
        g.insert(r("c")).await;
        spawn(&g, r("c"), r("root")).await;
        assert!(g.remove(r("c")).await);
        assert!(g.insert(r("c")).await, "the slot was cleared");
        // reinserted with no edges — it must be linked afresh.
        assert_eq!(g.parent(r("c")).await, None);
    }

    #[tokio::test]
    async fn edges_of_different_kinds_are_independent() {
        // The generality: an ad-hoc edge kind a consumer mints coexists with `spawn` over the same nodes,
        // and neighbours are read per kind without crosstalk.
        let g = InMemoryReducerGraph::new();
        let watches = EdgeKind::from_hash(Hash::of(b"example.watches"));
        g.insert(r("a")).await;
        g.insert(r("b")).await;
        spawn(&g, r("a"), r("b")).await; // a -> b as a spawn edge
        assert!(g.link(r("b"), r("a"), watches).await); // b -> a as an unrelated kind
        // Each kind is read on its own; the spawn query never sees the `watches` edge and vice versa.
        assert_eq!(
            g.neighbors(r("a"), EdgeKind::spawn(), Dir::Out).await,
            vec![r("b")]
        );
        assert!(g.neighbors(r("a"), watches, Dir::Out).await.is_empty());
        assert_eq!(g.neighbors(r("a"), watches, Dir::In).await, vec![r("b")]);
        assert_eq!(g.neighbors(r("b"), watches, Dir::Out).await, vec![r("a")]);
    }

    #[tokio::test]
    async fn reach_terminates_on_a_cycle() {
        // `reach` carries a visited set, so even a malformed cyclic edge kind (which the spawn tree never
        // forms) cannot make it loop.
        let g = InMemoryReducerGraph::new();
        let loops = EdgeKind::from_hash(Hash::of(b"example.loops"));
        for id in ["x", "y", "z"] {
            g.insert(r(id)).await;
        }
        g.link(r("x"), r("y"), loops).await;
        g.link(r("y"), r("z"), loops).await;
        g.link(r("z"), r("x"), loops).await; // close the cycle
        let mut reached = g.reach(r("x"), loops, Dir::Out).await;
        reached.sort_unstable();
        let mut expected = vec![r("y"), r("z")];
        expected.sort_unstable();
        assert_eq!(reached, expected, "every other node once, and it returns");
    }

    #[tokio::test]
    async fn watch_exit_edges_are_read_by_watchers() {
        // The two directions of a parent/child supervision link are independent watch_exit edges, and
        // `watchers(x)` reads whoever subscribed to x's exit — regardless of the spawn direction.
        let g = InMemoryReducerGraph::new();
        for id in ["parent", "child"] {
            g.insert(r(id)).await;
        }
        spawn(&g, r("child"), r("parent")).await;
        // parent watches child: parent -> child on watch_exit.
        assert!(
            g.link(r("parent"), r("child"), EdgeKind::watch_exit())
                .await
        );
        // child watches parent: child -> parent on watch_exit (independent, opposite direction).
        assert!(
            g.link(r("child"), r("parent"), EdgeKind::watch_exit())
                .await
        );
        assert_eq!(g.watchers(r("child")).await, vec![r("parent")]);
        assert_eq!(g.watchers(r("parent")).await, vec![r("child")]);
        // the spawn edge is untouched by the watch links.
        assert_eq!(g.parent(r("child")).await, Some(r("parent")));
    }
}
