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

use crate::{BlobStore, Bytes, Hash, KvStore, ReducerId};

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

#[cfg(test)]
mod tests {
    use super::HostState;
    // The `blobs` and `state` imports both have `get`/`put`, so use named trait aliases and fully-qualified
    // calls to disambiguate.
    use super::cadenza::platform::blobs::Host as Blobs;
    use super::cadenza::platform::identity::Host as Identity;
    use super::cadenza::platform::state::Host as State;
    use crate::{InMemoryBlobStore, InMemoryKvStore, ReducerId};

    fn host(id: ReducerId) -> HostState {
        HostState {
            id,
            blobs: Box::new(InMemoryBlobStore::new()),
            kv: Box::new(InMemoryKvStore::new()),
        }
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
}
