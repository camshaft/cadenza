//! The reducer graph — the typed relationships the running platform maintains over its reducers
//! (`design/cadenza-platform.md` §3/§7).
//!
//! Every relationship between reducers the system reasons about is an edge in one directed graph over the
//! active reducer set, labelled by an [`EdgeKind`]. The spawn tree is the [`EdgeKind::spawn`] edges (each a
//! `child -> parent` link); a supervision subscription is a [`watch_exit`](EdgeKind::watch_exit) edge; and a
//! **handler chain** is a set of weighted `owner -> handler` edges whose kind *is the contract-id* it
//! answers ([`for_contract`](EdgeKind::for_contract)) — so the whole routing substrate is one structure, not
//! a spawn tree plus a separate registry. Other relationships — a capability grant, a federation trust link
//! — are simply more edge kinds over the same nodes. Because an edge kind is a hash, a new relationship is a
//! new hash a consumer mints, needing no change to this trait or its backends: the graph stores and queries
//! edges without knowing what any kind *means*, and the meaning lives with the consumer that mints the kind.
//! An edge carries a **weight**, so a kind whose edges are ordered (a handler chain) comes back in order,
//! while unweighted edges (the spawn tree, a subscription) order by id.
//!
//! This is the **routing substrate** the kernel maintains as sessions register handlers and spawn: the
//! system reducer [`resolve`](ReducerGraph::resolve)s a contract to the chain that answers it, and walks a
//! reducer's [`ancestors`](ReducerGraph::ancestors) to reason about authority down the tree, since a child's
//! effects pass through its ancestors' middleware (§5). The tag byte of a hash keeps the kinds apart: a
//! contract-tagged edge kind is a handler chain, so [`contracts_for`](ReducerGraph::contracts_for) reads a
//! reducer's contract-tagged in-edges without confusing them with the structural spawn/watch-exit kinds.
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

use crate::{ContractId, Hash, HashTag, ReducerId};
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
        Self(Hash::of(
            HashTag::SystemProperty,
            b"cdz-platform.edge.spawn",
        ))
    }

    /// The supervision edge, `watcher -> watched` (§7). A one-way subscription: it asks for the watched
    /// reducer's lifecycle events to be delivered to the watcher when it terminates. Free many-to-many, and
    /// need not follow the spawn tree — the two directions of a parent/child supervision link are two
    /// independent edges of this kind. The [`watchers`](ReducerGraph::watchers) convenience reads it.
    #[must_use]
    pub fn watch_exit() -> Self {
        Self(Hash::of(
            HashTag::SystemProperty,
            b"cdz-platform.edge.watch-exit",
        ))
    }

    /// The edge kind for a contract's handler chain: the contract-id *is* the edge kind. An `owner ->
    /// handler` edge of this kind, ordered by weight, is one link of the chain that answers `contract` for
    /// `owner` (§3/§4) — the handler chains and the spawn tree are the one routing substrate. Since a
    /// contract-id is [`Contract`](HashTag::Contract)-tagged, these kinds are distinguishable from the
    /// structural [`spawn`](Self::spawn) / [`watch_exit`](Self::watch_exit) kinds, which
    /// [`contracts_for`](ReducerGraph::contracts_for) relies on.
    #[must_use]
    pub fn for_contract(contract: ContractId) -> Self {
        Self(contract.hash())
    }
}

/// Read an edge kind back from the raw bytes of the hash it carries — how a kind arrives when a consumer's
/// self-minted relationship crosses a boundary as a byte slice (a WIT payload). Fails (a wrong-length slice
/// names no hash) with the same error as [`Hash::try_from`].
impl TryFrom<&[u8]> for EdgeKind {
    type Error = std::array::TryFromSliceError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self::from_hash(Hash::try_from(bytes)?))
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

/// A directed graph over the active reducer set with edges labelled by [`EdgeKind`]. The core operations
/// (`insert` / `link` / `set_edges` / `remove` / `contains` / `neighbors` / `in_kinds`) are what a backend
/// implements; [`reach`](Self::reach), the spawn-tree conveniences, and the handler-chain conveniences
/// ([`resolve`](Self::resolve) / [`set_chain`](Self::set_chain) / [`contracts_for`](Self::contracts_for))
/// are derived over them.
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

    /// The direct neighbours of `node` along `kind` edges in direction `dir`, in **weight then id** order —
    /// so a weighted chain comes back in its chain order, and unweighted edges (all weight 0) in ascending
    /// id order. Empty for an unknown node or one with no such edges.
    async fn neighbors(&self, node: ReducerId, kind: EdgeKind, dir: Dir) -> Vec<ReducerId>;

    /// Replace all `kind` out-edges of `from` with `targets`, in order — each `targets[i]` linked at weight
    /// `i`, so [`neighbors(from, kind, Out)`](Self::neighbors) returns them in exactly this order. Returns
    /// the prior ordered targets (empty if none). An empty `targets` clears the kind's edges, so a kind is
    /// either an ordered non-empty chain or absent — the atomic whole-chain replace `set-handler` needs
    /// (§7). `from` must be in the active set (else it is a no-op returning empty); a target not yet in the
    /// active set is still recorded as an out-edge, but gains no reverse edge until it joins.
    async fn set_edges(
        &self,
        from: ReducerId,
        kind: EdgeKind,
        targets: Vec<ReducerId>,
    ) -> Vec<ReducerId>;

    /// The distinct kinds of `node`'s in-edges, ascending — what relationships point *at* it.
    /// [`contracts_for`](Self::contracts_for) filters these to the contract-tagged ones.
    async fn in_kinds(&self, node: ReducerId) -> Vec<EdgeKind>;

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

    /// The handler chain that answers `contract` for `reducer` — its `owner -> handler`
    /// [`for_contract`](EdgeKind::for_contract) out-edges in chain order (§3/§4). Empty if `reducer`
    /// registers no handler for `contract`, which the system reducer reports as `MissingHandler`.
    async fn resolve(&self, reducer: ReducerId, contract: ContractId) -> Vec<ReducerId> {
        self.neighbors(reducer, EdgeKind::for_contract(contract), Dir::Out)
            .await
    }

    /// Install or replace the whole handler `chain` for `contract` on `reducer`, returning the prior chain
    /// (empty if none) — the `set-handler` effect (§7). An empty `chain` removes the registration.
    async fn set_chain(
        &self,
        reducer: ReducerId,
        contract: ContractId,
        chain: Vec<ReducerId>,
    ) -> Vec<ReducerId> {
        self.set_edges(reducer, EdgeKind::for_contract(contract), chain)
            .await
    }

    /// The contracts `reducer` fronts a handler for — the reverse lookup behind `list-handlers` (§7): its
    /// contract-tagged in-edge kinds, since a reducer in a chain for `contract` has an in-edge of kind
    /// [`for_contract(contract)`](EdgeKind::for_contract). Returns just the contract-ids (the surface a peer
    /// may see), never the chains behind them, ascending and deduplicated (a reducer wired into a contract
    /// more than once yields it once).
    async fn contracts_for(&self, reducer: ReducerId) -> Vec<ContractId> {
        self.in_kinds(reducer)
            .await
            .into_iter()
            .filter(|kind| kind.hash().tag() == Some(HashTag::Contract))
            .map(|kind| ContractId::from_hash(kind.hash()))
            .collect()
    }
}

/// One node in the active graph: its edges grouped by kind, in each direction. Each edge carries a `weight`,
/// and a `BTreeSet` of `(weight, peer)` per `(kind, direction)` keeps neighbours in weight-then-id order and
/// deduplicates. Unweighted edges (from [`link`](ReducerGraph::link)) use weight 0, so they order by id as
/// before; a chain (from [`set_edges`](ReducerGraph::set_edges)) uses positional weights, so it keeps its
/// order — the same reducer at two positions is two distinct `(weight, peer)` entries.
#[derive(Debug, Default)]
struct Node {
    /// Edges leaving this node: for each kind, the `(weight, target)` it points to.
    out: HashMap<EdgeKind, BTreeSet<(u32, ReducerId)>>,
    /// Edges entering this node: for each kind, the `(weight, source)` that point to it.
    into: HashMap<EdgeKind, BTreeSet<(u32, ReducerId)>>,
}

/// Remove `edge` from `map[kind]`, dropping the `kind` entry entirely if that empties it — so a kind key in
/// an edge map always implies a non-empty set, which [`ReducerGraph::in_kinds`] (and thus `contracts_for`)
/// relies on to not report a kind whose edges are all gone.
fn drop_edge(
    map: &mut HashMap<EdgeKind, BTreeSet<(u32, ReducerId)>>,
    kind: EdgeKind,
    edge: (u32, ReducerId),
) {
    if let Some(set) = map.get_mut(&kind) {
        set.remove(&edge);
        if set.is_empty() {
            map.remove(&kind);
        }
    }
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
        // An unweighted edge: weight 0, so `neighbors` orders it by id among its kind.
        let added = nodes
            .get_mut(&from)
            .expect("from present, just checked")
            .out
            .entry(kind)
            .or_default()
            .insert((0, to));
        if added {
            nodes
                .get_mut(&to)
                .expect("to present, just checked")
                .into
                .entry(kind)
                .or_default()
                .insert((0, from));
        }
        added
    }

    async fn remove(&self, node: ReducerId) -> bool {
        let mut nodes = self.nodes.lock().expect("graph lock");
        let Some(removed) = nodes.remove(&node) else {
            return false;
        };
        // Drop the reverse of every out-edge: for each `node -[w]-> to`, remove `(w, node)` from `to`'s
        // in-edges of that kind.
        for (kind, tos) in &removed.out {
            for (weight, to) in tos {
                if let Some(target) = nodes.get_mut(to) {
                    drop_edge(&mut target.into, *kind, (*weight, node));
                }
            }
        }
        // Drop the reverse of every in-edge: for each `from -[w]-> node`, remove `(w, node)` from `from`'s
        // out-edges of that kind.
        for (kind, froms) in &removed.into {
            for (weight, from) in froms {
                if let Some(source) = nodes.get_mut(from) {
                    drop_edge(&mut source.out, *kind, (*weight, node));
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
        // The set is ordered by `(weight, peer)`, so this yields weight-then-id order; drop the weight.
        edges
            .get(&kind)
            .map(|set| set.iter().map(|(_, peer)| *peer).collect())
            .unwrap_or_default()
    }

    async fn set_edges(
        &self,
        from: ReducerId,
        kind: EdgeKind,
        targets: Vec<ReducerId>,
    ) -> Vec<ReducerId> {
        let mut nodes = self.nodes.lock().expect("graph lock");
        if !nodes.contains_key(&from) {
            return Vec::new();
        }
        // Take the prior edges of this kind out of `from` (weight-ordered), and drop their reverse edges.
        let prior: Vec<(u32, ReducerId)> = nodes
            .get_mut(&from)
            .expect("from present, just checked")
            .out
            .remove(&kind)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        for (weight, target) in &prior {
            if let Some(t) = nodes.get_mut(target) {
                drop_edge(&mut t.into, kind, (*weight, from));
            }
        }
        // Link the new chain, each target at its position; a target present in the active set also gets the
        // reverse edge (so `contracts_for` and `remove` see it).
        for (i, target) in targets.iter().enumerate() {
            let weight = i as u32;
            nodes
                .get_mut(&from)
                .expect("from present, just checked")
                .out
                .entry(kind)
                .or_default()
                .insert((weight, *target));
            if let Some(t) = nodes.get_mut(target) {
                t.into.entry(kind).or_default().insert((weight, from));
            }
        }
        prior.into_iter().map(|(_, peer)| peer).collect()
    }

    async fn in_kinds(&self, node: ReducerId) -> Vec<EdgeKind> {
        let nodes = self.nodes.lock().expect("graph lock");
        nodes
            .get(&node)
            .map(|n| {
                let mut kinds: Vec<EdgeKind> = n.into.keys().copied().collect();
                kinds.sort_unstable();
                kinds
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Dir, EdgeKind, InMemoryReducerGraph, ReducerGraph};
    use crate::{ContractId, Hash, HashTag, ReducerId};

    // Distinct reducer ids.
    fn r(tag: &str) -> ReducerId {
        ReducerId::of(tag.as_bytes())
    }
    // Distinct contract-ids (each a valid Contract-tagged edge kind).
    fn c(tag: &str) -> ContractId {
        ContractId::of(tag.as_bytes())
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
        let watches = EdgeKind::from_hash(Hash::of(HashTag::SystemProperty, b"example.watches"));
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
        let loops = EdgeKind::from_hash(Hash::of(HashTag::SystemProperty, b"example.loops"));
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

    #[tokio::test]
    async fn set_chain_resolves_in_order_and_replace_returns_prior() {
        let g = InMemoryReducerGraph::new();
        for id in ["owner", "authz", "rate-limit", "edge"] {
            g.insert(r(id)).await;
        }
        let http = c("http.get");
        // A fresh chain: authz wraps rate-limit wraps the edge handler. Nothing was there before.
        assert!(
            g.set_chain(
                r("owner"),
                http,
                vec![r("authz"), r("rate-limit"), r("edge")]
            )
            .await
            .is_empty()
        );
        // resolve returns the chain in exactly the order it was set — the weights preserve it, not id order.
        assert_eq!(
            g.resolve(r("owner"), http).await,
            vec![r("authz"), r("rate-limit"), r("edge")]
        );
        // Replacing returns the prior chain and the new one wins entirely (no merge).
        let prior = g
            .set_chain(r("owner"), http, vec![r("authz"), r("edge")])
            .await;
        assert_eq!(prior, vec![r("authz"), r("rate-limit"), r("edge")]);
        assert_eq!(
            g.resolve(r("owner"), http).await,
            vec![r("authz"), r("edge")]
        );
        // An empty chain removes the registration, returning the prior chain; then resolve is empty.
        assert_eq!(
            g.set_chain(r("owner"), http, Vec::new()).await,
            vec![r("authz"), r("edge")]
        );
        assert!(g.resolve(r("owner"), http).await.is_empty());
        // A contract with no handler resolves empty.
        assert!(g.resolve(r("owner"), c("nobody-answers")).await.is_empty());
    }

    #[tokio::test]
    async fn contracts_for_reads_contract_tagged_in_edges_not_structural_ones() {
        let g = InMemoryReducerGraph::new();
        for id in ["owner", "authz", "edge"] {
            g.insert(r(id)).await;
        }
        // authz is middleware in two contracts' chains on the owner; edge only in one.
        g.set_chain(r("owner"), c("http.get"), vec![r("authz"), r("edge")])
            .await;
        g.set_chain(r("owner"), c("http.post"), vec![r("authz")])
            .await;
        // authz also sits under the owner in the spawn tree and is watched by it — structural in-edges that
        // must NOT show up as contracts.
        g.link(r("authz"), r("owner"), EdgeKind::spawn()).await;
        g.link(r("owner"), r("authz"), EdgeKind::watch_exit()).await;

        let mut contracts = g.contracts_for(r("authz")).await;
        contracts.sort_unstable();
        let mut expected = vec![c("http.get"), c("http.post")];
        expected.sort_unstable();
        assert_eq!(
            contracts, expected,
            "only the contract-tagged in-edge kinds"
        );
        // edge fronts only one contract; a reducer wired nowhere fronts none.
        assert_eq!(g.contracts_for(r("edge")).await, vec![c("http.get")]);
        assert!(g.contracts_for(r("owner")).await.is_empty());
    }

    #[tokio::test]
    async fn a_handler_can_appear_twice_in_one_chain() {
        let g = InMemoryReducerGraph::new();
        for id in ["owner", "authz", "mid"] {
            g.insert(r(id)).await;
        }
        let contract = c("loop.contract");
        // The same reducer at two positions is kept (distinct weights), and its contract still lists once.
        g.set_chain(r("owner"), contract, vec![r("authz"), r("mid"), r("authz")])
            .await;
        assert_eq!(
            g.resolve(r("owner"), contract).await,
            vec![r("authz"), r("mid"), r("authz")]
        );
        assert_eq!(g.contracts_for(r("authz")).await, vec![contract]);
    }

    #[tokio::test]
    async fn removing_a_handler_clears_it_from_every_chain_it_fronts() {
        let g = InMemoryReducerGraph::new();
        for id in ["a", "b", "authz", "edge"] {
            g.insert(r(id)).await;
        }
        // `authz` sits in two owners' chains; removing it must drop it from both, leaving the rest in order.
        g.set_chain(r("a"), c("http.get"), vec![r("authz"), r("edge")])
            .await;
        g.set_chain(r("b"), c("http.post"), vec![r("authz"), r("edge")])
            .await;
        assert!(g.remove(r("authz")).await);
        assert_eq!(g.resolve(r("a"), c("http.get")).await, vec![r("edge")]);
        assert_eq!(g.resolve(r("b"), c("http.post")).await, vec![r("edge")]);
        // and `edge` keeps its position — removal drops only the removed node's link, not the whole chain.
        assert_eq!(g.contracts_for(r("edge")).await.len(), 2);
    }

    #[tokio::test]
    async fn removing_a_chain_owner_drops_its_chains_from_the_reverse_lookup() {
        let g = InMemoryReducerGraph::new();
        for id in ["owner", "handler"] {
            g.insert(r(id)).await;
        }
        g.set_chain(r("owner"), c("http.get"), vec![r("handler")])
            .await;
        assert_eq!(g.contracts_for(r("handler")).await, vec![c("http.get")]);
        // Removing the owner drops its out-edges and the reverse edges on its handlers.
        assert!(g.remove(r("owner")).await);
        assert!(g.contracts_for(r("handler")).await.is_empty());
    }

    #[tokio::test]
    async fn a_contract_out_edge_does_not_shadow_a_spawn_parent() {
        // An owner has both a spawn parent and a handler chain; each kind is read on its own — resolving a
        // contract never returns the parent, and `parent` never returns a handler.
        let g = InMemoryReducerGraph::new();
        for id in ["owner", "parent", "handler"] {
            g.insert(r(id)).await;
        }
        g.link(r("owner"), r("parent"), EdgeKind::spawn()).await;
        g.set_chain(r("owner"), c("http.get"), vec![r("handler")])
            .await;
        assert_eq!(g.parent(r("owner")).await, Some(r("parent")));
        assert_eq!(
            g.resolve(r("owner"), c("http.get")).await,
            vec![r("handler")]
        );
    }

    #[tokio::test]
    async fn a_chain_may_reference_a_handler_not_yet_in_the_active_set() {
        // A chain records a forward edge to each id whether or not it is a live node yet, so `resolve`
        // returns the whole chain; a handler gains its reverse edge (so `contracts_for` sees it) only once
        // it is in the active set at set time.
        let g = InMemoryReducerGraph::new();
        g.insert(r("owner")).await;
        g.insert(r("live")).await;
        // `future` is not inserted.
        g.set_chain(r("owner"), c("http.get"), vec![r("live"), r("future")])
            .await;
        assert_eq!(
            g.resolve(r("owner"), c("http.get")).await,
            vec![r("live"), r("future")],
            "the full chain resolves, including the not-yet-live handler"
        );
        assert_eq!(g.contracts_for(r("live")).await, vec![c("http.get")]);
        assert!(
            g.contracts_for(r("future")).await.is_empty(),
            "no reverse edge for a handler that was not in the active set at set time"
        );
    }

    /// The graph answers under Cameron's Bach simulator as under tokio — its operations are await-only over
    /// an in-memory map, so the routing substrate drives unchanged on the deterministic simulator (the seam
    /// for replaying dispatch, which reads the spawn tree and handler chains from here).
    #[test]
    fn graph_drives_under_the_bach_simulator() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let g = InMemoryReducerGraph::new();
                g.insert(r("root")).await;
                g.insert(r("child")).await;
                g.link(r("child"), r("root"), EdgeKind::spawn()).await;
                assert_eq!(g.parent(r("child")).await, Some(r("root")));
                assert_eq!(g.children(r("root")).await, vec![r("child")]);
            }
            .group("reducer-graph")
            .primary()
            .spawn();
        });
    }
}
