//! The wasmtime component host for wasm reducers (feature `wasm-reducer`, §19b).
//!
//! Operator directive §19b: the reducer boundary is the wasm COMPONENT MODEL, not a Rust trait later
//! mapped. This module binds `wit/reducer.wit` (the `cadenza:agent-kernel` reducer world) via
//! wasmtime's `bindgen!` and runs a reducer as a real wasm component — the guest EXPORTS `fold.apply`
//! and IMPORTS `kv` (which the host provides; §4b: a reducer reads its own KV as a direct non-effect
//! call). Log + KV stay HOST concerns (traits); this is their guest-facing wiring.
//!
//! This slice is the FOUNDATION: it generates the bindings and stands up the host-side `kv` import
//! backed by the kernel's [`crate::kv::Kv`], proving the world binds and the linker wires up. Actually
//! DRIVING a guest component through the fold loop (a `Reducer` impl over `apply`, with a compiled
//! guest fixture) is the next slice — kept separate so this one is small + gate-green. The in-process
//! Rust [`crate::reducer::Reducer`] trait remains the interim reducer path meanwhile.

#![cfg(feature = "wasm-reducer")]

use crate::kv::Kv;

// Generate host bindings from the reducer world. `bindgen!` reads the WIT package and produces the
// `Reducer` world type + the `kv`/`fold`/`types` interface glue. `path` is relative to CARGO_MANIFEST_DIR.
wasmtime::component::bindgen!({
    world: "reducer",
    path: "wit/reducer.wit",
});

// Re-export the generated agent-kernel type modules under clear names so the rest of the crate refers
// to `wasm_host::EffectRequest` etc. rather than the deep generated path.
pub use self::cadenza::agent_kernel::types::{ContentType, EffectKind, EffectRequest};

/// The host state a reducer component runs against: its session KV (the `kv` import is served from
/// here) plus room for the fold's output. One per fold invocation (the guest is stateless between
/// events — §4 — so the host owns the KV and hands the guest a view for the call).
pub struct ReducerHost {
    kv: Kv,
}

impl ReducerHost {
    pub fn new(kv: Kv) -> Self {
        ReducerHost { kv }
    }

    /// Take the (possibly mutated) KV back after a fold — the host persists it as the session's derived
    /// state (KV mutations are the deterministic side output of folding, §4).
    pub fn into_kv(self) -> Kv {
        self.kv
    }
}

// Host implementation of the `kv` import the guest calls DIRECTLY during a fold (§4b — NOT an effect).
// Backed by the kernel's persistent-map KV; keys/values are opaque bytes (the guest defines the schema).
impl self::cadenza::agent_kernel::kv::Host for ReducerHost {
    fn get(&mut self, key: Vec<u8>) -> Option<Vec<u8>> {
        self.kv.get(&key).map(|v| v.to_vec())
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.kv.put(key, value);
    }

    fn delete(&mut self, key: Vec<u8>) -> bool {
        self.kv.delete(&key)
    }

    fn prefix_scan(&mut self, prefix: Vec<u8>) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.kv
            .prefix_scan(&prefix)
            .into_iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The bindings compile and the host `kv` import is backed by the kernel KV: exercise it directly
    // (a guest-less unit test of the host side — the end-to-end guest fold is the next slice).
    #[test]
    fn host_kv_import_is_backed_by_the_kernel_kv() {
        use self::cadenza::agent_kernel::kv::Host;
        let mut host = ReducerHost::new(Kv::new());
        assert_eq!(host.get(b"k".to_vec()), None);
        host.put(b"k".to_vec(), b"v".to_vec());
        assert_eq!(host.get(b"k".to_vec()), Some(b"v".to_vec()));
        host.put(b"pending/1".to_vec(), b"a".to_vec());
        host.put(b"pending/2".to_vec(), b"b".to_vec());
        let scan = host.prefix_scan(b"pending/".to_vec());
        assert_eq!(
            scan,
            vec![
                (b"pending/1".to_vec(), b"a".to_vec()),
                (b"pending/2".to_vec(), b"b".to_vec()),
            ]
        );
        assert!(host.delete(b"k".to_vec()));
        assert_eq!(host.get(b"k".to_vec()), None);
        // KV comes back out for the host to persist as derived state.
        assert_eq!(host.into_kv().len(), 2);
    }
}
