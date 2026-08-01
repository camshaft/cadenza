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

use crate::hash::Hash;
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
    // The component dependencies this reducer declares by content hash (§23 — generic, NOT "the
    // runtime"). Detected at construction from the component's `+<hash>` imports; each must be resolved
    // from CAS and composed into the linker before `apply` can instantiate a reducer that has any.
    // Empty = a dependency-free reducer (e.g. the interim Rust guest). The linker-compose of resolved
    // dep bytes is the next slice; construction records the declared set so `apply` knows.
    deps: Vec<ComponentDep>,
    // The per-fold fuel budget (§22d): the hard instruction ceiling one `apply` may consume before the
    // guest is aborted with [`ComponentError::FuelExhausted`]. A runaway/looping reducer can't hang the
    // kernel (Copilot PR#1009 DoS gap). Enforced by wasmtime's fuel metering (engine `consume_fuel` +
    // `Store::set_fuel` per fold). This is the interim, sync first-step toward full gas (§22a); the
    // budget is uniform per fold today — per-session gas accounting arrives with the async substrate.
    fuel_budget: u64,
}

/// Default per-fold fuel ceiling (§22d). Chosen generous enough that a legitimate reducer fold (a bit
/// of KV work + assembling a handful of effect-requests) never approaches it, but finite so a runaway
/// guest is aborted rather than hanging the kernel. Tunable per reducer via
/// [`ComponentReducer::with_fuel_budget`]; the real per-session budget lands with gas (§22a).
pub const DEFAULT_FOLD_FUEL: u64 = 1_000_000_000;

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
    /// The guest exhausted its fuel budget mid-fold (§22d): a runaway/looping reducer hit the hard
    /// instruction ceiling before returning. Surfaced DISTINCTLY from a semantic [`ComponentError::Trap`]
    /// because it's a resource-exhaustion outcome (a real DoS vector — Copilot PR#1009), not a guest
    /// logic bug — the driver can act on it differently (e.g. quarantine the reducer, alert) and, once
    /// gas (§22a) lands, this is the signal a session's budget was consumed.
    FuelExhausted { budget: u64 },
    /// A declared component dependency isn't present in the blob store (missing by hash). DISTINCT from
    /// [`ComponentError::DepStoreError`] (Copilot PR#1013 #3): "the store doesn't hold it" is a
    /// different, often-actionable condition (publish/replicate the dep) than "the store itself errored."
    DepMissing { hash: Hash },
    /// The blob store errored while resolving a declared dependency (I/O, corruption). DISTINCT from
    /// [`ComponentError::DepMissing`] — the dep may well exist; the store failed to serve it.
    DepStoreError { hash: Hash, source: String },
}

/// A component DEPENDENCY a reducer declares by content hash (operator §23 — the kernel is
/// RUNTIME-AGNOSTIC). Per component-abi.md (contract v3), a component names a dependency import with the
/// dependency's content address as semver build-metadata (`<iface>@<semver>+<hash>`). The kernel reads
/// the `+<hash>` off ANY such import — it has NO knowledge of what a given dependency IS (the Cadenza
/// value-heap runtime is just one more content-addressed component, not a built-in the kernel knows by
/// name). It resolves every declared dep from CAS and links it, uniformly. This REPLACES the old
/// runtime-specific `RuntimeReq`/`RUNTIME_IFACE` machinery (which hard-coded "the Cadenza runtime" —
/// exactly what the operator directed removing; also dissolves Copilot PR#1013 #1/#2, the first-of-many
/// + `heap2@` prefix-false-match, since there's no privileged prefix to match or pick "the first" of).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentDep {
    /// The verbatim import name the component declares — the linker MUST bind under exactly this.
    pub import_name: String,
    /// The dependency's content address (from the `+<hash>` build-metadata), for CAS lookup.
    pub hash: Hash,
}

/// The declared component dependencies of a component (§23): EVERY import whose name carries a
/// `+<hash>` content-address build-metadata is a declared dep, resolved generically — NOT a name-matched
/// "the runtime" (the kernel has zero knowledge of any specific dependency). Imports the host itself
/// satisfies (the `kv` host import) carry no `+<hash>`, so they're excluded by construction — the
/// distinction is "does this import name carry a `+<hash>`," never a name allow-list. Errors only if an
/// import LOOKS like a content-addressed dep (has a `+`) but its hash is malformed (a corrupt name).
fn declared_deps(
    component: &wasmtime::component::Component,
    engine: &wasmtime::Engine,
) -> Result<Vec<ComponentDep>, ComponentError> {
    let mut deps = Vec::new();
    for (name, _item) in component.component_type().imports(engine) {
        // A content-addressed dep names its hash as `+<hash>` build-metadata. Only such imports are
        // deps; a host-satisfied import (e.g. `kv`) has no `+<hash>` and is skipped.
        let Some((_iface, hash_hex)) = name.rsplit_once('+') else {
            continue;
        };
        let hash = parse_hash_hex(hash_hex).ok_or_else(|| {
            ComponentError::InvalidComponent(format!(
                "component dependency import {name:?} has a malformed content-address hash {hash_hex:?}"
            ))
        })?;
        deps.push(ComponentDep {
            import_name: name.to_string(),
            hash,
        });
    }
    Ok(deps)
}

/// Fetch one declared dependency's component bytes from a blob store by its content address (§23). The
/// missing-vs-store-error split (PR#1013 #3) lets a caller distinguish "publish the dep" from "the store
/// broke." The kernel can't run a reducer whose declared dep it can't resolve — surface it, don't run a
/// half-linked component.
fn resolve_dep_bytes(
    dep: &ComponentDep,
    blobs: &dyn crate::blob::BlobStore,
) -> Result<Vec<u8>, ComponentError> {
    match blobs.get(&dep.hash) {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(ComponentError::DepMissing { hash: dep.hash }),
        Err(e) => Err(ComponentError::DepStoreError {
            hash: dep.hash,
            source: e.to_string(),
        }),
    }
}

/// Parse 64 LOWERCASE-hex chars into a [`Hash`] (the `+<hash>` content-address build-metadata). `None`
/// on any non-hex / wrong-length / UPPERCASE input: content addresses are canonical lowercase hex
/// (Copilot PR#1013 #4 — `from_str_radix` accepts uppercase, which would admit two spellings of one
/// address; reject the non-canonical form).
fn parse_hash_hex(hex: &str) -> Option<Hash> {
    if hex.len() != 64 {
        return None;
    }
    // Reject uppercase explicitly (from_str_radix would accept it): canonical content addresses are
    // lowercase, and admitting `AB..` alongside `ab..` would let one blob have two keys.
    if hex.bytes().any(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(Hash::from_bytes(bytes))
}

impl ComponentReducer {
    /// Build a component reducer from a compiled component's bytes. Sets up the `Engine`, compiles the
    /// `Component`, and registers the host `kv` import on the `Linker` (via the bindgen-generated
    /// `Reducer::add_to_linker`) so an instantiated guest can call its own KV directly (§4b). Does NOT
    /// instantiate yet — instantiation is per-fold (stateless guest).
    pub fn from_component_bytes(bytes: &[u8]) -> Result<Self, ComponentError> {
        // Enable fuel metering on the engine (§22d): every instruction the guest executes consumes
        // fuel, so `apply` can cap a fold at a finite instruction budget and abort a runaway guest
        // (Copilot PR#1009 DoS gap) with `OutOfFuel` rather than hanging. The per-fold budget is
        // charged in `apply` via `Store::set_fuel`; here we just turn metering on.
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        let engine = wasmtime::Engine::new(&config)
            .map_err(|e| ComponentError::InvalidComponent(e.to_string()))?;
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
        // Detect the component's declared dependencies — generically, by their `+<hash>` imports (§23).
        // The kernel has NO knowledge of what any dep IS (the Cadenza runtime is just one such dep).
        // Composing resolved dep bytes into the linker is the next slice; here we record the declared set
        // so a caller can resolve their bytes from CAS (`resolve_deps`) and `apply` knows what to compose.
        let deps = declared_deps(&component, &engine)?;
        Ok(ComponentReducer {
            engine,
            component,
            linker,
            deps,
            fuel_budget: DEFAULT_FOLD_FUEL,
        })
    }

    /// Override the per-fold fuel budget (§22d). Use a smaller ceiling for untrusted/low-trust reducers
    /// or a larger one for a reducer with a legitimately heavier fold; the [`DEFAULT_FOLD_FUEL`] suits a
    /// typical fold. (When gas §22a lands this becomes per-session budget accounting rather than a
    /// uniform per-fold cap.)
    pub fn with_fuel_budget(mut self, fuel: u64) -> Self {
        self.fuel_budget = fuel;
        self
    }

    /// The per-fold fuel budget this reducer enforces (§22d).
    pub fn fuel_budget(&self) -> u64 {
        self.fuel_budget
    }

    /// The engine (exposed for tests / advanced host composition).
    pub fn engine(&self) -> &wasmtime::Engine {
        &self.engine
    }

    /// The component dependencies this reducer declares by content hash (§23). Empty = dependency-free
    /// (e.g. the interim Rust guest). A caller composes each by fetching its bytes from CAS (see
    /// [`ComponentReducer::resolve_deps`]). The kernel treats every dep identically — no dep is special.
    pub fn deps(&self) -> &[ComponentDep] {
        &self.deps
    }

    /// Resolve ALL of this reducer's declared dependency bytes from a blob store (§23), each paired with
    /// the import name the linker must bind it under. Empty vec if dependency-free. `Err(DepMissing)` if
    /// a declared dep isn't in the store, `Err(DepStoreError)` if the store itself errored (PR#1013 #3 —
    /// the two are distinct). Generic: it resolves the Cadenza runtime, if present, exactly as it
    /// resolves any other dep — the kernel never asks "is this the runtime?" (The linker-compose of these
    /// bytes — binding each under its `import_name` — is the next slice.)
    pub fn resolve_deps(
        &self,
        blobs: &dyn crate::blob::BlobStore,
    ) -> Result<Vec<(ComponentDep, Vec<u8>)>, ComponentError> {
        self.deps
            .iter()
            .map(|dep| resolve_dep_bytes(dep, blobs).map(|bytes| (dep.clone(), bytes)))
            .collect()
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
        // Fuel metering is enabled on the engine (§22d). Instantiation isn't the DoS surface — a
        // reactive fold guest's runaway risk is in its `fold.apply` body, not its (structure-bounded)
        // instantiation — so give instantiation ample headroom, then reset fuel to the per-fold budget
        // right before the call. That way the budget bounds the FOLD precisely, and an exhausted budget
        // is unambiguously the guest's fold looping (not load cost). set_fuel can't fail with metering
        // on, but surface any error rather than unwrap.
        store
            .set_fuel(u64::MAX)
            .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
        let instance = Reducer::instantiate(&mut store, &self.component, &self.linker)
            .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
        store
            .set_fuel(self.fuel_budget)
            .map_err(|e| ComponentError::Trap(e.to_string()))?;
        let effects = match instance.cadenza_agent_kernel_fold().call_apply(
            &mut store,
            &content_type,
            payload.as_deref(),
            resumes.as_deref(),
        ) {
            Ok(effects) => effects,
            Err(e) => {
                // Distinguish a runaway guest (fuel exhausted) from a semantic guest trap: a fold that
                // consumed its whole budget is a resource-exhaustion outcome the driver handles
                // differently (§22d / PR#1009). `Trap::OutOfFuel` is carried in the error chain.
                if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
                    return Err(ComponentError::FuelExhausted {
                        budget: self.fuel_budget,
                    });
                }
                return Err(ComponentError::Trap(e.to_string()));
            }
        };
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
        // Dependency-free reducer: no declared deps detected, and resolve_deps → empty regardless of the
        // blob store (§23 — nothing to compose).
        assert!(reducer.deps().is_empty());
        assert_eq!(
            reducer
                .resolve_deps(&crate::blob::MemBlobStore::new())
                .unwrap(),
            vec![]
        );
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

    #[test]
    fn parse_hash_hex_round_trips_and_rejects_bad_input() {
        let h = Hash::of(b"the dependency");
        assert_eq!(parse_hash_hex(&h.to_hex()), Some(h));
        assert_eq!(parse_hash_hex("tooshort"), None);
        assert_eq!(parse_hash_hex(&"z".repeat(64)), None); // right length, non-hex
                                                           // PR#1013 #4: UPPERCASE hex is rejected — content addresses are canonical lowercase, so the
                                                           // uppercase spelling of a valid hash must NOT parse (else one blob would have two keys).
        assert_eq!(parse_hash_hex(&h.to_hex().to_uppercase()), None);
    }

    #[test]
    fn dependency_free_component_declares_no_deps() {
        // The interim Rust guest (and this empty component) declare no content-addressed deps → empty
        // (§23: nothing to compose). Dep DETECTION on a real dep-importing component is exercised once a
        // real Cadenza reducer fixture exists (next slices).
        let engine = wasmtime::Engine::default();
        let bytes = wat::parse_str("(component)").expect("empty component");
        let component = wasmtime::component::Component::new(&engine, &bytes).unwrap();
        assert!(declared_deps(&component, &engine).unwrap().is_empty());
    }

    #[test]
    fn resolve_dep_bytes_fetches_from_cas_or_splits_missing_vs_store_error() {
        use crate::blob::{BlobStore, MemBlobStore};
        let mut blobs = MemBlobStore::new();
        let dep_bytes = b"pretend a content-addressed dependency component";
        let hash = blobs.put(dep_bytes).unwrap();
        let dep = ComponentDep {
            import_name: format!("cadenza:runtime/heap@0.0.0+{}", hash.to_hex()),
            hash,
        };
        // Present in CAS → fetched (the kernel resolves it generically — it's just a dep by hash).
        assert_eq!(resolve_dep_bytes(&dep, &blobs).unwrap(), dep_bytes.to_vec());
        // Absent → DepMissing (PR#1013 #3: distinct from a store error — "publish it" vs "store broke").
        let missing = ComponentDep {
            import_name: "some:iface/x@0.0.0+…".into(),
            hash: Hash::of(b"a dep never stored"),
        };
        match resolve_dep_bytes(&missing, &blobs) {
            Err(ComponentError::DepMissing { hash }) => assert_eq!(hash, missing.hash),
            other => panic!("expected DepMissing, got {other:?}"),
        }
    }
}
