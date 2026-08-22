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

use crate::ReducerId;

/// The host state threaded through a running reducer component's wasmtime store — what the host imports read
/// and write on the reducer's behalf. For now it carries the reducer's own id (the `identity` import); the
/// key-value store, content-addressed store, and — for an event reducer — the graph/deliver/provenance are
/// added as those imports are implemented.
struct HostState {
    /// This reducer's id (§3), returned by the `identity` import.
    id: ReducerId,
}

impl cadenza::platform::identity::Host for HostState {
    async fn id(&mut self) -> Vec<u8> {
        self.id.hash().as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::HostState;
    use super::cadenza::platform::identity::Host as _;
    use crate::ReducerId;

    #[tokio::test]
    async fn identity_returns_the_reducers_own_id() {
        // The `identity` host import hands the guest its own reducer-id, as the id's raw hash bytes.
        let id = ReducerId::of(b"me");
        let mut host = HostState { id };
        assert_eq!(host.id().await, id.hash().as_bytes().to_vec());
    }
}
