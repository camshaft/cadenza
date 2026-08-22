//! The wasm-runtime host (`design/cadenza-platform.md` §3) — behind the `host` feature, off by default.
//!
//! `wasmtime` instantiates a reducer component and drives it through the WIT world (`wit/world.wit`): the
//! host provides the imports — `state`, `blobs`, `identity`, and, for an event reducer, the `graph`,
//! `deliver`, and `program-of` reads — and calls the guest's `on-message`/`on-response`/`on-notification`
//! exports. Every host import is async (`async: true` below), so a disk- or network-backed backend never
//! blocks the host thread while a reducer awaits it; the guest sees the calls as ordinary.
//!
//! This slice generates the host-side bindings for the (privileged) event-reducer world and confirms the WIT
//! ABI projects into valid wasmtime host bindings. Instantiating a component as a [`Reducer`](crate::Reducer)
//! and backing the imports over the in-memory [`KvStore`](crate::KvStore) / [`BlobStore`](crate::BlobStore)
//! is the following slice.
#![allow(dead_code)]

// Generated host bindings for the event-reducer world (the superset: the ordinary reducer imports plus the
// privileged `graph`/`deliver`/`provenance`). The ordinary reducer world is the same guest export with the
// privileged imports absent, so this projection covers both.
wasmtime::component::bindgen!({
    world: "event-reducer-world",
    path: "wit/world.wit",
    imports: { default: async },
    exports: { default: async },
});

use crate::{BlobStore, Bytes, EdgeKind, Hash, KvStore, ReducerGraph, ReducerId};
use std::sync::Arc;

/// A reducer-id or edge-kind crosses the WIT boundary as its raw hash bytes; a value that is not exactly
/// `Hash::LEN` bytes names nothing, so it converts to `None` and the graph op treats it as a miss.
fn to_reducer(bytes: &[u8]) -> Option<ReducerId> {
    Some(ReducerId::from_hash(Hash::from_bytes(
        <[u8; Hash::LEN]>::try_from(bytes).ok()?,
    )))
}
fn to_kind(bytes: &[u8]) -> Option<EdgeKind> {
    Some(EdgeKind::from_hash(Hash::from_bytes(
        <[u8; Hash::LEN]>::try_from(bytes).ok()?,
    )))
}
fn from_reducers(ids: Vec<ReducerId>) -> Vec<Vec<u8>> {
    ids.into_iter()
        .map(|id| id.hash().as_bytes().to_vec())
        .collect()
}

/// The host state threaded through a running reducer component's wasmtime store — what the host imports read
/// and write on the reducer's behalf. For now it carries the reducer's own id (the `identity` import) and the
/// content-addressed store (the `blobs` import); the key-value store and — for an event reducer — the
/// graph/deliver/provenance are added as those imports are implemented. (The `blobs` store is owned here for
/// now; wiring it to the one shared node-wide store is a later assembly step.)
struct HostState {
    /// This reducer's id (§3), returned by the `identity` import.
    id: ReducerId,
    /// The content-addressed store (§8), backing the `blobs` import.
    blobs: Box<dyn BlobStore>,
    /// The reducer's own key-value state (§7), backing the `state` import.
    kv: Box<dyn KvStore>,
    /// The one shared reducer graph (§3), backing the privileged `graph` import — an event reducer both reads
    /// and updates it to route and supervise. Shared (an `Arc`), since it is the node-wide routing substrate,
    /// not per-reducer; an ordinary reducer holds the handle but its linker never wires the `graph` import.
    graph: Arc<dyn ReducerGraph>,
}

impl cadenza::platform::identity::Host for HostState {
    async fn id(&mut self) -> Vec<u8> {
        self.id.hash().as_bytes().to_vec()
    }
}

impl cadenza::platform::blobs::Host for HostState {
    async fn get(&mut self, hash: Vec<u8>) -> Option<Vec<u8>> {
        // A malformed hash (not exactly `Hash::LEN` bytes) names nothing, so it reads back as absent.
        let hash = Hash::from_bytes(<[u8; Hash::LEN]>::try_from(hash.as_slice()).ok()?);
        self.blobs.get(hash).await.map(|bytes| bytes.to_vec())
    }

    async fn put(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        self.blobs.put(Bytes::from(bytes)).await.as_bytes().to_vec()
    }
}

impl cadenza::platform::state::Host for HostState {
    async fn get(&mut self, key: Vec<u8>) -> Option<Vec<u8>> {
        self.kv.get(&key).await.map(|value| value.to_vec())
    }

    async fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.kv.put(Bytes::from(key), Bytes::from(value)).await;
    }

    async fn delete(&mut self, key: Vec<u8>) {
        // The WIT `delete` reports nothing; the key-value store's whether-it-was-present is not surfaced.
        self.kv.delete(&key).await;
    }
}

impl cadenza::platform::graph::Host for HostState {
    async fn insert(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.insert(node).await,
            None => false,
        }
    }

    async fn contains(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.contains(node).await,
            None => false,
        }
    }

    async fn remove(&mut self, node: Vec<u8>) -> bool {
        match to_reducer(&node) {
            Some(node) => self.graph.remove(node).await,
            None => false,
        }
    }

    async fn link(&mut self, source: Vec<u8>, target: Vec<u8>, kind: Vec<u8>) -> bool {
        match (to_reducer(&source), to_reducer(&target), to_kind(&kind)) {
            (Some(source), Some(target), Some(kind)) => self.graph.link(source, target, kind).await,
            _ => false,
        }
    }

    async fn set_edges(
        &mut self,
        source: Vec<u8>,
        kind: Vec<u8>,
        targets: Vec<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        let (Some(source), Some(kind)) = (to_reducer(&source), to_kind(&kind)) else {
            return Vec::new();
        };
        // A malformed target names nothing, so it is dropped from the chain rather than aborting the set.
        let targets = targets.iter().filter_map(|t| to_reducer(t)).collect();
        from_reducers(self.graph.set_edges(source, kind, targets).await)
    }

    async fn neighbors(
        &mut self,
        node: Vec<u8>,
        kind: Vec<u8>,
        dir: cadenza::platform::graph::Dir,
    ) -> Vec<Vec<u8>> {
        let (Some(node), Some(kind)) = (to_reducer(&node), to_kind(&kind)) else {
            return Vec::new();
        };
        from_reducers(self.graph.neighbors(node, kind, dir.into()).await)
    }

    async fn in_kinds(&mut self, node: Vec<u8>) -> Vec<Vec<u8>> {
        match to_reducer(&node) {
            Some(node) => self
                .graph
                .in_kinds(node)
                .await
                .into_iter()
                .map(|kind| kind.hash().as_bytes().to_vec())
                .collect(),
            None => Vec::new(),
        }
    }

    async fn reach(
        &mut self,
        node: Vec<u8>,
        kind: Vec<u8>,
        dir: cadenza::platform::graph::Dir,
    ) -> Vec<Vec<u8>> {
        let (Some(node), Some(kind)) = (to_reducer(&node), to_kind(&kind)) else {
            return Vec::new();
        };
        from_reducers(self.graph.reach(node, kind, dir.into()).await)
    }
}

impl From<cadenza::platform::graph::Dir> for crate::Dir {
    fn from(dir: cadenza::platform::graph::Dir) -> Self {
        match dir {
            cadenza::platform::graph::Dir::Outgoing => crate::Dir::Out,
            cadenza::platform::graph::Dir::Incoming => crate::Dir::In,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HostState;
    // The `blobs` and `state` imports both have `get`/`put`, so use named trait aliases and fully-qualified
    // calls to disambiguate.
    use super::cadenza::platform::blobs::Host as Blobs;
    use super::cadenza::platform::graph::Dir;
    use super::cadenza::platform::graph::Host as Graph;
    use super::cadenza::platform::identity::Host as Identity;
    use super::cadenza::platform::state::Host as State;
    use crate::{
        Hash, HashTag, InMemoryBlobStore, InMemoryKvStore, InMemoryReducerGraph, ReducerId,
    };
    use std::sync::Arc;

    fn host(id: ReducerId) -> HostState {
        HostState {
            id,
            blobs: Box::new(InMemoryBlobStore::new()),
            kv: Box::new(InMemoryKvStore::new()),
            graph: Arc::new(InMemoryReducerGraph::new()),
        }
    }

    /// The raw hash bytes of a reducer-id / edge-kind, as they cross the WIT boundary.
    fn rid_bytes(tag: &[u8]) -> Vec<u8> {
        ReducerId::of(tag).hash().as_bytes().to_vec()
    }
    fn kind_bytes(tag: &[u8]) -> Vec<u8> {
        Hash::of(HashTag::SystemProperty, tag).as_bytes().to_vec()
    }

    #[tokio::test]
    async fn identity_returns_the_reducers_own_id() {
        // The `identity` host import hands the guest its own reducer-id, as the id's raw hash bytes.
        let id = ReducerId::of(b"me");
        let mut host = host(id);
        assert_eq!(Identity::id(&mut host).await, id.hash().as_bytes().to_vec());
    }

    #[tokio::test]
    async fn blobs_round_trip_and_a_malformed_hash_is_absent() {
        let mut host = host(ReducerId::of(b"me"));
        // `put` stores the bytes and returns their content hash; `get` reads them back by that hash.
        let hash = Blobs::put(&mut host, b"a blob".to_vec()).await;
        assert_eq!(
            Blobs::get(&mut host, hash).await.as_deref(),
            Some(b"a blob".as_slice())
        );
        // A hash the store does not hold reads back as absent, and so does a malformed (wrong-length) hash.
        assert_eq!(
            Blobs::get(&mut host, b"not a real hash".to_vec()).await,
            None
        );
    }

    #[tokio::test]
    async fn state_get_put_delete() {
        let mut host = host(ReducerId::of(b"me"));
        // Absent key reads back as nothing; put then get returns the value; delete removes it.
        assert_eq!(State::get(&mut host, b"k".to_vec()).await, None);
        State::put(&mut host, b"k".to_vec(), b"v".to_vec()).await;
        assert_eq!(
            State::get(&mut host, b"k".to_vec()).await.as_deref(),
            Some(b"v".as_slice())
        );
        State::delete(&mut host, b"k".to_vec()).await;
        assert_eq!(State::get(&mut host, b"k".to_vec()).await, None);
    }

    #[tokio::test]
    async fn graph_insert_link_and_read_back() {
        let mut host = host(ReducerId::of(b"me"));
        let (a, b, kind) = (rid_bytes(b"a"), rid_bytes(b"b"), kind_bytes(b"edge"));
        assert!(Graph::insert(&mut host, a.clone()).await);
        assert!(Graph::insert(&mut host, b.clone()).await);
        assert!(Graph::link(&mut host, a.clone(), b.clone(), kind.clone()).await);
        // `a`'s outgoing `kind` neighbours are `[b]`; `b`'s incoming are `[a]`.
        assert_eq!(
            Graph::neighbors(&mut host, a.clone(), kind.clone(), Dir::Outgoing).await,
            vec![b.clone()]
        );
        assert_eq!(
            Graph::neighbors(&mut host, b, kind, Dir::Incoming).await,
            vec![a]
        );
        // A malformed (wrong-length) node names nothing.
        assert!(!Graph::contains(&mut host, b"not a hash".to_vec()).await);
    }
}
