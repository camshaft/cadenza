//! The wasmtime component host for wasm reducers (§19b).
//!
//! Operator directive §19b: the reducer boundary is the wasm COMPONENT MODEL, not a Rust trait later
//! mapped. This module binds `wit/reducer.wit` (the `cadenza:agent-kernel` reducer world) via
//! wasmtime's `bindgen!` and runs a reducer as a real wasm component — the guest EXPORTS `fold.apply`
//! and IMPORTS `kv` (which the host provides; §4b: a reducer reads its own KV as a direct non-effect
//! call). Log + KV stay HOST concerns (traits); this is their guest-facing wiring.
//!
//! Host surface: [`ReducerHost`] serves the `kv` import (backed by the kernel [`crate::kv::Kv`]) and
//! [`ComponentReducer`] builds a reducer from component bytes + drives a fold via [`ComponentReducer::apply`]
//! (instantiate → call the guest's `fold.apply` → return effects + mutated KV). What remains for
//! end-to-end use: a compiled guest FIXTURE that exports `fold.apply` (a wit-bindgen Rust guest,
//! concierge-ruled Option A) to run `apply` against, and wiring `apply` into the kernel's fold loop.
//! The in-process Rust [`crate::reducer::Reducer`] trait remains the interim reducer path meanwhile.

use crate::kv::Kv;

// Generate host bindings from the reducer world. `bindgen!` reads the WIT package and produces the
// `Reducer` world type + the `kv`/`fold`/`types` interface glue. `path` is relative to CARGO_MANIFEST_DIR.
wasmtime::component::bindgen!({
    world: "reducer",
    path: "wit/reducer.wit",
});

// Re-export the generated agent-kernel types under clear names so the rest of the crate refers to
// `wasm_host::EffectRequest` etc. rather than the deep generated path.
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

// The `types` interface defines only data types (content-type/effect-kind/effect-request), no
// functions — but bindgen still generates a marker `Host` trait for it that the host must implement
// (empty). Required because the `kv`/`fold` interfaces `use types.*`.
impl self::cadenza::agent_kernel::types::Host for ReducerHost {}

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

/// A reducer backed by a wasm COMPONENT bound to the `cadenza:agent-kernel` reducer world (§19b). Holds
/// the wasmtime `Engine` + the compiled `Component` + a `Linker` with the host `kv` import registered;
/// each fold instantiates the component fresh (the guest is stateless between events — §4 — and the KV
/// state lives host-side, threaded in per call). This is the component-model path that will REPLACE the
/// in-process Rust [`crate::reducer::Reducer`] trait. `apply` (below) drives a fold; what remains for
/// end-to-end use is a compiled guest FIXTURE that exports `fold.apply` (a wit-bindgen Rust guest —
/// concierge-ruled Option A) + wiring `apply` into the kernel's fold loop. Until that fixture lands,
/// `apply` is exercised only against a guest in tests (next slice); the Rust `Reducer` trait stays the
/// working path meanwhile.
pub struct ComponentReducer {
    engine: wasmtime::Engine,
    // The instantiation inputs `apply` reads each fold: instantiate `component` against `linker` (which
    // carries the `kv` host import) into a fresh Store, then call the guest's `fold.apply`.
    component: wasmtime::component::Component,
    linker: wasmtime::component::Linker<ReducerHost>,
}

/// Errors constructing or running a component reducer. Kept small; grows as the fold path lands.
#[derive(Debug)]
pub enum ComponentError {
    /// The bytes aren't a valid component for the reducer world.
    InvalidComponent(String),
    /// Host-import linking failed (the `kv` import couldn't be registered).
    Link(String),
    /// Instantiating the component against the host failed.
    Instantiate(String),
    /// The guest trapped during `fold.apply` (a totality-contract violation — §16c gap A: the driver
    /// treats this as a fold failure, distinct from a clean empty-effect return).
    Trap(String),
}

impl ComponentReducer {
    /// Build a component reducer from a compiled component's bytes. Sets up the `Engine`, compiles the
    /// `Component`, and registers the host `kv` import on the `Linker` (via the bindgen-generated
    /// `Reducer::add_to_linker`) so an instantiated guest can call its own KV directly (§4b). Does NOT
    /// instantiate yet — instantiation is per-fold (stateless guest).
    pub fn from_component_bytes(bytes: &[u8]) -> Result<Self, ComponentError> {
        let engine = wasmtime::Engine::default();
        let component = wasmtime::component::Component::new(&engine, bytes)
            .map_err(|e| ComponentError::InvalidComponent(e.to_string()))?;
        let mut linker = wasmtime::component::Linker::<ReducerHost>::new(&engine);
        // Register the `kv` host import (the guest's direct-read surface, §4b). `add_to_linker` is
        // generated by bindgen! for the world's imports; the store data type IS `ReducerHost` (which
        // implements `kv::Host`), so `HasSelf` maps the store to itself (wasmtime 37 HasData form).
        Reducer::add_to_linker::<_, wasmtime::component::HasSelf<ReducerHost>>(
            &mut linker,
            |h: &mut ReducerHost| h,
        )
        .map_err(|e| ComponentError::Link(e.to_string()))?;
        Ok(ComponentReducer {
            engine,
            component,
            linker,
        })
    }

    /// The engine (exposed for tests / advanced host composition).
    pub fn engine(&self) -> &wasmtime::Engine {
        &self.engine
    }

    /// Fold ONE event through the wasm guest (§19b): instantiate the component against a fresh
    /// [`ReducerHost`] carrying `kv` (the guest reads it directly, §4b), call the guest's exported
    /// `fold.apply`, and return the requested effects paired with the (possibly-mutated) KV for the
    /// host to persist as derived state (§4 — KV mutations are the fold's deterministic side output).
    /// The guest is instantiated fresh per fold (stateless between events — §4).
    ///
    /// `resumes` is the guest's OWN continuation token, echoed verbatim from the originating effect's
    /// `correlation` for a result/timer event (the single resume mechanism — operator design review;
    /// the guest never sees the kernel's internal effect id). Errors on instantiation failure or a
    /// guest trap (totality is the guest's contract, but a trap is surfaced as a fold failure the
    /// driver handles — §16c gap A).
    pub fn apply(
        &self,
        kv: Kv,
        content_type: ContentType,
        payload: Option<Vec<u8>>,
        resumes: Option<Vec<u8>>,
    ) -> Result<(Vec<EffectRequest>, Kv), ComponentError> {
        let mut store = wasmtime::Store::new(&self.engine, ReducerHost::new(kv));
        let instance = Reducer::instantiate(&mut store, &self.component, &self.linker)
            .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
        let effects = instance
            .cadenza_agent_kernel_fold()
            .call_apply(
                &mut store,
                &content_type,
                payload.as_deref(),
                resumes.as_deref(),
            )
            .map_err(|e| ComponentError::Trap(e.to_string()))?;
        let kv = store.into_data().into_kv();
        Ok((effects, kv))
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

    // The ComponentReducer CONSTRUCTION path wires up (Engine + Component + Linker + the generated
    // kv-import registration) on a real — if trivial — component. The `apply` fold + a guest that
    // exports fold.apply is the next slice (pending the guest-fixture toolchain decision).
    #[test]
    fn component_reducer_builds_engine_component_and_registers_kv_import() {
        // A valid, minimal component (exports nothing — enough to prove Component::new + the linker's
        // kv-import registration succeed; a fold-exporting guest is the next slice).
        let bytes = wat::parse_str("(component)").expect("assemble empty component");
        // (ComponentReducer holds wasmtime types that aren't Debug, so match rather than .expect().)
        let reducer = match ComponentReducer::from_component_bytes(&bytes) {
            Ok(r) => r,
            Err(e) => panic!("engine+component+linker with kv import should register: {e:?}"),
        };
        // Engine is live (sanity that construction produced a usable host).
        let _ = reducer.engine();
    }

    #[test]
    fn component_reducer_rejects_invalid_bytes() {
        // (Ok variant isn't Debug, so match rather than .unwrap_err().)
        match ComponentReducer::from_component_bytes(b"not a wasm component") {
            Err(ComponentError::InvalidComponent(_)) => {}
            Err(other) => panic!("expected InvalidComponent, got {other:?}"),
            Ok(_) => panic!("garbage bytes must not build a component"),
        }
    }
}
