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

use crate::event::{EffectOutcome, Event, EventBody};
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

// Generate host bindings for the AUTHORIZER world in its OWN module (a second `bindgen!` at module scope
// would clash on the generated `types`/world names with the reducer bindings above). A policy component
// EXPORTS `authorize` + imports nothing, so this world has no host trait to implement — the bindings are
// just the typed `Authorizer` world + its `AuthorizerPre` for pre-instantiation, plus the request/decision
// records.
mod authz_bindings {
    wasmtime::component::bindgen!({
        world: "authorizer-world",
        path: "wit/authorizer.wit",
    });
}

// Generate a SECOND set of reducer bindings with ASYNC lowering of the guest export, in its own module
// (the async `call_apply_async` + async `instantiate_async`). A bindgen with `async` config lowers the
// `fold.apply` EXPORT to an async call; `async_support(true)` is a per-ENGINE flag and a sync call panics
// on an async store (and vice versa), so the async path needs its OWN engine + its OWN generated
// `call_apply_async` — it can't reuse the sync bindings above. `only_imports: []` keeps the `kv` IMPORT
// methods SYNC (they're pure in-memory BTreeMap ops on `ReducerHost` — no reason to yield, and async'ing
// them would force the `kv::Host` impl async for nothing). The store data type is the SAME `ReducerHost`.
mod async_reducer_bindings {
    wasmtime::component::bindgen!({
        world: "reducer",
        path: "wit/reducer.wit",
        // Lower the guest EXPORT (`fold.apply`) async → generates `call_apply_async`. IMPORTS (`kv`)
        // stay SYNC (omitted → default sync): pure in-memory BTreeMap ops on `ReducerHost`, no yield.
        exports: { default: async },
    });
}

/// The host state a reducer component runs against: its session KV (the `kv` import is served from
/// here) plus room for the fold's output. One per fold invocation (the guest is stateless between
/// events — §4 — so the host owns the KV and hands the guest a view for the call).
///
/// TRANSACTIONAL (error-atomicity, PR#1076/#1150): the guest mutates its KV THROUGH this host via the
/// `kv.put`/`kv.delete` import, so a naive host that wrote straight to the base map would leave PARTIAL
/// mutations behind if the fold traps or exhausts its fuel mid-way. Instead every write is buffered in
/// an `overlay` and the `base` is NOT touched until [`ReducerHost::commit`]. A fold that succeeds
/// commits (mutations become the session's derived state, §4); a fold that fails is dropped WITHOUT
/// committing, so the base is byte-for-byte the pre-fold state — true all-or-nothing. And it's cheap:
/// only the WRITE-SET is buffered (O(writes)), never a full-KV clone (which would be O(KV size) per
/// fold — the perf trap PR#1076 flagged). Reads see the guest's own uncommitted writes (read-your-writes
/// within the fold): the overlay shadows the base.
pub struct ReducerHost {
    /// The committed KV, moved in at fold start and left UNTOUCHED until `commit` — so discarding the
    /// host (an errored fold) yields exactly the pre-fold state.
    base: Kv,
    /// Writes buffered during THIS fold: `Some(v)` = a put, `None` = a delete tombstone. Applied to
    /// `base` only on `commit`; discarded on error. `BTreeMap` so `prefix_scan`'s merge stays ordered.
    overlay: std::collections::BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

impl ReducerHost {
    pub fn new(kv: Kv) -> Self {
        ReducerHost {
            base: kv,
            overlay: std::collections::BTreeMap::new(),
        }
    }

    /// Commit the fold's buffered writes into the base KV. Called by [`ComponentReducer::apply`] ONLY on
    /// a successful fold — the transactional boundary. After this the overlay is empty and `base` carries
    /// the fold's mutations. NOT called on an errored fold (the writes are discarded → base untouched).
    pub fn commit(&mut self) {
        for (key, write) in std::mem::take(&mut self.overlay) {
            match write {
                Some(value) => self.base.put(key, value),
                None => {
                    self.base.delete(&key);
                }
            }
        }
    }

    /// Take the base KV back after a fold. If [`ReducerHost::commit`] ran (success path), this carries
    /// the fold's mutations; if not (error path), it's the pre-fold state verbatim — which is what makes
    /// a failed fold atomic. Uncommitted overlay writes are dropped here.
    pub fn into_kv(self) -> Kv {
        self.base
    }
}

// The `types` interface defines only data types (content-type/effect-kind/effect-request), no
// functions — but bindgen still generates a marker `Host` trait for it that the host must implement
// (empty). Required because the `kv`/`fold` interfaces `use types.*`.
impl self::cadenza::agent_kernel::types::Host for ReducerHost {}

// Host implementation of the `kv` import the guest calls DIRECTLY during a fold (§4b — NOT an effect).
// Backed by the kernel's persistent-map KV; keys/values are opaque bytes (the guest defines the schema).
// Reads/writes go through the transactional overlay (see [`ReducerHost`]): writes buffer, reads shadow.
impl self::cadenza::agent_kernel::kv::Host for ReducerHost {
    fn get(&mut self, key: Vec<u8>) -> Option<Vec<u8>> {
        // Overlay shadows base (read-your-writes): a buffered put wins, a buffered tombstone hides base.
        match self.overlay.get(&key) {
            Some(Some(value)) => Some(value.clone()),
            Some(None) => None,
            None => self.base.get(&key).map(|v| v.to_vec()),
        }
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.overlay.insert(key, Some(value));
    }

    fn delete(&mut self, key: Vec<u8>) -> bool {
        // Return whether the key existed BEFORE this delete (matching `Kv::delete`), reading through the
        // overlay, then record the tombstone.
        let existed = match self.overlay.get(&key) {
            Some(Some(_)) => true,
            Some(None) => false,
            None => self.base.get(&key).is_some(),
        };
        self.overlay.insert(key, None);
        existed
    }

    fn prefix_scan(&mut self, prefix: Vec<u8>) -> Vec<(Vec<u8>, Vec<u8>)> {
        // Merge base entries under the prefix with the overlay's buffered writes/tombstones under it, in
        // canonical key order (§16c-S8 determinism): start from base, then apply overlay so the guest
        // sees its own uncommitted writes.
        let mut merged: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = self
            .base
            .prefix_scan(&prefix)
            .into_iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect();
        for (key, write) in self
            .overlay
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
        {
            match write {
                Some(value) => {
                    merged.insert(key.clone(), value.clone());
                }
                None => {
                    merged.remove(key);
                }
            }
        }
        merged.into_iter().collect()
    }
}

// The ASYNC `bindgen!` module generates its OWN `kv::Host` / `types::Host` traits (distinct from the sync
// module's), so `ReducerHost` must implement THOSE too to serve the async reducer's `kv` import. The kv
// methods are pure in-memory ops (kept SYNC — imports weren't lowered async), so these DELEGATE to the sync
// `kv::Host` impl above rather than duplicate the overlay logic — one source of truth for the KV semantics.
impl async_reducer_bindings::cadenza::agent_kernel::types::Host for ReducerHost {}

impl async_reducer_bindings::cadenza::agent_kernel::kv::Host for ReducerHost {
    fn get(&mut self, key: Vec<u8>) -> Option<Vec<u8>> {
        self::cadenza::agent_kernel::kv::Host::get(self, key)
    }
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self::cadenza::agent_kernel::kv::Host::put(self, key, value)
    }
    fn delete(&mut self, key: Vec<u8>) -> bool {
        self::cadenza::agent_kernel::kv::Host::delete(self, key)
    }
    fn prefix_scan(&mut self, prefix: Vec<u8>) -> Vec<(Vec<u8>, Vec<u8>)> {
        self::cadenza::agent_kernel::kv::Host::prefix_scan(self, prefix)
    }
}

/// A reducer backed by a wasm COMPONENT bound to the `cadenza:agent-kernel` reducer world (§19b). Holds
/// the wasmtime `Engine` + the compiled `Component` + a `Linker` with the host `kv` import registered;
/// each fold instantiates the component fresh (the guest is stateless between events — §4 — and the KV
/// state lives host-side, threaded in per call). This is the component-model path that will REPLACE the
/// in-process Rust [`crate::reducer::Reducer`] trait. `apply` (below) drives a fold, and
/// `ComponentReducer` implements `Reducer` so a wasm guest folds on the SAME kernel loop as a Rust one
/// — exercised end-to-end against a committed wit-bindgen guest fixture (see `tests/
/// component_reducer_e2e.rs`; concierge-ruled Option A). The Rust `Reducer` trait stays a working
/// interim path alongside it. What remains for a reducer that declares component DEPENDENCIES is
/// composing their resolved bytes into the linker (§23 dep-compose — see `deps`/`resolve_deps`); a
/// dependency-free reducer (like the fixture) runs today.
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
    // Resolved dependency component bytes (§23), paired with the import name each satisfies, ready to
    // COMPOSE into the per-fold linker before instantiate (see `apply` + `compose_dep_into_linker`).
    // Populated by `with_resolved_deps` from `resolve_deps`' CAS lookup; EMPTY for a dependency-free
    // reducer (which instantiates directly against `self.linker`). Held as bytes (not live instances)
    // because a dep instance must live in the SAME per-fold `Store` as the consumer — so composition
    // happens per fold, from these bytes, not once at construction.
    resolved_deps: Vec<(String, Vec<u8>)>,
    // PRE-INSTANTIATION artifact for the dependency-FREE path (operator perf directive: don't re-do the
    // link/type-check work every fold). `Linker::instantiate` is `instantiate_pre(component)?.instantiate
    // (store)` — the `instantiate_pre` half (resolving + type-checking the linker against the component)
    // is IDENTICAL every fold since neither `component` nor `linker` changes, so we do it ONCE at
    // construction and each fold just calls `.instantiate(store)` on the cached `ReducerPre`. `None` for a
    // reducer with resolved deps (its linker is composed PER-FOLD in the shared store — the deps' instances
    // can't outlive a fold's store — so that path can't reuse a single pre-instantiation; it stays on the
    // per-fold `Reducer::instantiate`). True Instance-reuse across folds is unsafe (an `Instance` is bound
    // to its `Store`, and each fold needs a fresh `Store` for its `ReducerHost` KV) — caching the
    // `ReducerPre` is the safe, wasmtime-idiomatic form of the operator's "persist instances" intent.
    instance_pre: Option<ReducerPre<ReducerHost>>,
    // The per-fold fuel budget (§22d): the hard instruction ceiling one `apply` may consume before the
    // guest is aborted with [`ComponentError::FuelExhausted`]. A runaway/looping reducer can't hang the
    // kernel (Copilot PR#1009 DoS gap). Enforced by wasmtime's fuel metering (engine `consume_fuel` +
    // `Store::set_fuel` per fold). The budget is a uniform per-fold ceiling.
    fuel_budget: u64,
    // The component store used to resolve a dep's OWN transitive BARE imports (§23 leaves-first compose —
    // the value-heap runtime imports `cadenza:nfc/normalize`, resolved by name from this store's
    // `runtime.toml`). `None` = no transitive resolution (a dep must be a leaf); set via
    // [`ComponentReducer::with_component_store`] when the reducer's runtime dep itself has deps (the real
    // nix/CDZ_STORE path). Held so `compose_dep_into_linker` can recurse leaves-first per fold.
    component_store: Option<crate::component_store::ComponentStore>,
}

/// Default per-fold fuel ceiling (§22d). Chosen generous enough that a legitimate reducer fold (a bit
/// of KV work + assembling a handful of effect-requests) never approaches it, but finite so a runaway
/// guest is aborted rather than hanging the kernel. Tunable per reducer via
/// [`ComponentReducer::with_fuel_budget`]; the real per-session budget lands with gas (§22a).
pub const DEFAULT_FOLD_FUEL: u64 = 1_000_000_000;

/// Cap on the byte-buffer length [`HeapHandle::read_bytes`] will PRE-RESERVE for (reviewer, #2166 read-side
/// DoS). `bytes-len` returns a raw unbounded `u32` reported by the untrusted guest value-heap; feeding it
/// straight into `Vec::with_capacity` lets a bogus large length (up to `u32::MAX` ≈ 4 GiB) reserve up front
/// and abort the host via the alloc-error handler before the read loop can fail-loud. 1 MiB is generous for
/// a real inline payload/token (larger bodies are content-addressed blobs, not inline value-heap bytes)
/// while bounding the eager reservation; a truthful larger buffer still reads correctly (the Vec just grows
/// with amortized reallocations past the cap), and a bogus length Traps cheaply on the first missing `bytes-get`.
const MAX_PREALLOC_BYTES: u32 = 1 << 20;

/// Errors constructing or running a component reducer. Kept small; grows as the fold path lands.
#[derive(Debug)]
pub enum ComponentError {
    /// The bytes aren't a valid component for the reducer world.
    InvalidComponent(String),
    /// Host-import linking failed (the `kv` import couldn't be registered).
    Link(String),
    /// Engine/host setup or instantiating the component against the host failed. This is a
    /// platform/host-config condition (e.g. `Engine::new` with the given `Config` failed, or
    /// instantiation against the linker failed) — NOT a statement about the component bytes (that's
    /// [`ComponentError::InvalidComponent`]). A caller reads this as "the host couldn't run it," not
    /// "your component is malformed."
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
    /// Composing a resolved dependency component into the linker failed (§23): its bytes didn't compile,
    /// it didn't export the interface the consumer imports under that name, or the linker rejected the
    /// binding. DISTINCT from [`ComponentError::DepMissing`]/[`ComponentError::DepStoreError`] (those are about
    /// FETCHING the dep) — this is about WIRING a fetched dep, so a caller can tell "couldn't get the
    /// dep" from "the dep doesn't fit the import it's meant to satisfy."
    Compose { import_name: String, reason: String },
    /// A generic [`invoke_component`] call couldn't find the requested `interface#func` export, or the
    /// export's result wasn't the canonical artifact-set shape the invoke seam decodes. DISTINCT from
    /// [`ComponentError::Trap`] (the export exists + ran but trapped) and [`ComponentError::InvalidComponent`]
    /// (the bytes aren't a component at all): here the component is valid but doesn't expose the named
    /// export in the artifact-returning shape the generic multi-export invoke seam needs — an actionable
    /// "this component isn't invokable at `interface#func`" signal (operator invoke-ABI ruling seq 107/108).
    InvokeExport {
        interface: String,
        func: String,
        reason: String,
    },
}

/// One emitted ARTIFACT of a generic component invocation — the result unit of the operator's invoke
/// primitive (Slack seq 107/108, 2026-08-04): a single invocation (e.g. the compiler) emits a SET of
/// these, and a caller-supplied selector program (slice-2) routes each to its sink (session-response |
/// CAS). Mirrors rcdzc's `abi::Artifact` shape `{kind, name, bytes}` — but the kernel owns its OWN copy
/// (it does NOT depend on the compiler crate; a component's emitted artifacts are self-describing on the
/// wire, so the kernel decodes them without knowing what produced them). `kind` categorizes the artifact
/// (e.g. `"wasm"`, `"ast"`, `"diagnostics"`), `name` identifies it within the set, `bytes` is its opaque
/// content (itself AST-encoded where the artifact is a value — the wire format is the AST encoding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// What KIND of artifact this is (`"wasm"`, `"ast"`, `"diagnostics"`, …) — the selector program keys
    /// routing on this (and/or `name`). Opaque to the kernel: the producing component defines the vocab.
    pub kind: String,
    /// The artifact's NAME within the emitted set — distinguishes multiple artifacts of the same kind and
    /// gives the selector program a per-artifact handle + a default CAS name.
    pub name: String,
    /// The artifact's content bytes (AST-encoded where it's a value). Opaque to the invoke mechanism; a
    /// sink (session-response inline, or CAS `blob.put`) consumes them verbatim.
    pub bytes: Vec<u8>,
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

/// The BARE interface name of a content-addressed import — the import name with its `+<hash>`
/// build-metadata AND its `@<semver>` version stripped, e.g. `cadenza:runtime/heap@0.0.0+<hash>` →
/// `cadenza:runtime/heap` (and `cadenza:runtime/heap+<hash>` with NO `@` → `cadenza:runtime/heap` too).
///
/// Strip `+<hash>` FIRST (via `rsplit_once('+')`, exactly as [`declared_deps`] recognizes a dep), THEN the
/// `@<semver>` — so the runtime-dep SELECTION and the dep PARSER agree on the same bare name. A `split('@')`
/// alone under-matches the `+<hash>`-with-no-`@` form (the `+hash` would survive), which was the #2219 bug.
fn bare_iface_name(import_name: &str) -> &str {
    let no_hash = import_name
        .rsplit_once('+')
        .map_or(import_name, |(iface, _hash)| iface);
    // `split_once('@')` (only the first `@` matters), mirroring the `rsplit_once('+')` above — an
    // unsuffixed name maps to itself. (NOT `split('@').next().unwrap_or(...)`: `next()` is infallible so
    // the `unwrap_or` was dead code — #2237 review.)
    no_hash
        .split_once('@')
        .map_or(no_hash, |(iface, _ver)| iface)
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
async fn resolve_dep_bytes(
    dep: &ComponentDep,
    blobs: &dyn crate::blob::BlobStore,
) -> Result<Vec<u8>, ComponentError> {
    match blobs.get(&dep.hash).await {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(ComponentError::DepMissing { hash: dep.hash }),
        Err(e) => Err(ComponentError::DepStoreError {
            hash: dep.hash,
            source: e.to_string(),
        }),
    }
}

/// Read the func names a component exports under `iface_name` (an exported INSTANCE). Read off the
/// component TYPE (not a live instance) so a wrong-shape dep is caught with a clear message rather than
/// an opaque call-time trap. Sync + store-free — the piece SHARED verbatim by the sync compose helpers
/// AND their async twins (`*_async`), so the func-name discovery has ONE home. `None` = the component
/// doesn't export `iface_name` as an interface (the caller turns that into the actionable Compose error).
fn read_iface_func_names(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
    iface_name: &str,
) -> Option<Vec<String>> {
    use wasmtime::component::types::ComponentItem;
    component
        .component_type()
        .exports(engine)
        .find(|(n, _)| *n == iface_name)
        .and_then(|(_, item)| match item {
            ComponentItem::ComponentInstance(inst) => Some(
                inst.exports(engine)
                    .filter_map(|(fname, i)| {
                        matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string())
                    })
                    .collect(),
            ),
            _ => None,
        })
}

/// Compose a resolved dependency COMPONENT into `linker` so a consumer that imports `import_name` can
/// instantiate against it (§23 — the runtime-agnostic dep-compose the kernel does uniformly for EVERY
/// declared dep, the value-heap runtime being just one). Mirrors the sibling `cdz-run::run_with_peers`
/// composition: instantiate the dep in the SHARED `store`, read the func names off its exported
/// interface `import_name`, extract each `Func` from the instance, and forward it into `linker` under
/// `import_name` via a raw dynamic closure that calls the dep func + its `post_return`. It's runtime
/// LINKER composition (host-side wiring of one component's exports into another's imports), NOT
/// bytes-level pre-composition — so no wac/wasm-compose dependency. The dep runs in the same store as
/// the consumer, so a value handle it returns is intelligible to the consumer (component-abi.md §A
/// Runtime Value Crosses As An Opaque Handle). Errors (as [`ComponentError::Compose`]) if the dep bytes
/// don't compile, don't export `import_name` as an interface, or the linker rejects the binding.
///
/// `T` is the store data type (the reducer host); the dep's funcs don't touch it — they're pure
/// component-to-component calls forwarded verbatim — so this is generic over the store type.
///
/// Returns the dep's own [`Instance`](wasmtime::component::Instance) in `store` — the SAME instance whose
/// funcs were forwarded into `linker`. For the value-heap runtime dep this is what the host binds a
/// [`HeapHandle`] on (§19e fold-boundary rebind): the reducer's lowered `apply` calls the runtime through
/// the linker while the host marshals its `apply` args by driving the very SAME instance's heap ops, so
/// every handle indexes one shared heap. Callers that only need the linker wiring (the WIT-structural
/// path) ignore the returned instance.
fn compose_dep_into_linker<T: 'static>(
    engine: &wasmtime::Engine,
    store: &mut wasmtime::Store<T>,
    linker: &mut wasmtime::component::Linker<T>,
    import_name: &str,
    dep_bytes: &[u8],
    // The store to resolve a dep's OWN transitive bare imports from (§23 leaves-first compose, e.g. the
    // runtime's `cadenza:nfc/normalize`). `None` = treat the dep as a leaf (the dependency-free WAT test
    // stubs, and any caller that has no store): the dep instantiates against a fresh empty linker as before.
    store_provider: Option<&crate::component_store::ComponentStore>,
) -> Result<wasmtime::component::Instance, ComponentError> {
    use wasmtime::component::Component;
    let compose_err = |reason: String| ComponentError::Compose {
        import_name: import_name.to_string(),
        reason,
    };
    let dep = Component::new(engine, dep_bytes)
        .map_err(|e| compose_err(format!("dependency bytes are not a valid component: {e}")))?;
    // The consumer IMPORTS under the full content-addressed name (`cadenza:runtime/heap@0.0.0+<hash>`), but
    // the dep component EXPORTS the BARE interface name (`cadenza:runtime/heap`) — a real Cadenza reducer's
    // `+<hash>` import build-metadata is NOT part of the dep's export name (verified against a compiled
    // reducer_b1 + its runtime component). So look the dep's exported interface up by the bare name (strip
    // the `@version+hash` suffix), while still registering into the CONSUMER's linker under the full
    // `import_name` it imports (below). A bare name with no suffix is unchanged. (Earlier this used
    // `import_name` for both sides — it only worked because the WAT test stubs happened to export the same
    // full string; a real content-addressed dep exposed the mismatch.)
    let dep_iface_name = bare_iface_name(import_name);
    // The func names the dep exports under `dep_iface_name` (read off the TYPE — see `read_iface_func_names`).
    let func_names = read_iface_func_names(engine, &dep, dep_iface_name).ok_or_else(|| {
        compose_err(format!(
            "dependency does not export the interface {dep_iface_name:?} the consumer imports \
             (as {import_name:?})"
        ))
    })?;
    // Instantiate the dep in the shared store. A dep is NOT necessarily a leaf — the value-heap runtime
    // itself imports the BARE `cadenza:nfc/normalize` component (FINDING#23). So before instantiating,
    // TRANSITIVELY compose the dep's OWN bare (store-resolvable) imports into its linker, leaves-first
    // (nfc → runtime → reducer), mirroring cdz-run's `instantiate_runtime`/`compose_nfc_into_runtime_linker`.
    // With no store provided (the unit-test stubs, which import nothing), this is a no-op and the dep stays
    // treated as a leaf against a fresh empty linker.
    let mut dep_linker = wasmtime::component::Linker::<T>::new(engine);
    compose_transitive_bare_deps(engine, store, &mut dep_linker, &dep, store_provider)?;
    let dep_instance = dep_linker
        .instantiate(&mut *store, &dep)
        .map_err(|e| compose_err(format!("instantiating the dependency failed: {e}")))?;
    let iface_idx = dep_instance
        .get_export_index(&mut *store, None, dep_iface_name)
        .ok_or_else(|| {
            compose_err("dependency instance is missing its exported interface".into())
        })?;
    // Forward each dep func into the consumer's linker under the full `import_name` (the content-addressed
    // name the consumer imports — NOT the bare `dep_iface_name`).
    let mut iface = linker
        .instance(import_name)
        .map_err(|e| compose_err(format!("linker.instance({import_name:?}): {e}")))?;
    for fname in &func_names {
        let fidx = dep_instance
            .get_export_index(&mut *store, Some(&iface_idx), fname)
            .ok_or_else(|| compose_err(format!("dependency missing exported func {fname:?}")))?;
        let f = dep_instance
            .get_func(&mut *store, fidx)
            .ok_or_else(|| compose_err(format!("dependency export {fname:?} is not a func")))?;
        iface
            .func_new(fname, move |mut ctx, params, results| {
                f.call(&mut ctx, params, results)?;
                f.post_return(&mut ctx)?;
                Ok(())
            })
            .map_err(|e| compose_err(format!("binding dep func {fname:?} into the linker: {e}")))?;
    }
    Ok(dep_instance)
}

/// The BARE inter-runtime interface a runtime component imports as its own dependency (FINDING#23): the
/// value-heap runtime's world declares `import cadenza:nfc/normalize`, resolved from the store by the
/// manifest name `nfc`. The `runtime.toml` manifest key is the PACKAGE segment (between `:` and `/`) = `nfc`
/// — NOT the interface leaf after the last `/` (`normalize`).
const NFC_IFACE: &str = "cadenza:nfc/normalize";

/// Compose a dep's OWN transitive BARE imports into `dep_linker` before the dep is instantiated — the
/// leaves-first §23 walk (nfc → runtime → reducer), mirroring cdz-run's `compose_nfc_into_runtime_linker`.
///
/// A "bare import" is an interface import with NO `+<hash>` build-metadata that the host doesn't itself
/// serve — the runtime's `cadenza:nfc/normalize` is the one such today. It's resolved from the store BY
/// NAME — the PACKAGE segment (between `:` and `/`), `nfc`, NOT the interface leaf `normalize` — via
/// [`ComponentStore::get_by_manifest_name`] (`runtime.toml`'s `nfc = "<hash>"` → `<hash>.wasm`), NOT by a
/// `+<hash>` in the name (bare imports carry none). The
/// resolved component is itself a LEAF (nfc imports nothing) so it instantiates against a fresh empty
/// linker; its interface funcs are forwarded into `dep_linker` under the import name (verbatim as
/// [`compose_dep_into_linker`] forwards a dep). `None` store, or a dep with no bare store-resolvable
/// import, is a no-op (the dep is a leaf). Only the known `NFC_IFACE` is resolved; any OTHER bare import
/// is left for wasmtime to report as unsatisfied (we don't blindly store-resolve arbitrary names).
fn compose_transitive_bare_deps<T: 'static>(
    engine: &wasmtime::Engine,
    store: &mut wasmtime::Store<T>,
    dep_linker: &mut wasmtime::component::Linker<T>,
    dep: &wasmtime::component::Component,
    store_provider: Option<&crate::component_store::ComponentStore>,
) -> Result<(), ComponentError> {
    let compose_err = |reason: String| ComponentError::Compose {
        import_name: NFC_IFACE.to_string(),
        reason,
    };
    let Some((nfc, func_names)) = resolve_transitive_nfc(engine, dep, store_provider)? else {
        return Ok(()); // leaf dep — nothing to compose
    };
    // NFC is a LEAF (imports nothing) → instantiate against a fresh empty linker.
    let nfc_linker = wasmtime::component::Linker::<T>::new(engine);
    let nfc_instance = nfc_linker
        .instantiate(&mut *store, &nfc)
        .map_err(|e| compose_err(format!("instantiating the nfc component failed: {e}")))?;
    let nfc_idx = nfc_instance
        .get_export_index(&mut *store, None, NFC_IFACE)
        .ok_or_else(|| compose_err("nfc instance is missing its exported interface".into()))?;
    let mut iface = dep_linker
        .instance(NFC_IFACE)
        .map_err(|e| compose_err(format!("dep_linker.instance({NFC_IFACE:?}): {e}")))?;
    for fname in &func_names {
        let fidx = nfc_instance
            .get_export_index(&mut *store, Some(&nfc_idx), fname)
            .ok_or_else(|| compose_err(format!("nfc component missing exported func {fname:?}")))?;
        let f = nfc_instance
            .get_func(&mut *store, fidx)
            .ok_or_else(|| compose_err(format!("nfc export {fname:?} is not a func")))?;
        iface
            .func_new(fname, move |mut ctx, params, results| {
                f.call(&mut ctx, params, results)?;
                f.post_return(&mut ctx)?;
                Ok(())
            })
            .map_err(|e| {
                compose_err(format!(
                    "binding nfc func {fname:?} into the dep linker: {e}"
                ))
            })?;
    }
    Ok(())
}

/// Resolve the transitive NFC component a dep needs, if any — the store-only PREAMBLE shared by
/// [`compose_transitive_bare_deps`] (sync) and [`compose_transitive_bare_deps_async`]. Returns
/// `Ok(None)` when the dep is a LEAF (doesn't import `NFC_IFACE`) → nothing to compose. Returns the
/// compiled NFC [`Component`] + its exported func names otherwise. Touches no store handle, so it's
/// identical for both engines — only the INSTANTIATE differs downstream (sync `instantiate` vs
/// `instantiate_async`), which is the whole reason the async twin exists (#2256 async dep-compose).
fn resolve_transitive_nfc(
    engine: &wasmtime::Engine,
    dep: &wasmtime::component::Component,
    store_provider: Option<&crate::component_store::ComponentStore>,
) -> Result<Option<(wasmtime::component::Component, Vec<String>)>, ComponentError> {
    use wasmtime::component::Component;
    let imports_nfc = dep
        .component_type()
        .imports(engine)
        .any(|(name, _)| name == NFC_IFACE);
    if !imports_nfc {
        return Ok(None); // leaf dep (or an older runtime that imports nothing) — nothing to compose
    }
    let compose_err = |reason: String| ComponentError::Compose {
        import_name: NFC_IFACE.to_string(),
        reason,
    };
    let store_provider = store_provider.ok_or_else(|| {
        compose_err(
            "dependency imports cadenza:nfc/normalize but no component store was provided to resolve it \
             (the transitive nfc dep — pass a ComponentStore via CDZ_STORE)"
                .to_string(),
        )
    })?;
    // Resolve NFC from the store BY MANIFEST NAME (the `runtime.toml` PACKAGE segment `nfc`, not the
    // interface leaf `normalize`; bare imports carry no `+<hash>`, so name path not get_by_hash).
    let nfc_name = NFC_IFACE
        .split_once(':')
        .and_then(|(_ns, rest)| rest.split('/').next())
        .unwrap_or(NFC_IFACE);
    let nfc_bytes = store_provider.get_by_manifest_name(nfc_name).map_err(|e| {
        compose_err(format!(
            "resolving {NFC_IFACE} ({nfc_name}) from the store: {e:?}"
        ))
    })?;
    let nfc = Component::new(engine, &nfc_bytes).map_err(|e| {
        compose_err(format!(
            "nfc component bytes are not a valid component: {e}"
        ))
    })?;
    let func_names = read_iface_func_names(engine, &nfc, NFC_IFACE)
        .ok_or_else(|| compose_err(format!("nfc component does not export {NFC_IFACE}")))?;
    Ok(Some((nfc, func_names)))
}

/// ASYNC twin of [`compose_transitive_bare_deps`] — identical resolution + forwarding, but the inner NFC
/// instantiate uses `instantiate_async` and each forwarded func is bound with `func_new_async` (calling
/// `call_async`/`post_return_async`). On an `async_support` engine BOTH the sync `Linker::instantiate`
/// AND a forwarded sync `Func::call` PANIC ("must use async instantiation" / "must use call_async") —
/// so the async fold path (`AsyncComponentReducer::apply`) cannot reuse the sync composer (#2256 async
/// twin; the panic v-ah-host's live genesis E2E hit). The store data type `T` must be `Send` because an
/// async host func may be polled across an await point.
async fn compose_transitive_bare_deps_async<T: Send + 'static>(
    engine: &wasmtime::Engine,
    store: &mut wasmtime::Store<T>,
    dep_linker: &mut wasmtime::component::Linker<T>,
    dep: &wasmtime::component::Component,
    store_provider: Option<&crate::component_store::ComponentStore>,
) -> Result<(), ComponentError> {
    let compose_err = |reason: String| ComponentError::Compose {
        import_name: NFC_IFACE.to_string(),
        reason,
    };
    let Some((nfc, func_names)) = resolve_transitive_nfc(engine, dep, store_provider)? else {
        return Ok(()); // leaf dep — nothing to compose
    };
    // NFC is a LEAF (imports nothing) → instantiate ASYNC against a fresh empty linker.
    let nfc_linker = wasmtime::component::Linker::<T>::new(engine);
    let nfc_instance = nfc_linker
        .instantiate_async(&mut *store, &nfc)
        .await
        .map_err(|e| compose_err(format!("instantiating the nfc component failed: {e}")))?;
    let nfc_idx = nfc_instance
        .get_export_index(&mut *store, None, NFC_IFACE)
        .ok_or_else(|| compose_err("nfc instance is missing its exported interface".into()))?;
    let mut iface = dep_linker
        .instance(NFC_IFACE)
        .map_err(|e| compose_err(format!("dep_linker.instance({NFC_IFACE:?}): {e}")))?;
    for fname in &func_names {
        let fidx = nfc_instance
            .get_export_index(&mut *store, Some(&nfc_idx), fname)
            .ok_or_else(|| compose_err(format!("nfc component missing exported func {fname:?}")))?;
        let f = nfc_instance
            .get_func(&mut *store, fidx)
            .ok_or_else(|| compose_err(format!("nfc export {fname:?} is not a func")))?;
        iface
            .func_new_async(fname, move |mut ctx, params, results| {
                Box::new(async move {
                    f.call_async(&mut ctx, params, results).await?;
                    f.post_return_async(&mut ctx).await?;
                    Ok(())
                })
            })
            .map_err(|e| {
                compose_err(format!(
                    "binding nfc func {fname:?} into the dep linker: {e}"
                ))
            })?;
    }
    Ok(())
}

/// ASYNC twin of [`compose_dep_into_linker`] — same dep-compose (read func names off the TYPE, compose the
/// dep's OWN transitive bare deps, forward its funcs into the consumer's linker), but the dep instantiate
/// uses `instantiate_async` and forwarded funcs are bound with `func_new_async` (see
/// [`compose_transitive_bare_deps_async`] for WHY sync panics on the async engine). Returns the dep's live
/// `Instance` (same as the sync form — the value-heap runtime instance the async host would bind a
/// `HeapHandle` on). `T: Send` for the async host-func poll-across-await requirement.
async fn compose_dep_into_linker_async<T: Send + 'static>(
    engine: &wasmtime::Engine,
    store: &mut wasmtime::Store<T>,
    linker: &mut wasmtime::component::Linker<T>,
    import_name: &str,
    dep_bytes: &[u8],
    store_provider: Option<&crate::component_store::ComponentStore>,
) -> Result<wasmtime::component::Instance, ComponentError> {
    use wasmtime::component::Component;
    let compose_err = |reason: String| ComponentError::Compose {
        import_name: import_name.to_string(),
        reason,
    };
    let dep = Component::new(engine, dep_bytes)
        .map_err(|e| compose_err(format!("dependency bytes are not a valid component: {e}")))?;
    // Look the dep's exported interface up by the BARE name (strip @version+hash); register into the
    // CONSUMER's linker under the full `import_name` (see the sync `compose_dep_into_linker` for the why).
    let dep_iface_name = bare_iface_name(import_name);
    let func_names = read_iface_func_names(engine, &dep, dep_iface_name).ok_or_else(|| {
        compose_err(format!(
            "dependency does not export the interface {dep_iface_name:?} the consumer imports \
             (as {import_name:?})"
        ))
    })?;
    // Compose the dep's OWN transitive bare deps (nfc) ASYNC, then instantiate the dep ASYNC.
    let mut dep_linker = wasmtime::component::Linker::<T>::new(engine);
    compose_transitive_bare_deps_async(engine, store, &mut dep_linker, &dep, store_provider)
        .await?;
    let dep_instance = dep_linker
        .instantiate_async(&mut *store, &dep)
        .await
        .map_err(|e| compose_err(format!("instantiating the dependency failed: {e}")))?;
    let iface_idx = dep_instance
        .get_export_index(&mut *store, None, dep_iface_name)
        .ok_or_else(|| {
            compose_err("dependency instance is missing its exported interface".into())
        })?;
    let mut iface = linker
        .instance(import_name)
        .map_err(|e| compose_err(format!("linker.instance({import_name:?}): {e}")))?;
    for fname in &func_names {
        let fidx = dep_instance
            .get_export_index(&mut *store, Some(&iface_idx), fname)
            .ok_or_else(|| compose_err(format!("dependency missing exported func {fname:?}")))?;
        let f = dep_instance
            .get_func(&mut *store, fidx)
            .ok_or_else(|| compose_err(format!("dependency export {fname:?} is not a func")))?;
        iface
            .func_new_async(fname, move |mut ctx, params, results| {
                Box::new(async move {
                    f.call_async(&mut ctx, params, results).await?;
                    f.post_return_async(&mut ctx).await?;
                    Ok(())
                })
            })
            .map_err(|e| compose_err(format!("binding dep func {fname:?} into the linker: {e}")))?;
    }
    Ok(dep_instance)
}

/// Parse a component dependency's `+<hash>` content-address build-metadata into a [`Hash`]. Delegates
/// to [`Hash::from_hex`] — the single home for the canonical-lowercase-hex rule (PR#1013 #4) — rather
/// than reimplementing it here (this used to carry its own copy of the length/lowercase checks).
fn parse_hash_hex(hex: &str) -> Option<Hash> {
    Hash::from_hex(hex)
}

/// A host-side handle to a live `cadenza:runtime/heap` instance, exposing the value-heap ops the reducer
/// fold-boundary MARSHALLING needs (operator ruling C, 2026-08-04): the kernel host builds the reducer's
/// structurally-typed WIT arguments (a content-type `record`, `option<list<u8>>` payloads) as value-heap
/// HANDLES the Cadenza reducer consumes, and reads its `list<effect-request>` result back — because a real
/// Cadenza component crosses every compound as an opaque `u32` handle into the shared runtime, NOT as a
/// structural component-model value (`backend/wasm/host.rs::extern_abi_val_type`; component-abi.md
/// "components composed against a shared runtime exchange values as handles"). So `reducer.wit` stays
/// structurally typed (the contract), and this adapter marshals each field ↔ handle around the reducer's
/// `apply(u32,u32,u32)->u32` handle-ABI export — reusing the SAME runtime instance the reducer's linker
/// is composed against (§23 [`compose_dep_into_linker`]), so a handle the host mints indexes the heap the
/// reducer reads.
///
/// This is SLICE 1: the BUILD (write) ops B1's empty-effects fold needs — `box-int` (scalar field),
/// `str-new` (a `String` field), `arr-alloc`/`arr-set` (a record as a sorted-field array), `sum-new` (an
/// `option` ctor), and `vec-len` (read the effect-list length). The READ ops for B2/B3 (`arr-get`/
/// `str-get`/`sum-disc`/`sum-payload`/`vec-get`/`get-int`/`bytes-*`) layer on in the next slice.
///
/// Each op is a wasmtime component [`Func`] extracted off the instantiated runtime under its
/// `cadenza:runtime/heap` interface; a method calls it over the runtime's `u32`/`s64` valtypes. The
/// runtime handle indices are the FROZEN `runtime.wit` order (documented per method).
pub struct HeapHandle<T: 'static> {
    store: wasmtime::Store<T>,
    /// The fuel budget reported in a [`ComponentError::FuelExhausted`] if a heap op hits `OutOfFuel` (the
    /// store is metered) — set at [`HeapHandle::bind`]. The marshalling ops are host-driven + bounded, so
    /// exhaustion is unexpected, but the classification mirrors the sibling wasm paths so the budget signal
    /// is preserved if the shared store IS metered (#2133).
    fuel_budget: u64,
    // BUILD ops (slice 1) — construct the reducer's structural WIT args as value-heap handles.
    box_int: wasmtime::component::Func,
    arr_alloc: wasmtime::component::Func,
    arr_set: wasmtime::component::Func,
    sum_new: wasmtime::component::Func,
    vec_len: wasmtime::component::Func,
    str_new: wasmtime::component::Func,
    // READ ops (slice 2) — project the reducer's returned effect-request list back to structural values.
    arr_get: wasmtime::component::Func,
    str_get: wasmtime::component::Func,
    sum_disc: wasmtime::component::Func,
    sum_payload: wasmtime::component::Func,
    vec_get: wasmtime::component::Func,
    get_int: wasmtime::component::Func,
    // BYTES ops (slice 2b) — build/read a `Bytes` handle for the reducer's `Option<Bytes>` payload/resumes
    // fields + their effect-request payload result (all u32-shaped: byte value 0..255 rides as a u32).
    bytes_alloc: wasmtime::component::Func,
    bytes_set: wasmtime::component::Func,
    bytes_len: wasmtime::component::Func,
    bytes_get: wasmtime::component::Func,
}

/// Classify a heap-op `Func::call`/`post_return` failure (github-liaison #2122/#2133): the SAME 3-way split
/// the sibling wasm paths ([`invoke_component`], [`ComponentReducer::apply`]) use:
/// - `Trap::OutOfFuel` → [`ComponentError::FuelExhausted`] (a resource-ceiling hit — the Copilot PR#1009
///   DoS convention; distinct from a semantic trap so the budget signal isn't lost if heap ops run metered);
/// - any other trap → [`ComponentError::Trap`] (a genuine guest trap);
/// - a NON-trap error → [`ComponentError::Instantiate`] (a host-side signature mismatch = a WIT/runtime-ABI
///   drift, NOT a guest trap — so ABI drift doesn't masquerade as "the guest trapped").
fn classify_heap_call_err(e: wasmtime::Error, budget: u64) -> ComponentError {
    if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
        ComponentError::FuelExhausted { budget }
    } else if e.downcast_ref::<wasmtime::Trap>().is_some() {
        ComponentError::Trap(e.to_string())
    } else {
        ComponentError::Instantiate(format!("heap-op call failed (runtime-ABI mismatch?): {e}"))
    }
}

impl<T: 'static> HeapHandle<T> {
    /// Bind a `HeapHandle` from an already-instantiated runtime component `instance` in `store` (the
    /// SAME instance composed into the reducer's linker, so handles are shared). Extracts each B1 heap op
    /// as a [`Func`] off the `cadenza:runtime/heap` exported interface — a missing op is a `Compose`
    /// error naming it (the runtime doesn't expose the expected interface, caught here rather than as an
    /// opaque trap at marshal time).
    ///
    /// On error the CONSUMED `store` is handed back with the error (`Err((e, store))`), so a caller can
    /// recover the store's data — e.g. the fold-boundary rebind returns the reducer host's base KV rather
    /// than dropping it (github-liaison #2203 MED: the "any error hands the base KV back" atomicity
    /// contract). A caller that doesn't need the store just discards it.
    pub fn bind(
        mut store: wasmtime::Store<T>,
        instance: &wasmtime::component::Instance,
    ) -> Result<Self, (ComponentError, wasmtime::Store<T>)> {
        let iface_err = |reason: String| ComponentError::Compose {
            import_name: "cadenza:runtime/heap".to_string(),
            reason,
        };
        // Extract every op in an inner closure returning a plain `Result<_, ComponentError>` (it borrows
        // `store`); the borrow ends when it returns, so on Err `store` is free to move into the error tuple.
        let ops = (|| {
            let iface_idx = instance
                .get_export_index(&mut store, None, "cadenza:runtime/heap")
                .ok_or_else(|| iface_err("runtime does not export cadenza:runtime/heap".into()))?;
            let mut op = |name: &str| -> Result<wasmtime::component::Func, ComponentError> {
                let idx = instance
                    .get_export_index(&mut store, Some(&iface_idx), name)
                    .ok_or_else(|| iface_err(format!("heap interface missing op {name:?}")))?;
                instance
                    .get_func(&mut store, idx)
                    .ok_or_else(|| iface_err(format!("heap export {name:?} is not a func")))
            };
            Ok::<_, ComponentError>((
                op("box-int")?,
                op("arr-alloc")?,
                op("arr-set")?,
                op("sum-new")?,
                op("vec-len")?,
                op("str-new")?,
                op("arr-get")?,
                op("str-get")?,
                op("sum-disc")?,
                op("sum-payload")?,
                op("vec-get")?,
                op("get-int")?,
                op("bytes-alloc")?,
                op("bytes-set")?,
                op("bytes-len")?,
                op("bytes-get")?,
            ))
        })();
        let (
            box_int,
            arr_alloc,
            arr_set,
            sum_new,
            vec_len,
            str_new,
            arr_get,
            str_get,
            sum_disc,
            sum_payload,
            vec_get,
            get_int,
            bytes_alloc,
            bytes_set,
            bytes_len,
            bytes_get,
        ) = match ops {
            Ok(ops) => ops,
            Err(e) => return Err((e, store)),
        };
        Ok(HeapHandle {
            store,
            fuel_budget: DEFAULT_FOLD_FUEL,
            box_int,
            arr_alloc,
            arr_set,
            sum_new,
            vec_len,
            str_new,
            arr_get,
            str_get,
            sum_disc,
            sum_payload,
            vec_get,
            get_int,
            bytes_alloc,
            bytes_set,
            bytes_len,
            bytes_get,
        })
    }

    /// Call a heap op that takes u32 args and returns one u32 handle — the common shape (arr-alloc,
    /// arr-set, sum-new, vec-len). Args are passed as `Val::U32`; the single result is read back as u32.
    fn call_u32s(
        &mut self,
        f: &wasmtime::component::Func,
        args: &[u32],
    ) -> Result<u32, ComponentError> {
        use wasmtime::component::Val;
        let budget = self.fuel_budget;
        let params: Vec<Val> = args.iter().map(|&a| Val::U32(a)).collect();
        let mut results = [Val::U32(0)];
        f.call(&mut self.store, &params, &mut results)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        f.post_return(&mut self.store)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        match results[0] {
            Val::U32(h) => Ok(h),
            ref other => Err(ComponentError::Trap(format!(
                "heap op returned {other:?}, not a u32 handle"
            ))),
        }
    }

    /// `box-int(v: s64) -> u32` (runtime.wit idx 0): box a scalar int as a heap handle — a scalar field
    /// inside a record crosses as a BOXED handle (v-rust-backend confirm), so a record's int field is
    /// `arr-set(arr, i, box_int(v))`.
    pub fn box_int(&mut self, v: i64) -> Result<u32, ComponentError> {
        use wasmtime::component::Val;
        let budget = self.fuel_budget;
        let f = self.box_int;
        let mut results = [Val::U32(0)];
        f.call(&mut self.store, &[Val::S64(v)], &mut results)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        f.post_return(&mut self.store)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        match results[0] {
            Val::U32(h) => Ok(h),
            ref other => Err(ComponentError::Trap(format!(
                "box-int returned {other:?}, not a u32 handle"
            ))),
        }
    }

    /// `str-new(s: string) -> u32` (idx 17): intern a String as a heap handle — a `String` field crosses
    /// as its rope handle.
    pub fn str_new(&mut self, s: &str) -> Result<u32, ComponentError> {
        use wasmtime::component::Val;
        let budget = self.fuel_budget;
        let f = self.str_new;
        let mut results = [Val::U32(0)];
        f.call(&mut self.store, &[Val::String(s.into())], &mut results)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        f.post_return(&mut self.store)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        match results[0] {
            Val::U32(h) => Ok(h),
            ref other => Err(ComponentError::Trap(format!(
                "str-new returned {other:?}, not a u32 handle"
            ))),
        }
    }

    /// `arr-alloc(len: u32) -> u32` (idx 6): allocate a `len`-element value-heap array — a record/tuple
    /// crosses as an array of its field handles (in SORTED field-name order for a record).
    pub fn arr_alloc(&mut self, len: u32) -> Result<u32, ComponentError> {
        let f = self.arr_alloc;
        self.call_u32s(&f, &[len])
    }

    /// `arr-set(arr, index, elem) -> arr` (idx 7): set element `index` of `arr` to handle `elem`, returning
    /// the array handle for threading.
    pub fn arr_set(&mut self, arr: u32, index: u32, elem: u32) -> Result<u32, ComponentError> {
        let f = self.arr_set;
        self.call_u32s(&f, &[arr, index, elem])
    }

    /// `sum-new(disc, payload) -> u32` (idx 10): build a sum handle with discriminant `disc` and payload
    /// handle `payload` — an `option` is `sum-new(0, some_handle)` / `sum-new(1, unit_handle)`; a nullary
    /// variant carries the UNIT value as its payload. ⚠ the runtime's unit is the INLINE-UNIT handle
    /// (`IMM_UNIT`), NOT handle 0 (a NULL handle/token) — obtain it via [`HeapHandle::unit`] (`arr-alloc(0)`,
    /// the empty array = inline unit, per runtime.wit "a nullary variant carries the unit value (an arr of
    /// length 0)"). Passing 0 would build a sum with a NULL payload = a malformed value.
    pub fn sum_new(&mut self, disc: u32, payload: u32) -> Result<u32, ComponentError> {
        let f = self.sum_new;
        self.call_u32s(&f, &[disc, payload])
    }

    /// The runtime's UNIT value handle — the inline-unit (`IMM_UNIT`), obtained as `arr-alloc(0)` (the
    /// empty array IS the inline unit). Use this as the payload for a `None`/nullary sum, NOT handle 0 (a
    /// NULL handle). A compile-time-known immediate; `arr-alloc(0)` returns it with no heap node.
    pub fn unit(&mut self) -> Result<u32, ComponentError> {
        self.arr_alloc(0)
    }

    /// `vec-len(v: u32) -> u32` (idx ~30): the element count of a value-heap vector — reads the length of
    /// the reducer's returned `list<effect-request>` (B1 asserts it's 0).
    pub fn vec_len(&mut self, v: u32) -> Result<u32, ComponentError> {
        let f = self.vec_len;
        self.call_u32s(&f, &[v])
    }

    // ── READ ops (slice 2) — PROJECT the reducer's returned effect-request list back to structural values.
    // These read a value-heap handle the reducer produced: walk the `list<effect-request>` (vec-len/vec-get),
    // project each effect-request record's fields (arr-get at the SORTED index), and decode leaf values
    // (get-int for a boxed scalar, str-get for a String, sum-disc/sum-payload for an Option/enum-kind). The
    // dual of the BUILD ops above; together they marshal the reducer's structural WIT boundary (B2/B3).

    /// `vec-get(v, index) -> u32` (idx 31): the element handle at `index` of a value-heap vector — walks the
    /// returned effect-request list (each element an effect-request record handle).
    pub fn vec_get(&mut self, v: u32, index: u32) -> Result<u32, ComponentError> {
        let f = self.vec_get;
        self.call_u32s(&f, &[v, index])
    }

    /// `arr-get(arr, index) -> u32` (idx 8): the element handle at `index` of a value-heap array — projects
    /// a record field by its SORTED-field-name index (v-rust-backend: records lay out arr in sorted order).
    pub fn arr_get(&mut self, arr: u32, index: u32) -> Result<u32, ComponentError> {
        let f = self.arr_get;
        self.call_u32s(&f, &[arr, index])
    }

    /// `sum-disc(handle) -> u32` (idx 11): the discriminant of a sum handle — reads an Option's Some(0)/
    /// None(1) or an effect-kind variant's disc (0..5 per the EffectKind table).
    pub fn sum_disc(&mut self, handle: u32) -> Result<u32, ComponentError> {
        let f = self.sum_disc;
        self.call_u32s(&f, &[handle])
    }

    /// `sum-payload(handle) -> u32` (idx 12): the payload handle of a sum — the inner value of a `Some`, or
    /// the unit for a nullary case.
    pub fn sum_payload(&mut self, handle: u32) -> Result<u32, ComponentError> {
        let f = self.sum_payload;
        self.call_u32s(&f, &[handle])
    }

    /// `get-int(handle) -> s64` (idx 1): unbox a boxed scalar int — the read-dual of [`HeapHandle::box_int`]
    /// (a record's scalar field, e.g. a version or a numeric effect field).
    pub fn get_int(&mut self, handle: u32) -> Result<i64, ComponentError> {
        use wasmtime::component::Val;
        let budget = self.fuel_budget;
        let f = self.get_int;
        let mut results = [Val::S64(0)];
        f.call(&mut self.store, &[Val::U32(handle)], &mut results)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        f.post_return(&mut self.store)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        match results[0] {
            Val::S64(v) => Ok(v),
            ref other => Err(ComponentError::Trap(format!(
                "get-int returned {other:?}, not an s64"
            ))),
        }
    }

    /// `str-get(handle) -> string` (idx 18): read a String rope handle back to a Rust `String` — the
    /// read-dual of [`HeapHandle::str_new`] (a record's String field, e.g. an effect target).
    pub fn str_get(&mut self, handle: u32) -> Result<String, ComponentError> {
        use wasmtime::component::Val;
        let budget = self.fuel_budget;
        let f = self.str_get;
        let mut results = [Val::Bool(false)];
        f.call(&mut self.store, &[Val::U32(handle)], &mut results)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        f.post_return(&mut self.store)
            .map_err(|e| classify_heap_call_err(e, budget))?;
        match &results[0] {
            Val::String(s) => Ok(s.clone()),
            other => Err(ComponentError::Trap(format!(
                "str-get returned {other:?}, not a string"
            ))),
        }
    }

    // ── BYTES ops (slice 2b) — build/read a value-heap `Bytes` handle. The reducer's payload/resumes are
    // `Option<Bytes>`, and an emitted effect-request's payload/correlation are `Option<Bytes>` too, so the
    // marshalling needs to put arg bytes ON the heap (bytes_from) and read result bytes back (read_bytes).
    // A byte value rides as a u32 (0..255) — the runtime's frozen `bytes-*` shape (idx 13-16).

    /// `bytes-alloc(len) -> u32` (idx 13): allocate a `len`-byte value-heap buffer.
    pub fn bytes_alloc(&mut self, len: u32) -> Result<u32, ComponentError> {
        let f = self.bytes_alloc;
        self.call_u32s(&f, &[len])
    }

    /// `bytes-set(buf, index, value) -> buf` (idx 14): set byte `index` of `buf` to `value` (0..255),
    /// returning the buffer handle for threading.
    pub fn bytes_set(&mut self, buf: u32, index: u32, value: u8) -> Result<u32, ComponentError> {
        let f = self.bytes_set;
        self.call_u32s(&f, &[buf, index, value as u32])
    }

    /// `bytes-len(buf) -> u32` (idx 16): the length of a value-heap byte buffer.
    pub fn bytes_len(&mut self, buf: u32) -> Result<u32, ComponentError> {
        let f = self.bytes_len;
        self.call_u32s(&f, &[buf])
    }

    /// `bytes-get(buf, index) -> u32` (idx 15): the byte at `index` of `buf` (0..255, as a u32).
    pub fn bytes_get(&mut self, buf: u32, index: u32) -> Result<u32, ComponentError> {
        let f = self.bytes_get;
        self.call_u32s(&f, &[buf, index])
    }

    /// Build a value-heap `Bytes` handle from a Rust byte slice — `bytes-alloc(len)` then a `bytes-set` per
    /// byte (threading the handle). The marshalling convenience for an `Option<Bytes>` payload's Some arm.
    ///
    /// The slice length is range-checked to `u32` UP FRONT (`u32::try_from`): the value-heap's `bytes-*`
    /// shape carries a `u32` length, so a `> u32::MAX` slice cannot fit. Truncating (`data.len() as u32`)
    /// would silently under-allocate and then let the `i as u32` loop index wrap and write PAST the buffer,
    /// corrupting the guest value-heap (#2151). Failing loud here keeps the loop index provably in-bounds
    /// (`i < len ≤ u32::MAX`) — symmetric with [`HeapHandle::read_bytes`]'s defensive byte check.
    pub fn bytes_from(&mut self, data: &[u8]) -> Result<u32, ComponentError> {
        let len = u32::try_from(data.len()).map_err(|_| {
            ComponentError::Trap(format!(
                "bytes_from: slice of {} bytes too large for a u32 value-heap length",
                data.len()
            ))
        })?;
        let mut buf = self.bytes_alloc(len)?;
        for (i, &b) in data.iter().enumerate() {
            buf = self.bytes_set(buf, i as u32, b)?;
        }
        Ok(buf)
    }

    /// Read a value-heap `Bytes` handle back to a Rust `Vec<u8>` — `bytes-len` then a `bytes-get` per byte.
    /// The read-dual of [`HeapHandle::bytes_from`]; a byte over 255 is a malformed value-heap byte (Trap).
    ///
    /// The pre-reservation is CAPPED at [`MAX_PREALLOC_BYTES`]: `bytes-len` returns a raw unbounded `u32`
    /// reported by the untrusted guest value-heap, so a bogus large length must NOT drive an eager multi-GB
    /// `Vec::with_capacity` (which aborts the host via the alloc-error handler BEFORE the loop can fail-loud
    /// on the missing bytes — reviewer, #2166 read-side DoS). A truthful large buffer just grows the Vec
    /// with amortized reallocations; a bogus length Traps CHEAPLY on the first missing `bytes-get`.
    pub fn read_bytes(&mut self, buf: u32) -> Result<Vec<u8>, ComponentError> {
        let len = self.bytes_len(buf)?;
        let mut out = Vec::with_capacity(len.min(MAX_PREALLOC_BYTES) as usize);
        for i in 0..len {
            let b = self.bytes_get(buf, i)?;
            let byte = u8::try_from(b).map_err(|_| {
                ComponentError::Trap(format!("bytes-get returned {b}, not a byte (0..255)"))
            })?;
            out.push(byte);
        }
        Ok(out)
    }

    /// The underlying store, for the caller to instantiate/call the reducer against the SAME store (so a
    /// handle the host minted is valid in the reducer's `apply` call).
    pub fn store_mut(&mut self) -> &mut wasmtime::Store<T> {
        &mut self.store
    }

    /// Consume the handle, returning the owned store — for the fold-boundary rebind to commit/recover the
    /// reducer host's KV after the marshalled `apply` (the handle is done once the effects are unmarshalled).
    pub fn into_store(self) -> wasmtime::Store<T> {
        self.store
    }

    /// Set the fuel budget reported on an `OutOfFuel` classification (the marshalled `apply` reuses the
    /// reducer's per-fold budget, so the handle's budget should match — see [`HeapHandle::bind`]).
    pub fn set_fuel_budget(&mut self, budget: u64) {
        self.fuel_budget = budget;
    }

    /// Call a handle-lowered reducer's `apply(ct, payload, resumes) -> list-handle` (§19e): three `u32`
    /// value-heap arg handles in, one `u32` result handle (the returned `list<effect-request>`) out. Runs
    /// on the SAME store the arg handles were built in, so the guest reads them off the shared heap.
    ///
    /// Errors are classified with REDUCER-FOLD context, NOT as a runtime heap-op (#2203): the fuel/trap
    /// split matches `classify_heap_call_err` (OutOfFuel → `FuelExhausted`, other trap → `Trap`), but a
    /// NON-trap `call` error here is the reducer's `apply` export not matching the expected
    /// `func(u32,u32,u32)->u32` signature (an ABI drift in the REDUCER, not the runtime), so it's an
    /// `InvokeExport` naming the fold apply — reusing `call_u32s`'s "heap-op call failed" label would
    /// mis-point a debugger at the runtime.
    pub fn call_apply_lowered(
        &mut self,
        apply: &wasmtime::component::Func,
        ct: u32,
        payload: u32,
        resumes: u32,
    ) -> Result<u32, ComponentError> {
        use wasmtime::component::Val;
        let budget = self.fuel_budget;
        let reducer_err = |e: wasmtime::Error| -> ComponentError {
            if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
                ComponentError::FuelExhausted { budget }
            } else if e.downcast_ref::<wasmtime::Trap>().is_some() {
                ComponentError::Trap(e.to_string())
            } else {
                ComponentError::InvokeExport {
                    interface: "cadenza:agent-kernel/fold".to_string(),
                    func: "apply".to_string(),
                    reason: format!(
                        "reducer apply is not func(u32,u32,u32)->u32 (reducer-ABI mismatch?): {e}"
                    ),
                }
            }
        };
        let params = [Val::U32(ct), Val::U32(payload), Val::U32(resumes)];
        let mut results = [Val::U32(0)];
        apply
            .call(&mut self.store, &params, &mut results)
            .map_err(reducer_err)?;
        apply.post_return(&mut self.store).map_err(reducer_err)?;
        match results[0] {
            Val::U32(h) => Ok(h),
            ref other => Err(ComponentError::InvokeExport {
                interface: "cadenza:agent-kernel/fold".to_string(),
                func: "apply".to_string(),
                reason: format!("reducer apply returned {other:?}, not a u32 list handle"),
            }),
        }
    }
}

/// Generic MULTI-EXPORT component INVOCATION — the core mechanism of the operator's resolve-name→
/// component→invoke primitive (Slack seq 107/108, 2026-08-04). Instantiate arbitrary component `bytes`,
/// call the export named by `interface`#`func` over an AST-encoded arg, and decode its result into a SET
/// of [`Artifact`]s. NOT the reducer world, NOT a single canonical entry: the caller names WHICH export
/// (multi-export, seq-107), the arg + artifact bytes are AST-encoded (the wire format is the AST
/// encoding), and the result is `Vec<Artifact>` (seq-108's multi-artifact — the compiler emits several).
/// A later slice adds the selector program that routes each artifact to its sink (session | CAS); this
/// slice is the pure INVOKE mechanism, no placement.
///
/// The invoked export must be a WIT `func(list<u8>) -> list<record { kind: string, name: string, bytes:
/// list<u8> }>` — one AST-encoded arg in, an artifact set out. (`arg` is a single AST-encoded value; a
/// multi-arg call is a later refinement once the strong-typing/type-infer seam lands — the arg is
/// self-describing AST bytes.) `interface` is the exported instance name (e.g. `cadenza:compiler/api`);
/// `func` is the function within it. Passing an EMPTY `interface` looks the func up as a TOP-LEVEL
/// export (a component that exports the func directly, not under an instance).
///
/// Fuel-metered (§22d, [`DEFAULT_FOLD_FUEL`]): a runaway invokee aborts at the budget with
/// [`ComponentError::FuelExhausted`] rather than hanging — a resolved-and-invoked component is untrusted
/// guest code, exactly as a reducer is. Bad bytes → [`ComponentError::InvalidComponent`]; a valid
/// component missing `interface#func` or returning the wrong shape → [`ComponentError::InvokeExport`]
/// (actionable "not invokable there"); a clean call that traps → [`ComponentError::Trap`]. The invokee
/// declares no host imports (a component that imports host state is a reducer, driven via the fold loop /
/// v-ah-host's session path); a dep-carrying invokee is a later slice.
pub fn invoke_component(
    bytes: &[u8],
    interface: &str,
    func: &str,
    arg: &[u8],
    fuel_budget: u64,
) -> Result<Vec<Artifact>, ComponentError> {
    use wasmtime::component::{Component, Linker, Val};
    let export_err = |reason: String| ComponentError::InvokeExport {
        interface: interface.to_string(),
        func: func.to_string(),
        reason,
    };

    // Fuel-metered engine (per-engine flag → fresh engine per invoke; caching a compiled Component by
    // hash is a perf slice once invocation is hot — correctness first).
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    let engine =
        wasmtime::Engine::new(&config).map_err(|e| ComponentError::Instantiate(e.to_string()))?;
    let component = Component::new(&engine, bytes)
        .map_err(|e| ComponentError::InvalidComponent(e.to_string()))?;

    let mut store = wasmtime::Store::new(&engine, ());
    // Ample fuel for instantiation (structure-bounded, not the DoS surface), then reset to the caller's
    // budget right before the call so the budget bounds the INVOCATION precisely.
    store
        .set_fuel(u64::MAX)
        .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
    // The invokee is a pure value transform: no host imports to serve → empty linker. (A host-importing
    // component is a reducer, driven via the fold loop — not this seam.)
    let linker = Linker::<()>::new(&engine);
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| ComponentError::Instantiate(e.to_string()))?;

    // Resolve `interface#func`: an empty interface = a top-level func export; otherwise navigate into the
    // exported instance `interface`, then the `func` within it. A missing export is InvokeExport (the
    // component is valid but not invokable there), NOT a trap.
    let func_handle = {
        let iface_idx = if interface.is_empty() {
            None
        } else {
            Some(
                instance
                    .get_export_index(&mut store, None, interface)
                    .ok_or_else(|| {
                        export_err(format!("component exports no interface {interface:?}"))
                    })?,
            )
        };
        let func_idx = instance
            .get_export_index(&mut store, iface_idx.as_ref(), func)
            .ok_or_else(|| {
                // Empty interface = a top-level lookup; word the error that way rather than the confusing
                // `interface "" exports no func` (github-liaison #2050 LOW).
                if interface.is_empty() {
                    export_err(format!("component exports no top-level func {func:?}"))
                } else {
                    export_err(format!("interface {interface:?} exports no func {func:?}"))
                }
            })?;
        instance
            .get_func(&mut store, func_idx)
            .ok_or_else(|| export_err(format!("export {interface:?}#{func:?} is not a func")))?
    };

    store
        .set_fuel(fuel_budget)
        .map_err(|e| ComponentError::Instantiate(e.to_string()))?;

    // Call over the canonical invoke shape: params = one AST-encoded `list<u8>`, result = one
    // `list<record{kind,name,bytes}>`. `call` type-checks params/results against the func signature and
    // errors if the export isn't that shape — surfaced as InvokeExport (wrong shape), a real trap as Trap,
    // fuel exhaustion as FuelExhausted (mirroring `apply`'s split).
    let params = [Val::List(arg.iter().copied().map(Val::U8).collect())];
    let mut results = [Val::Bool(false)]; // placeholder; `call` overwrites with the real result
    if let Err(e) = func_handle.call(&mut store, &params, &mut results) {
        if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
            return Err(ComponentError::FuelExhausted {
                budget: fuel_budget,
            });
        }
        if e.downcast_ref::<wasmtime::Trap>().is_some() {
            return Err(ComponentError::Trap(e.to_string()));
        }
        // A non-trap `call` error is a param/result type mismatch — the export isn't the artifact-set shape.
        return Err(export_err(format!(
            "export {interface:?}#{func:?} is not func(list<u8>) -> list<record{{kind,name,bytes}}>: {e}"
        )));
    }
    // PROPAGATE post_return (github-liaison #2050 MED): the dep-forwarding path (`compose_dep_into_linker`)
    // propagates it too. post_return can surface a guest trap / out-of-fuel / resource-cleanup failure
    // AFTER the call returned; dropping it (`let _ =`) makes a post-return-trapping guest look successful and
    // leaks the cleanup failure. Classify it exactly like the call error above (fuel → FuelExhausted, trap →
    // Trap, else a host-side Instantiate — a non-trap post_return failure is host/engine, not guest shape).
    if let Err(e) = func_handle.post_return(&mut store) {
        if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
            return Err(ComponentError::FuelExhausted {
                budget: fuel_budget,
            });
        }
        if e.downcast_ref::<wasmtime::Trap>().is_some() {
            return Err(ComponentError::Trap(e.to_string()));
        }
        return Err(ComponentError::Instantiate(format!(
            "post_return after {interface:?}#{func:?} failed: {e}"
        )));
    }

    // Decode the single `list<record{kind:string, name:string, bytes:list<u8>}>` result into Artifacts.
    decode_artifact_list(&results[0], &export_err)
}

/// A BOUNDED description of a component [`Val`]'s shape for an error message — its type/variant, never its
/// full `{:?}` (github-liaison #2050 MED/DoS): `decode_artifact_list` reports on UNTRUSTED guest output, so
/// Debug-formatting a huge/deeply-nested wrong-shape value into the error string is an unbounded log/memory
/// blowup on the error path (same class as the #1852 unbounded-set finding). This reports the variant name
/// with a size hint (list/record length) — enough to diagnose a shape mismatch, capped regardless of the
/// value's size.
fn val_shape(val: &wasmtime::component::Val) -> String {
    use wasmtime::component::Val;
    match val {
        Val::Bool(_) => "bool".into(),
        Val::S8(_) => "s8".into(),
        Val::U8(_) => "u8".into(),
        Val::S16(_) => "s16".into(),
        Val::U16(_) => "u16".into(),
        Val::S32(_) => "s32".into(),
        Val::U32(_) => "u32".into(),
        Val::S64(_) => "s64".into(),
        Val::U64(_) => "u64".into(),
        Val::Float32(_) => "float32".into(),
        Val::Float64(_) => "float64".into(),
        Val::Char(_) => "char".into(),
        Val::String(s) => format!("string(len {})", s.len()),
        Val::List(items) => format!("list(len {})", items.len()),
        Val::Record(fields) => format!("record({} fields)", fields.len()),
        Val::Tuple(items) => format!("tuple(len {})", items.len()),
        // Drop the case LABEL: it's component type-metadata (attacker-controllable, arbitrarily long), so
        // embedding it re-opens the same unbounded-message DoS the rest of val_shape closes (#2057 follow-up).
        // Just the arm.
        Val::Variant(_, _) => "variant".into(),
        Val::Enum(_) => "enum".into(),
        Val::Option(_) => "option".into(),
        Val::Result(_) => "result".into(),
        Val::Flags(f) => format!("flags({} set)", f.len()),
        Val::Resource(_) => "resource".into(),
        // wasmtime's Val is #[non_exhaustive]; a future variant reports generically rather than {:?}-ing it.
        _ => "other".into(),
    }
}

/// Decode a wasmtime component [`Val`] that is a `list<record{kind:string, name:string, bytes:list<u8>}>`
/// into [`Artifact`]s — the result-shape half of [`invoke_component`], split out so the shape contract is
/// in one place. Any deviation (not a list, an element that isn't the 3-field record, a field of the
/// wrong type) is an [`ComponentError::InvokeExport`] via `export_err` — the export ran but didn't return
/// the artifact-set shape the invoke seam decodes.
fn decode_artifact_list(
    val: &wasmtime::component::Val,
    export_err: &impl Fn(String) -> ComponentError,
) -> Result<Vec<Artifact>, ComponentError> {
    use wasmtime::component::Val;
    // Error reasons report `val_shape` (bounded variant + size hint), NEVER `{val:?}` of the untrusted
    // guest output (github-liaison #2050 MED/DoS): a huge/deeply-nested wrong-shape value would otherwise be
    // fully Debug-formatted into the error string = unbounded log/memory blowup.
    let Val::List(items) = val else {
        return Err(export_err(format!(
            "result is {}, not a list<record{{kind,name,bytes}}>",
            val_shape(val)
        )));
    };
    let field_str = |rec: &[(String, Val)], want: &str| -> Result<String, ComponentError> {
        match rec.iter().find(|(n, _)| n == want).map(|(_, v)| v) {
            Some(Val::String(s)) => Ok(s.clone()),
            Some(other) => Err(export_err(format!(
                "artifact field {want:?} is {}, not a string",
                val_shape(other)
            ))),
            None => Err(export_err(format!(
                "artifact record is missing field {want:?}"
            ))),
        }
    };
    let field_bytes = |rec: &[(String, Val)], want: &str| -> Result<Vec<u8>, ComponentError> {
        match rec.iter().find(|(n, _)| n == want).map(|(_, v)| v) {
            Some(Val::List(bs)) => bs
                .iter()
                .map(|b| match b {
                    Val::U8(x) => Ok(*x),
                    other => Err(export_err(format!(
                        "artifact {want:?} list element is {}, not a u8",
                        val_shape(other)
                    ))),
                })
                .collect(),
            Some(other) => Err(export_err(format!(
                "artifact field {want:?} is {}, not a list<u8>",
                val_shape(other)
            ))),
            None => Err(export_err(format!(
                "artifact record is missing field {want:?}"
            ))),
        }
    };
    items
        .iter()
        .map(|item| match item {
            Val::Record(fields) => Ok(Artifact {
                kind: field_str(fields, "kind")?,
                name: field_str(fields, "name")?,
                bytes: field_bytes(fields, "bytes")?,
            }),
            other => Err(export_err(format!(
                "artifact-set element is {}, not a record{{kind,name,bytes}}",
                val_shape(other)
            ))),
        })
        .collect()
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
        // `Engine::new` failing is a config/platform/host-setup condition, NOT a malformed component
        // (the bytes haven't even been read yet) — classify it as `Instantiate` (host-side), same as
        // the `set_fuel` host-setup failures in `apply`. Only `Component::new` below is about the bytes.
        let engine = wasmtime::Engine::new(&config)
            .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
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
        // Pre-instantiate ONCE for the dependency-free path (perf): resolve + type-check the linker
        // against the component now, so each fold just `.instantiate(store)`s the cached `ReducerPre`
        // instead of re-linking. Only for a reducer with no deps to compose per-fold (a dep reducer's
        // linker differs each fold). Best-effort (PR#1270): pre-instantiate ONLY if it succeeds —
        // `ReducerPre::new` type-checks that the component exports
        // the `fold` world, so a valid-but-non-fold-exporting component (e.g. a construction-only test
        // fixture, or a not-yet-a-reducer blob) can't be pre-instantiated. In that case leave
        // `instance_pre = None`: `apply` falls back to the per-fold `Reducer::instantiate`, which surfaces
        // the SAME "no fold export" error at apply time — so construction stays lenient (unchanged
        // contract: any valid component builds), and only real fold-exporting reducers get the fast path.
        let instance_pre = if deps.is_empty() {
            linker
                .instantiate_pre(&component)
                .ok()
                .and_then(|pre| ReducerPre::new(pre).ok())
        } else {
            None
        };
        Ok(ComponentReducer {
            engine,
            component,
            linker,
            deps,
            resolved_deps: Vec::new(),
            instance_pre,
            fuel_budget: DEFAULT_FOLD_FUEL,
            component_store: None,
        })
    }

    /// Attach a [`ComponentStore`](crate::component_store::ComponentStore) used to resolve a dep's OWN
    /// transitive BARE imports (§23 leaves-first compose). The value-heap runtime imports the bare
    /// `cadenza:nfc/normalize` component; with a store set, `compose_dep_into_linker` resolves it by name
    /// (`runtime.toml`'s `nfc = "<hash>"`) and composes it into the runtime's linker before instantiating
    /// the runtime. Without one, a dep that imports nfc fails to compose (a `Compose` error naming it),
    /// so this is REQUIRED for a real runtime dep whose world imports nfc. (The host reads the store dir
    /// from `CDZ_STORE` / v-nix's `componentStore`.)
    pub fn with_component_store(mut self, store: crate::component_store::ComponentStore) -> Self {
        self.component_store = Some(store);
        self
    }

    /// Attach the resolved bytes of this reducer's declared dependencies (§23), so `apply` COMPOSES each
    /// into the per-fold linker before instantiating the guest (via `compose_dep_into_linker`). Pair
    /// this with [`ComponentReducer::resolve_deps`], which fetches the bytes from CAS: resolve, then
    /// attach. A dependency-free reducer never needs this (its `deps` are empty). Idempotent-replacing:
    /// the last call's set is what `apply` composes. The kernel stays runtime-AGNOSTIC — it composes the
    /// value-heap runtime, if present, exactly as any other content-addressed dep.
    pub fn with_resolved_deps(mut self, resolved: Vec<(ComponentDep, Vec<u8>)>) -> Self {
        self.resolved_deps = resolved
            .into_iter()
            .map(|(dep, bytes)| (dep.import_name, bytes))
            .collect();
        // Attaching deps moves this reducer to the per-fold compose path — the cached dependency-free
        // pre-instantiation no longer applies (its linker lacks the dep imports), so drop it. `apply`
        // sees `instance_pre == None` and composes deps per fold. (If `resolved` is empty this is a no-op
        // set + a harmless clear — a genuinely dep-free reducer keeps its pre via `from_component_bytes`.)
        if !self.resolved_deps.is_empty() {
            self.instance_pre = None;
        }
        self
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

    /// Whether this reducer takes the cached-`ReducerPre` FAST PATH per fold (perf): `true` for a
    /// dependency-free, fold-exporting reducer (pre-instantiated once at construction, so each `apply`
    /// skips re-linking); `false` for a dep reducer (composed per fold) or a component that doesn't
    /// export the `fold` world (falls back to per-fold instantiate). Exposed for observability + tests.
    pub fn uses_cached_instance_pre(&self) -> bool {
        self.instance_pre.is_some()
    }

    /// Resolve ALL of this reducer's declared dependency bytes from a blob store (§23), each paired with
    /// the import name the linker must bind it under. Empty vec if dependency-free. `Err(DepMissing)` if
    /// a declared dep isn't in the store, `Err(DepStoreError)` if the store itself errored (PR#1013 #3 —
    /// the two are distinct). Generic: it resolves the Cadenza runtime, if present, exactly as it
    /// resolves any other dep — the kernel never asks "is this the runtime?" (The linker-compose of these
    /// bytes — binding each under its `import_name` — is the next slice.)
    pub async fn resolve_deps(
        &self,
        blobs: &dyn crate::blob::BlobStore,
    ) -> Result<Vec<(ComponentDep, Vec<u8>)>, ComponentError> {
        // Sequential await (not a .map().collect() — can't await in a map closure): resolve each dep's
        // bytes from CAS in order, short-circuiting on the first error.
        let mut out = Vec::with_capacity(self.deps.len());
        for dep in &self.deps {
            let bytes = resolve_dep_bytes(dep, blobs).await?;
            out.push((dep.clone(), bytes));
        }
        Ok(out)
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
    ///
    /// TRANSACTIONAL + no full-KV clone: `apply` takes `kv` BY VALUE (moved, not cloned — `fold`
    /// hands it in via `mem::take`) and returns it in BOTH arms. The guest's writes go to the host's
    /// [`ReducerHost`] OVERLAY, not the base; `apply` COMMITS the overlay into the base ONLY on a
    /// successful fold. So `Ok((effects, kv))` carries the fold's mutations, while `Err((error, kv))`
    /// hands back the base VERBATIM — a trapped / fuel-exhausted / instantiate-failed fold leaves the
    /// KV exactly as it was (all-or-nothing atomicity), because its uncommitted overlay writes are
    /// discarded when the host drops. Only the write-set is buffered (O(writes)), never an O(KV size)
    /// full copy (the PR#1076 perf trap).
    pub fn apply(
        &self,
        kv: Kv,
        content_type: ContentType,
        payload: Option<Vec<u8>>,
        resumes: Option<Vec<u8>>,
    ) -> Result<(Vec<EffectRequest>, Kv), (ComponentError, Kv)> {
        let mut store = wasmtime::Store::new(&self.engine, ReducerHost::new(kv));
        // Fuel metering is enabled on the engine (§22d). Instantiation isn't the DoS surface — a
        // reactive fold guest's runaway risk is in its `fold.apply` body, not its (structure-bounded)
        // instantiation — so give instantiation ample headroom, then reset fuel to the per-fold budget
        // right before the call. That way the budget bounds the FOLD precisely, and an exhausted budget
        // is unambiguously the guest's fold looping (not load cost). set_fuel can't fail with metering
        // on, but surface any error rather than unwrap. On any error, hand the base KV back (the overlay
        // is discarded, so it's the untouched pre-fold state) so the caller can keep it.
        if let Err(e) = store.set_fuel(u64::MAX) {
            let kv = store.into_data().into_kv();
            return Err((ComponentError::Instantiate(e.to_string()), kv));
        }
        // Instantiate the guest. FAST PATH (dependency-free): use the cached `instance_pre` — the link/
        // type-check was done ONCE at construction, so this fold just `.instantiate(store)`s it (perf
        // directive; no per-fold re-link). DEP PATH: this reducer's resolved dep components must be
        // composed into the linker in THIS fold's `store` (a dep instance can't outlive the store), so we
        // clone the base linker, compose each dep, and instantiate against the composed linker — can't
        // reuse a single pre-instantiation. (`instance_pre` is None exactly when there are deps.)
        let instance = match &self.instance_pre {
            Some(pre) => match pre.instantiate(&mut store) {
                Ok(i) => i,
                Err(e) => {
                    let kv = store.into_data().into_kv();
                    return Err((ComponentError::Instantiate(e.to_string()), kv));
                }
            },
            None => {
                // A dep-bearing reducer whose deps were never attached would fail with an opaque wasmtime
                // "missing imports" linker error — surface an ACTIONABLE one naming the builders instead
                // (parity with the async twin's guard; same class as #2203 c4 / #2244 / #2253).
                if self.resolved_deps.is_empty() && !self.deps.is_empty() {
                    let kv = store.into_data().into_kv();
                    return Err((
                        ComponentError::Instantiate(format!(
                            "reducer declares {} component dep(s) but none are attached — call \
                             with_resolved_deps (from resolve_deps) + with_component_store before folding",
                            self.deps.len()
                        )),
                        kv,
                    ));
                }
                let mut l = self.linker.clone();
                for (import_name, bytes) in &self.resolved_deps {
                    if let Err(e) = compose_dep_into_linker(
                        &self.engine,
                        &mut store,
                        &mut l,
                        import_name,
                        bytes,
                        self.component_store.as_ref(),
                    ) {
                        let kv = store.into_data().into_kv();
                        return Err((e, kv));
                    }
                }
                match Reducer::instantiate(&mut store, &self.component, &l) {
                    Ok(i) => i,
                    Err(e) => {
                        let kv = store.into_data().into_kv();
                        return Err((ComponentError::Instantiate(e.to_string()), kv));
                    }
                }
            }
        };
        // A `set_fuel` failure is a HOST-setup error (fuel metering not enabled on the engine), NOT a guest
        // trap — classify it as `Instantiate`, same as the load-phase `set_fuel(u64::MAX)` above. Mapping it
        // to `Trap` would tell the driver "the guest trapped" when the host mis-configured. (metering IS on
        // via `consume_fuel(true)` at construction, so this can't fail in practice — but the error path's
        // classification must not conflate host failure with guest semantics.)
        if let Err(e) = store.set_fuel(self.fuel_budget) {
            let kv = store.into_data().into_kv();
            return Err((ComponentError::Instantiate(e.to_string()), kv));
        }
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
                // differently (§22d / PR#1009). `Trap::OutOfFuel` is carried in the error chain. The KV
                // handed back is the base with its overlay DISCARDED — the guest's partial writes (which
                // only ever touched the overlay) vanish, so the fold is atomic.
                let kv = store.into_data().into_kv();
                if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
                    return Err((
                        ComponentError::FuelExhausted {
                            budget: self.fuel_budget,
                        },
                        kv,
                    ));
                }
                return Err((ComponentError::Trap(e.to_string()), kv));
            }
        };
        // Success: COMMIT the overlay into the base (the transactional boundary), then hand the KV back.
        let mut host = store.into_data();
        host.commit();
        let kv = host.into_kv();
        Ok((effects, kv))
    }

    /// The OPTION-C HANDLE-LOWERED fold (§19e): drive a real Cadenza reducer whose `fold.apply` is lowered
    /// to `apply(u32, u32, u32) -> u32` over the shared `cadenza:runtime/heap` value-heap, instead of the
    /// WIT-structural [`ComponentReducer::apply`] (which the bindgen path drives). rcdzc CANNOT emit the
    /// structural shape — every compound crosses the boundary as an opaque value-heap HANDLE (v-hb
    /// confirmed by compiling `reducer_b1.cdz`), so the host MARSHALS the `(content-type, payload, resumes)`
    /// inputs into three heap handles ([`crate::heap_marshal`]), calls the guest's `apply`, then UNMARSHALS
    /// the returned `list<effect-request>` handle back to effects ([`crate::heap_unmarshal`]).
    ///
    /// The runtime is a composed dep (§23): [`compose_dep_into_linker`] returns the runtime `Instance`, and
    /// the host binds a [`HeapHandle`] on THAT instance — so the handles the host builds and the ones the
    /// guest's `apply` reads index the ONE shared heap. `apply` is exported through the
    /// `cadenza:agent-kernel/fold` INTERFACE instance (not a top-level func), so it's reached via a nested
    /// `get_export_index`. Requires exactly one resolved runtime dep exporting `cadenza:runtime/heap`; a
    /// dependency-free reducer (no runtime) can't be handle-lowered (it has no heap to marshal on).
    ///
    /// Fuel/atomicity mirror [`ComponentReducer::apply`]: metered per-fold, `OutOfFuel` → `FuelExhausted`,
    /// any error hands the base KV back (the guest's overlay is discarded, so a failed fold is atomic). On
    /// success the overlay commits.
    pub fn apply_handle_lowered(
        &self,
        kv: Kv,
        content_type: ContentType,
        payload: Option<Vec<u8>>,
        resumes: Option<Vec<u8>>,
    ) -> Result<(Vec<EffectRequest>, Kv), (ComponentError, Kv)> {
        let mut store = wasmtime::Store::new(&self.engine, ReducerHost::new(kv));
        if let Err(e) = store.set_fuel(u64::MAX) {
            let kv = store.into_data().into_kv();
            return Err((ComponentError::Instantiate(e.to_string()), kv));
        }
        // Compose every declared dep into a per-fold linker, capturing the value-heap RUNTIME instance (the
        // dep exporting `cadenza:runtime/heap`) — that's the instance the host marshals on. A handle-lowered
        // reducer MUST declare a runtime dep; without one there's no heap, so this path doesn't apply.
        let mut l = self.linker.clone();
        let mut runtime_instance: Option<wasmtime::component::Instance> = None;
        for (import_name, bytes) in &self.resolved_deps {
            let inst = match compose_dep_into_linker(
                &self.engine,
                &mut store,
                &mut l,
                import_name,
                bytes,
                self.component_store.as_ref(),
            ) {
                Ok(inst) => inst,
                Err(e) => {
                    let kv = store.into_data().into_kv();
                    return Err((e, kv));
                }
            };
            // Match the runtime dep by its BARE interface name EXACTLY (via `bare_iface_name`, which strips
            // both `+<hash>` and `@<semver>` — the SAME parse `declared_deps` uses), NOT `starts_with(...)`.
            // A `starts_with` prefix would also match a future sibling like `cadenza:runtime/heap2@…`,
            // wrongly tripping the exactly-one fail-loud below (or mis-selecting it if alone). And a
            // `split('@')`-only strip UNDER-matches the `cadenza:runtime/heap+<hash>` (no `@`) form (the
            // `+hash` survives). This pairs with the fail-loud so both COUNT and IDENTITY are exact (#2208 c2 / #2219).
            if bare_iface_name(import_name) == "cadenza:runtime/heap" {
                // FAIL LOUD on a second runtime match (#2203 MED): silently overwriting `runtime_instance`
                // would let a non-deterministic resolved_deps order pick the wrong instance, breaking the
                // shared-heap invariant. The handle-lowered boundary binds ONE value-heap runtime.
                if runtime_instance.is_some() {
                    let kv = store.into_data().into_kv();
                    return Err((
                        ComponentError::Compose {
                            import_name: import_name.clone(),
                            reason:
                                "reducer resolved MORE THAN ONE cadenza:runtime/heap dep — the \
                                     handle-lowered boundary marshals on exactly one shared runtime"
                                    .to_string(),
                        },
                        kv,
                    ));
                }
                runtime_instance = Some(inst);
            }
        }
        let Some(runtime_instance) = runtime_instance else {
            let kv = store.into_data().into_kv();
            return Err((
                ComponentError::Compose {
                    import_name: "cadenza:runtime/heap".to_string(),
                    reason: "no resolved cadenza:runtime/heap dep composed — a handle-lowered reducer \
                             needs its value-heap runtime resolved (via with_resolved_deps) to marshal on"
                        .to_string(),
                },
                kv,
            ));
        };
        // Instantiate the reducer against the composed linker (its runtime import is now satisfied). Use a
        // plain `Linker::instantiate` for the RAW `Instance` (not the bindgen `Reducer` wrapper, which only
        // exposes the structural `call_apply`) — the handle-lowered `apply` is reached generically via
        // `get_export_index`, so we need the raw instance.
        let reducer_instance = match l.instantiate(&mut store, &self.component) {
            Ok(i) => i,
            Err(e) => {
                let kv = store.into_data().into_kv();
                return Err((ComponentError::Instantiate(e.to_string()), kv));
            }
        };
        // Reach the INTERFACE-NESTED `apply`: `cadenza:agent-kernel/fold` instance → its `apply` func. Both
        // the runtime instance and this func handle live in `store`; extract the `Func` (Copy) BEFORE
        // `HeapHandle::bind` takes ownership of the store, then call it via `heap.store_mut()`.
        let apply_func = match reach_fold_apply(&mut store, &reducer_instance) {
            Ok(f) => f,
            Err(e) => {
                let kv = store.into_data().into_kv();
                return Err((e, kv));
            }
        };
        // Bind a HeapHandle on the SHARED runtime instance (consumes the store — we drive the reducer's
        // apply through `heap.store_mut()` from here). Set the per-fold fuel budget now (post-instantiation).
        // On bind failure recover the base KV from the store bind hands back (#2203 MED: never drop it —
        // the "any error hands the base KV back" atomicity contract, same as every other arm here).
        let mut heap = match HeapHandle::bind(store, &runtime_instance) {
            Ok(h) => h,
            Err((e, store)) => return Err((e, store.into_data().into_kv())),
        };
        heap.set_fuel_budget(self.fuel_budget);
        if let Err(e) = heap.store_mut().set_fuel(self.fuel_budget) {
            let kv = heap.into_store().into_data().into_kv();
            return Err((ComponentError::Instantiate(e.to_string()), kv));
        }
        // MARSHAL the inputs into three value-heap handles on the shared runtime.
        let (ct_h, payload_h, resumes_h) = match crate::heap_marshal::marshal_fold_inputs(
            &mut heap,
            (content_type.family.as_str(), content_type.version),
            payload.as_deref(),
            resumes.as_deref(),
        ) {
            Ok(hs) => hs,
            Err(e) => {
                let kv = heap.into_store().into_data().into_kv();
                return Err((e, kv));
            }
        };
        // CALL the guest's `apply(ct, payload, resumes) -> list-handle` over u32 handles.
        let result_handle = match heap.call_apply_lowered(&apply_func, ct_h, payload_h, resumes_h) {
            Ok(h) => h,
            Err(e) => {
                let kv = heap.into_store().into_data().into_kv();
                return Err((e, kv));
            }
        };
        // UNMARSHAL the returned `list<effect-request>` handle back to effects.
        let effects = match crate::heap_unmarshal::read_effect_requests(&mut heap, result_handle) {
            Ok(es) => es,
            Err(e) => {
                let kv = heap.into_store().into_data().into_kv();
                return Err((e, kv));
            }
        };
        // Success: COMMIT the overlay into the base KV, hand it back.
        let mut host = heap.into_store().into_data();
        host.commit();
        Ok((effects, host.into_kv()))
    }
}

/// Reach the INTERFACE-NESTED `apply` export of a handle-lowered reducer: navigate into the
/// `cadenza:agent-kernel/fold` interface instance, then its `apply` func (§19e — a real Cadenza reducer
/// exports `apply` THROUGH the fold interface, not as a top-level func, so a plain top-level lookup fails).
fn reach_fold_apply(
    store: &mut wasmtime::Store<ReducerHost>,
    instance: &wasmtime::component::Instance,
) -> Result<wasmtime::component::Func, ComponentError> {
    let err = |reason: String| ComponentError::InvokeExport {
        interface: "cadenza:agent-kernel/fold".to_string(),
        func: "apply".to_string(),
        reason,
    };
    let iface_idx = instance
        .get_export_index(&mut *store, None, "cadenza:agent-kernel/fold")
        .ok_or_else(|| err("reducer exports no cadenza:agent-kernel/fold interface".into()))?;
    let apply_idx = instance
        .get_export_index(&mut *store, Some(&iface_idx), "apply")
        .ok_or_else(|| err("fold interface exports no apply func".into()))?;
    instance
        .get_func(&mut *store, apply_idx)
        .ok_or_else(|| err("fold apply export is not a func".into()))
}

/// Drive a WASM `ComponentReducer` through the kernel's [`crate::reducer::Reducer`] loop (§19b/§19e
/// slice 2b-ii). This adapter is the bridge: it translates a kernel [`Event`] into the guest's `apply`
/// inputs `(content_type, payload, resumes)`, runs the fold, and maps the guest's returned effect
/// requests (which carry the guest's own `correlation` token) into kernel [`crate::reducer::Effect`]s
/// (`{request, token}`). So a real wasm reducer folds on the same loop the in-process Rust `Reducer`
/// trait uses — the operator's §19b real boundary, wired in.
///
/// CORRELATION (§19e, ruling B — kernel-owns): the guest never sees the kernel `EffectId`. On a
/// result/timer/denial event, `resumes` = the event's `token`, which the kernel put there from the
/// event's origin — copied from the originating `Dispatched` frame for an `EffectResult` (slice 2b-i)
/// or `TimerArmed` frame for a `TimerFired` (slice 2b-iii), or moved straight from the requesting effect
/// for an `AuthzDenied` (which has no prior durable frame — the effect never ran). Either way `fold`
/// reads it straight off `event`, staying PURE (no log/map access). On an effect the guest emits, its
/// `correlation` becomes the `Effect.token` the drive loop records into the new `Dispatched` (or
/// `TimerArmed`) frame. The loop closes: emit token → Dispatched/TimerArmed → Result/Fired/Denial →
/// resumes.
///
/// TOTALITY (§17): a guest trap / fuel-exhaustion / instantiation failure is surfaced as NO effects
/// (an empty fold) rather than a panic — the kernel treats a fold that produced nothing as quiescent.
/// (A future ABI refinement, §16c gap A, may distinguish a trapped fold from a clean empty one; v0
/// fails safe by emitting nothing so a broken reducer can't brick the loop.)
#[async_trait::async_trait(?Send)]
impl crate::reducer::Reducer for ComponentReducer {
    /// Native `Reducer` — but NOTE: `ComponentReducer` runs a SYNC wasm engine, so `fold` calls
    /// the sync `apply` with no `.await` (it does not cooperatively yield mid-fold). The fuel-yielding
    /// async wasm path is [`AsyncComponentReducer`], which now ALSO composes §23 deps per-fold (same as
    /// this sync path). `ComponentReducer` remains for callers on the sync engine; a future consolidation
    /// can collapse it into `AsyncComponentReducer` now that both are dep-capable.
    async fn fold(&self, event: &Event, kv: &mut Kv) -> crate::reducer::FoldOutput {
        // Map the kernel event → the guest's (content_type, payload, resumes) inputs.
        let (content_type, payload, resumes) = event_to_guest_inputs(&event.body);

        // Move the session KV into the fold WITHOUT cloning (PR#1076 perf): `Kv` is a `BTreeMap`, so a
        // `clone()` would deep-copy the whole session state every event → O(KV size) per fold. `mem::take`
        // swaps in an empty KV (O(1)) and hands the real one to `apply`, which returns it in BOTH arms.
        // On Ok we install the guest's committed mutations; on error we restore the base `apply` handed
        // back — which is byte-for-byte the pre-fold state (the guest's writes were buffered in an overlay
        // that `apply` discarded), so a trapped/fuel-exhausted fold leaves the session KV ATOMICALLY
        // untouched (PR#1076/#1150 error-atomicity — now a real guarantee, not just a comment).
        let taken = std::mem::take(kv);
        match self.apply(taken, content_type, payload, resumes) {
            Ok((guest_effects, new_kv)) => {
                *kv = new_kv;
                // Map each WIT-boundary effect-request → the kernel Effect via the ONE shared converter
                // (see `guest_effect_to_kernel_effect`; #2166 — the two EffectRequest types).
                let effects = guest_effects
                    .into_iter()
                    .map(guest_effect_to_kernel_effect)
                    .collect();
                crate::reducer::FoldOutput::with_effects(effects)
            }
            // Trap / fuel-exhausted / instantiate failure → fail safe: no effects, and RESTORE the base
            // KV `apply` handed back (mandatory: we `mem::take`-d `kv` out above, so a missing restore
            // would leave it empty). Because `apply` discarded the guest's overlay, this base is the
            // exact pre-fold state — the failed fold is atomic. The guest's contract is totality; a
            // violation can't brick the loop (§17).
            // A guest trap / fuel-exhaustion / instantiate failure is a FOLD FAILURE — surface it as a
            // first-class `FoldOutput::failed(reason)` so the kernel records a `FoldFailed` log event a
            // supervisor can observe (error-resilience: NOT a silent empty fold "into the void"). The KV
            // is still restored to the pre-fold state (atomic); no effects are emitted (§17 can't-brick).
            Err((err, restored_kv)) => {
                *kv = restored_kv;
                crate::reducer::FoldOutput::failed(format!("wasm reducer fold failed: {err:?}"))
            }
        }
    }
}

/// A wasm reducer that folds ASYNCHRONOUSLY (operator all-async directive) — the cooperative-gas-yield
/// counterpart of [`ComponentReducer`]. Its engine is configured with `async_support(true)` and
/// `fuel_async_yield_interval`, so a long `fold.apply` YIELDS at fuel intervals (letting the single-
/// threaded host loop interleave other sessions) instead of blocking, while the per-fold fuel BUDGET still
/// traps a true runaway. It implements [`crate::reducer::Reducer`] natively (its `fold` awaits
/// the guest's async `call_apply_async`) — the single async `Reducer` trait (the all-async arc removed
/// the sync twin + its adapter), so a pure-Rust and a wasm reducer both `impl Reducer` directly.
///
/// SEPARATE from [`ComponentReducer`] because `async_support` is a per-ENGINE flag: a sync call panics on
/// an async store and vice versa, so the async fold needs its own async-configured engine + the async
/// `bindgen!` lowering (`async_reducer_bindings`). During the sync→async migration both coexist; the sync
/// [`ComponentReducer`] is removed once every caller is async (the operator's "no sync path remains").
///
/// Handles BOTH the dependency-free path (cached `instance_pre` fast path) AND §23 dep-bearing reducers:
/// the latter compose their resolved deps (plus transitive bare imports like the runtime's
/// `cadenza:nfc/normalize`) into a per-fold linker in the fold's store, then `instantiate_async` — the
/// async twin of the sync [`ComponentReducer`]'s per-fold dep-compose. Attach deps via
/// [`AsyncComponentReducer::with_resolved_deps`] and [`AsyncComponentReducer::with_component_store`]. (This
/// is what unblocks the host genesis E2E, which drives a dep-bearing genesis reducer through async `apply`.)
pub struct AsyncComponentReducer {
    engine: wasmtime::Engine,
    // Pre-instantiation artifact (perf, same rationale as ComponentReducer::instance_pre): the async
    // world's `ReducerPre`, built once at construction for the DEPENDENCY-FREE fold-exporting reducer.
    // `None` for a reducer WITH declared deps — that path composes its deps into a per-fold linker in the
    // fold's store (the dep instances can't outlive a fold's store, so no single pre-instantiation can be
    // reused), exactly as the sync `ComponentReducer` does. So `instance_pre.is_some()` iff the fast path
    // applies (dependency-free AND deps not force-detached via `with_resolved_deps`).
    instance_pre: Option<async_reducer_bindings::ReducerPre<ReducerHost>>,
    // The component + base linker (with the `kv` host import), kept for the DEP path: `apply` clones the
    // linker per fold, composes the resolved deps into it (via `compose_dep_into_linker`), and instantiates
    // `component` against it in the fold's store. Unused by the dependency-free `instance_pre` fast path.
    component: wasmtime::component::Component,
    linker: wasmtime::component::Linker<ReducerHost>,
    // The component deps this reducer DECLARES by content hash (§23), detected at construction. Mirrors
    // `ComponentReducer::deps`: exposed via `deps()` so a caller can discover → `resolve_deps()` from CAS →
    // `with_resolved_deps` (the §23 flow). Empty = dependency-free.
    deps: Vec<ComponentDep>,
    // Resolved dependency bytes (§23) paired with the import name each satisfies, + the store to resolve a
    // dep's OWN transitive bare imports (the runtime's `cadenza:nfc/normalize`). Mirrors ComponentReducer's
    // `resolved_deps`/`component_store`. Attached via `with_resolved_deps`/`with_component_store`.
    resolved_deps: Vec<(String, Vec<u8>)>,
    component_store: Option<crate::component_store::ComponentStore>,
    fuel_budget: u64,
    // How much fuel the guest may burn between cooperative yields (§ async directive): the store yields
    // control every this-many fuel units so a long fold doesn't monopolize the single-threaded loop. The
    // per-fold `fuel_budget` is still the hard ceiling that traps a runaway (both: yield=cooperation,
    // budget=DoS trap).
    fuel_yield_interval: u64,
}

/// Default cooperative-yield interval (§ async directive): the guest yields control roughly every this
/// many fuel units. Smaller = more responsive interleaving, more yield overhead; a coarse default that a
/// long fold still yields under well before its billion-fuel budget.
pub const DEFAULT_FUEL_YIELD_INTERVAL: u64 = 1_000_000;

impl AsyncComponentReducer {
    /// Build an async reducer from a compiled component's bytes. Like
    /// [`ComponentReducer::from_component_bytes`] but the engine is `async_support`-enabled and the guest
    /// export is lowered async. Pre-instantiates the dependency-FREE fold world once (perf); a component that
    /// declares §23 deps skips pre-instantiation (`instance_pre = None`) and composes per-fold instead —
    /// attach its resolved deps via [`AsyncComponentReducer::with_resolved_deps`]. A dependency-free component
    /// that doesn't export the `fold` world is declined (`Instantiate`/`InvalidComponent`).
    pub fn from_component_bytes(bytes: &[u8]) -> Result<Self, ComponentError> {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        // The all-async engine: a long fold cooperatively yields (fuel_async_yield_interval, set per-store
        // in fold) instead of blocking; sync calls would panic on this engine (per-engine flag).
        config.async_support(true);
        let engine = wasmtime::Engine::new(&config)
            .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
        let component = wasmtime::component::Component::new(&engine, bytes)
            .map_err(|e| ComponentError::InvalidComponent(e.to_string()))?;
        // §23 async dep-compose: a reducer with declared deps composes them into a per-fold linker (in the
        // fold's store) exactly as the sync path does — NO longer declined. Detect the declared set now so
        // `apply` knows whether to take the dep-compose branch; the resolved bytes arrive via
        // `with_resolved_deps` (a real dep-bearing reducer needs them wired before folding).
        let deps = declared_deps(&component, &engine)?;
        let mut linker = wasmtime::component::Linker::<ReducerHost>::new(&engine);
        async_reducer_bindings::Reducer::add_to_linker::<
            _,
            wasmtime::component::HasSelf<ReducerHost>,
        >(&mut linker, |h: &mut ReducerHost| h)
        .map_err(|e| ComponentError::Link(e.to_string()))?;
        // Pre-instantiate the fold world ONCE (perf) — ONLY for the dependency-FREE path (like the sync
        // ComponentReducer). A dep-bearing reducer's linker is composed PER-FOLD in the fold's store, so it
        // can't reuse a single pre-instantiation; `instance_pre` stays None and `apply` composes per fold.
        // (A dependency-free component that doesn't export the fold world fails here — a real decline.)
        let instance_pre = if deps.is_empty() {
            let pre = linker
                .instantiate_pre(&component)
                .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
            Some(
                async_reducer_bindings::ReducerPre::new(pre)
                    .map_err(|e| ComponentError::Instantiate(e.to_string()))?,
            )
        } else {
            None
        };
        Ok(AsyncComponentReducer {
            engine,
            instance_pre,
            component,
            linker,
            deps,
            resolved_deps: Vec::new(),
            component_store: None,
            fuel_budget: DEFAULT_FOLD_FUEL,
            fuel_yield_interval: DEFAULT_FUEL_YIELD_INTERVAL,
        })
    }

    /// The component dependencies this async reducer DECLARES by content hash (§23). Empty = dependency-free.
    /// A caller composes each by fetching its bytes from CAS (see [`AsyncComponentReducer::resolve_deps`]),
    /// then attaches via [`AsyncComponentReducer::with_resolved_deps`]. Mirrors [`ComponentReducer::deps`].
    pub fn deps(&self) -> &[ComponentDep] {
        &self.deps
    }

    /// Resolve ALL declared dependency bytes from a blob store (§23), each paired with the import name the
    /// linker binds it under — the async twin of [`ComponentReducer::resolve_deps`]. Empty vec if
    /// dependency-free. `Err(DepMissing)`/`Err(DepStoreError)` split as in the sync path (PR#1013 #3).
    pub async fn resolve_deps(
        &self,
        blobs: &dyn crate::blob::BlobStore,
    ) -> Result<Vec<(ComponentDep, Vec<u8>)>, ComponentError> {
        let mut out = Vec::with_capacity(self.deps.len());
        for dep in &self.deps {
            let bytes = resolve_dep_bytes(dep, blobs).await?;
            out.push((dep.clone(), bytes));
        }
        Ok(out)
    }

    /// Attach the resolved bytes of this reducer's declared deps (§23), so async `apply` composes each into
    /// its per-fold linker before instantiating. Mirrors [`ComponentReducer::with_resolved_deps`]. Passing a
    /// non-empty set for a component whose `instance_pre` was cached (dependency-free) forces the per-fold
    /// path (`instance_pre = None`) — deps must compose in the fold's store.
    pub fn with_resolved_deps(mut self, resolved: Vec<(ComponentDep, Vec<u8>)>) -> Self {
        self.resolved_deps = resolved
            .into_iter()
            .map(|(dep, bytes)| (dep.import_name, bytes))
            .collect();
        if !self.resolved_deps.is_empty() {
            self.instance_pre = None;
        }
        self
    }

    /// Attach a [`ComponentStore`](crate::component_store::ComponentStore) to resolve a dep's OWN transitive
    /// bare imports (the runtime's `cadenza:nfc/normalize`), mirroring
    /// [`ComponentReducer::with_component_store`]. Required for a runtime dep whose world imports nfc.
    pub fn with_component_store(mut self, store: crate::component_store::ComponentStore) -> Self {
        self.component_store = Some(store);
        self
    }

    /// Fold ONE event through the wasm guest ASYNCHRONOUSLY (the async twin of [`ComponentReducer::apply`]).
    /// Same transactional + no-full-KV-clone contract: `kv` moves in, is returned in BOTH arms; the guest's
    /// writes hit the [`ReducerHost`] overlay, committed only on success (a trapped/fuel-exhausted fold
    /// leaves KV atomically untouched). The difference is the guest call `.await`s (`call_apply_async`) and
    /// the store is armed with `fuel_async_yield_interval` so a long fold yields cooperatively.
    pub async fn apply(
        &self,
        kv: Kv,
        content_type: ContentType,
        payload: Option<Vec<u8>>,
        resumes: Option<Vec<u8>>,
    ) -> Result<(Vec<EffectRequest>, Kv), (ComponentError, Kv)> {
        // Bridge the PUBLIC (sync-module re-exported) `ContentType` to the async bindgen module's own
        // generated type — the two `bindgen!`s produce distinct structs, so the async guest call needs its
        // module's `ContentType`. Same fields; a trivial field copy at the boundary.
        let content_type = async_reducer_bindings::cadenza::agent_kernel::types::ContentType {
            family: content_type.family,
            version: content_type.version,
        };
        let mut store = wasmtime::Store::new(&self.engine, ReducerHost::new(kv));
        // Cooperative yield: the guest yields control every `fuel_yield_interval` fuel so a long fold
        // doesn't monopolize the single-threaded loop. Set alongside the fuel budget below (yield-interval
        // = cooperation, set_fuel-ceiling = the DoS trap — both, §22d + async directive).
        if let Err(e) = store.fuel_async_yield_interval(Some(self.fuel_yield_interval)) {
            let kv = store.into_data().into_kv();
            return Err((ComponentError::Instantiate(e.to_string()), kv));
        }
        // Ample fuel for instantiation, then reset to the per-fold budget right before the call (same as
        // the sync path: the budget bounds the FOLD precisely, not load cost).
        if let Err(e) = store.set_fuel(u64::MAX) {
            let kv = store.into_data().into_kv();
            return Err((ComponentError::Instantiate(e.to_string()), kv));
        }
        // Instantiate the guest. FAST PATH (dependency-free): the cached `instance_pre`. DEP PATH: clone the
        // base linker, compose each resolved dep into it per-fold (with transitive nfc via the store), then
        // `instantiate_async` against the composed linker — the async twin of the sync `apply`'s None-branch
        // (dep instances live in THIS fold's store, so no single pre-instantiation can be reused).
        let instance = match &self.instance_pre {
            Some(pre) => match pre.instantiate_async(&mut store).await {
                Ok(i) => i,
                Err(e) => {
                    let kv = store.into_data().into_kv();
                    return Err((ComponentError::Instantiate(e.to_string()), kv));
                }
            },
            None => {
                // A dep-bearing reducer whose deps were never attached would fail with an opaque wasmtime
                // "missing imports" linker error — surface an ACTIONABLE one naming the builders instead
                // (reviewer #2253, same class as #2203 c4 / #2244).
                if self.resolved_deps.is_empty() && !self.deps.is_empty() {
                    let kv = store.into_data().into_kv();
                    return Err((
                        ComponentError::Instantiate(format!(
                            "async reducer declares {} component dep(s) but none are attached — call \
                             with_resolved_deps (from resolve_deps) + with_component_store before folding",
                            self.deps.len()
                        )),
                        kv,
                    ));
                }
                let mut l = self.linker.clone();
                for (import_name, bytes) in &self.resolved_deps {
                    // ASYNC composer: on the async_support engine the sync compose_dep_into_linker's inner
                    // `.instantiate` (and its forwarded sync `Func::call`) PANIC — use the async twin
                    // (#2256 async dep-compose; the panic v-ah-host's live genesis E2E hit).
                    if let Err(e) = compose_dep_into_linker_async(
                        &self.engine,
                        &mut store,
                        &mut l,
                        import_name,
                        bytes,
                        self.component_store.as_ref(),
                    )
                    .await
                    {
                        let kv = store.into_data().into_kv();
                        return Err((e, kv));
                    }
                }
                match async_reducer_bindings::Reducer::instantiate_async(
                    &mut store,
                    &self.component,
                    &l,
                )
                .await
                {
                    Ok(i) => i,
                    Err(e) => {
                        let kv = store.into_data().into_kv();
                        return Err((ComponentError::Instantiate(e.to_string()), kv));
                    }
                }
            }
        };
        // A `set_fuel` failure is a HOST-setup error (fuel metering not enabled on the engine), NOT a guest
        // trap — classify it as `Instantiate`, same as the load-phase `set_fuel(u64::MAX)` above. Mapping it
        // to `Trap` would tell the driver "the guest trapped" when the host mis-configured. (metering IS on
        // via `consume_fuel(true)` at construction, so this can't fail in practice — but the error path's
        // classification must not conflate host failure with guest semantics.)
        if let Err(e) = store.set_fuel(self.fuel_budget) {
            let kv = store.into_data().into_kv();
            return Err((ComponentError::Instantiate(e.to_string()), kv));
        }
        let effects = match instance
            .cadenza_agent_kernel_fold()
            .call_apply(
                &mut store,
                &content_type,
                payload.as_deref(),
                resumes.as_deref(),
            )
            .await
        {
            Ok(effects) => effects,
            Err(e) => {
                let kv = store.into_data().into_kv();
                if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
                    return Err((
                        ComponentError::FuelExhausted {
                            budget: self.fuel_budget,
                        },
                        kv,
                    ));
                }
                return Err((ComponentError::Trap(e.to_string()), kv));
            }
        };
        let mut host = store.into_data();
        host.commit();
        let kv = host.into_kv();
        // Bridge the async-bindgen guest `EffectRequest`s back to the PUBLIC (sync-module) `EffectRequest`
        // the crate exposes everywhere else — structurally identical, distinct generated types.
        let effects = effects
            .into_iter()
            .map(|g| EffectRequest {
                kind: async_guest_kind_to_public(&g.kind),
                target: g.target,
                payload: g.payload,
                correlation: g.correlation,
            })
            .collect();
        Ok((effects, kv))
    }
}

#[async_trait::async_trait(?Send)]
impl crate::reducer::Reducer for AsyncComponentReducer {
    async fn fold(&self, event: &Event, kv: &mut Kv) -> crate::reducer::FoldOutput {
        let (content_type, payload, resumes) = event_to_guest_inputs(&event.body);
        let taken = std::mem::take(kv);
        match self.apply(taken, content_type, payload, resumes).await {
            Ok((guest_effects, new_kv)) => {
                *kv = new_kv;
                // Same WIT-boundary → kernel Effect conversion as the sync path, via the shared converter.
                let effects = guest_effects
                    .into_iter()
                    .map(guest_effect_to_kernel_effect)
                    .collect();
                crate::reducer::FoldOutput::with_effects(effects)
            }
            Err((err, restored_kv)) => {
                *kv = restored_kv;
                crate::reducer::FoldOutput::failed(format!(
                    "async wasm reducer fold failed: {err:?}"
                ))
            }
        }
    }
}

/// Map the ASYNC-bindgen guest `effect-kind` to the PUBLIC (sync-module) [`EffectKind`] — the two
/// `bindgen!` modules generate DISTINCT `EffectKind` enums, so `apply` bridges the async guest's effects
/// back to the public type the crate exposes. Same variants (both mirror the one WIT enum).
fn async_guest_kind_to_public(
    k: &async_reducer_bindings::cadenza::agent_kernel::types::EffectKind,
) -> EffectKind {
    use async_reducer_bindings::cadenza::agent_kernel::types::EffectKind as AsyncKind;
    match k {
        AsyncKind::Shell => EffectKind::Shell,
        AsyncKind::Http => EffectKind::Http,
        AsyncKind::Model => EffectKind::Model,
        AsyncKind::Now => EffectKind::Now,
        AsyncKind::Timer => EffectKind::Timer,
        AsyncKind::Emit => EffectKind::Emit,
    }
}

/// An [`crate::authz::Authorize`] backed by a wasm POLICY COMPONENT (operator ruling: Cedar-as-wasm,
/// §10/SEC-F1). Holds the same wasmtime `Engine` + compiled policy `Component` + a `AuthorizerWorldPre`
/// (pre-instantiated once — the policy world imports nothing, so a fresh empty linker suffices), and
/// decides each request by instantiating the policy into a throwaway store, calling its exported
/// `authorize(request) -> decision`, and mapping the verdict to the kernel's `Result<(), String>`. This
/// is the component-model authz path that drops in wherever the flat-capability [`crate::authz::Authorizer`]
/// does — a Cedar policy set compiled to a component (built by v-agent-harness-host) is the intended
/// guest; construction here is guest-agnostic (any component exporting the `authorizer` world works).
///
/// FAIL-CLOSED (§10 + §17): a policy trap / instantiate failure is a DENY (a policy that can't decide
/// must not accidentally permit), never a panic — so a broken policy fails safe, not open.
pub struct ComponentAuthorizer {
    engine: wasmtime::Engine,
    pre: self::authz_bindings::AuthorizerWorldPre<()>,
    /// The principal (session/agent identity) every request is authorized under — Cedar's PRINCIPAL.
    /// v0 holds one principal per authorizer (the session it guards); delegation/on-behalf-of (§12f)
    /// layers on later.
    principal: String,
}

impl ComponentAuthorizer {
    /// Build a policy authorizer from a compiled policy component's bytes, authorizing every request
    /// under `principal` (the session/agent identity). The policy world imports nothing, so the linker
    /// is empty; pre-instantiate once so each `authorize` just instantiates into a fresh store. Errors
    /// if the bytes aren't a valid component exporting the `authorizer` world.
    pub fn from_policy_bytes(
        bytes: &[u8],
        principal: impl Into<String>,
    ) -> Result<Self, ComponentError> {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        let engine = wasmtime::Engine::new(&config)
            .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
        let component = wasmtime::component::Component::new(&engine, bytes)
            .map_err(|e| ComponentError::InvalidComponent(e.to_string()))?;
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let pre = linker
            .instantiate_pre(&component)
            .and_then(self::authz_bindings::AuthorizerWorldPre::new)
            .map_err(|e| ComponentError::Instantiate(e.to_string()))?;
        Ok(ComponentAuthorizer {
            engine,
            pre,
            principal: principal.into(),
        })
    }

    /// The engine (exposed for tests / advanced host composition).
    pub fn engine(&self) -> &wasmtime::Engine {
        &self.engine
    }
}

#[async_trait::async_trait(?Send)]
impl crate::authz::Authorize for ComponentAuthorizer {
    /// Decide via the policy component: map the `EffectRequest` to the PARC triple (principal, action =
    /// content-type FAMILY, target = resolved target), instantiate the policy into a fresh store, call
    /// `authorize`, and map the verdict. FAIL-CLOSED: any instantiate/trap failure denies (§10 safe
    /// default), never panics (§17). A generous fuel budget bounds a runaway policy without tripping a
    /// legitimate decision. Native `Authorize` — the policy instantiate/call is a SYNC wasm engine
    /// call today (no `.await`); a fuel-yielding async policy eval is a later refinement (the trait is
    /// async so it drops in without a signature change).
    async fn authorize(&self, req: &crate::effect::EffectRequest) -> Result<(), String> {
        let mut store = wasmtime::Store::new(&self.engine, ());
        if store.set_fuel(DEFAULT_FOLD_FUEL).is_err() {
            return Err("authz: fuel init failed (fail-closed deny)".to_string());
        }
        let world = match self.pre.instantiate(&mut store) {
            Ok(w) => w,
            Err(e) => {
                return Err(format!(
                    "authz: policy instantiate failed (fail-closed deny): {e}"
                ))
            }
        };
        let request = self::authz_bindings::cadenza::agent_kernel_authz::types::AuthRequest {
            principal: self.principal.clone(),
            // ACTION = the content-type FAMILY (seq-39 source of truth), NOT the EffectKind enum. The flat
            // `Authorizer` gates on `req.content_type.family` (via `matches_family`), so the policy component
            // MUST see the same string or the two authz paths disagree. Critically, a register-by-string
            // family (a `store/*` set/resolve, or any extension family) carries the `Emit` PLACEHOLDER kind —
            // keying `action` on the enum would present "emit" to the policy for a `store/set`, so a Cedar
            // policy could never gate store writes (or any register-by-string family) correctly.
            action: req.content_type.family.to_string(),
            target: req.target.to_string(),
        };
        match world
            .cadenza_agent_kernel_authz_authorizer()
            .call_authorize(&mut store, &request)
        {
            Ok(decision) if decision.allow => Ok(()),
            Ok(decision) => Err(if decision.reason.is_empty() {
                "denied by policy".to_string()
            } else {
                decision.reason
            }),
            // A trapped policy is a fail-CLOSED deny — a policy that can't decide must not permit.
            Err(e) => Err(format!("authz: policy trapped (fail-closed deny): {e}")),
        }
    }
}

/// Map a kernel [`EventBody`] to the guest `fold.apply` inputs `(content_type, payload, resumes)`.
/// `resumes` (§19e ruling B) is the event's continuation token, already copied onto result/timer events
/// from their originating `Dispatched` frame (slice-2b-i) — so this reads it off the event, never a map.
fn event_to_guest_inputs(body: &EventBody) -> (ContentType, Option<Vec<u8>>, Option<Vec<u8>>) {
    // A synthetic content-type for the kernel-internal event kinds the guest folds (results, timers,
    // denials): the guest matches on `family` to know what arrived. Inbound carries its OWN content-type.
    let synthetic = |family: &str| ContentType {
        family: family.to_string(),
        version: 1,
    };
    match body {
        EventBody::Inbound {
            content_type,
            payload,
        } => (
            ContentType {
                // The guest-binding ContentType.family is `String`; the kernel's is now `Cow` — convert
                // at the host→guest boundary.
                family: content_type.family.to_string(),
                version: content_type.version,
            },
            Some(payload_bytes(payload)),
            None,
        ),
        EventBody::EffectResult { result, token, .. } => (
            synthetic("effect-result"),
            effect_outcome_bytes(result),
            token.clone(),
        ),
        // TimerFired / AuthzDenied are ALSO terminal outcomes a guest resumes on (resumes_effect
        // recognizes all three), so they carry the guest's continuation token too, via the same (B)
        // mechanism as EffectResult (slice 2b-iii): a timer's token is copied from its originating
        // `TimerArmed` frame when it fires; a denial's token is moved from the requesting effect (a
        // denial has no prior durable frame). `fold` reads it straight off the event as `resumes`,
        // staying pure. The full effect→result / timer→fire / request→denial resume cycle is now wired.
        EventBody::TimerFired {
            fired_ms, token, ..
        } => (
            synthetic("timer-fired"),
            Some(fired_ms.to_le_bytes().to_vec()),
            token.clone(),
        ),
        EventBody::AuthzDenied { reason, token, .. } => (
            synthetic("authz-denied"),
            Some(reason.clone().into_bytes()),
            token.clone(),
        ),
        // Genesis / Dispatched / TimerArmed / Closed are not folded by the reducer (they're kernel
        // bookkeeping or setup — see the kernel's `observable()` predicate); the loop never calls fold
        // on them, but map defensively to an empty-payload synthetic content-type rather than panic.
        EventBody::Genesis { .. } => (synthetic("genesis"), None, None),
        EventBody::Dispatched { .. } => (synthetic("dispatched"), None, None),
        EventBody::TimerArmed { .. } => (synthetic("timer-armed"), None, None),
        EventBody::Closed { .. } => (synthetic("closed"), None, None),
        // FoldFailed is a kernel-recorded failure event, not a fold input (the loop never folds it —
        // `observable()` excludes it); map defensively rather than panic.
        EventBody::FoldFailed { .. } => (synthetic("fold-failed"), None, None),
    }
}

/// Opaque bytes of a kernel payload (inline bytes verbatim; a blob-by-hash surfaces its 32 hash bytes —
/// the guest resolves the blob via its own means, out of scope for v0's fold-input mapping).
fn payload_bytes(p: &crate::effect::Payload) -> Vec<u8> {
    match p {
        crate::effect::Payload::Inline(b) => b.to_vec(),
        crate::effect::Payload::Blob(h) => h.as_bytes().to_vec(),
    }
}

/// Bytes the guest sees for an effect result: the Ok payload's bytes, the Err message, or empty for a
/// timeout. (A richer tagged encoding is a later ABI concern; v0 hands the guest the result's content.)
fn effect_outcome_bytes(o: &EffectOutcome) -> Option<Vec<u8>> {
    match o {
        EffectOutcome::Ok(Some(p)) => Some(payload_bytes(p)),
        EffectOutcome::Ok(None) => None,
        EffectOutcome::Err(msg) => Some(msg.clone().into_bytes()),
        EffectOutcome::TimedOut => None,
    }
}

/// Map the guest `effect-kind` (WIT enum) to the kernel [`crate::effect::EffectKind`]. Same variants
/// (the WIT mirrors the kernel enum); this is the type-boundary translation.
fn guest_kind_to_kernel(k: &EffectKind) -> crate::effect::EffectKind {
    match k {
        EffectKind::Shell => crate::effect::EffectKind::Shell,
        EffectKind::Http => crate::effect::EffectKind::Http,
        EffectKind::Model => crate::effect::EffectKind::Model,
        EffectKind::Now => crate::effect::EffectKind::Now,
        EffectKind::Timer => crate::effect::EffectKind::Timer,
        EffectKind::Emit => crate::effect::EffectKind::Emit,
    }
}

/// Convert a guest `effect-request` (the WIT component-boundary [`EffectRequest`], as returned by a fold)
/// into the kernel's [`crate::reducer::Effect`] — the ONE place the boundary→kernel translation lives.
///
/// The two `EffectRequest` types are distinct (github-liaison #2166): [`EffectRequest`] here is the
/// WIT-generated type a `fold.apply` returns; [`crate::effect::EffectRequest`] is the kernel's own struct.
/// This maps between them: the kind via [`guest_kind_to_kernel`], the opaque payload bytes → an
/// `Inline` kernel payload (`None` stays `None`; a `Vec<u8>` freezes into ref-counted `Bytes` via `.into()`),
/// and the guest's `correlation` token → the kernel `Effect`'s continuation `token` (→ the `Dispatched`
/// frame). Guest effects are always `Timeliness::Interactive` — per-effect batchability is not on the
/// reducer WIT surface, so a guest cannot declare it (that would be a WIT-surface extension).
///
/// Extracted from the two `Reducer::fold` impls ([`ComponentReducer`] + [`AsyncComponentReducer`]), which
/// built this inline identically; the fold-boundary handle-ABI rebind reuses it too, so the boundary→kernel
/// mapping is defined once (no drift between the WIT-structural path and the handle-lowered path).
fn guest_effect_to_kernel_effect(g: EffectRequest) -> crate::reducer::Effect {
    crate::reducer::Effect {
        request: crate::effect::EffectRequest::new(
            guest_kind_to_kernel(&g.kind),
            g.target,
            g.payload.map(|p| crate::effect::Payload::Inline(p.into())),
            crate::effect::Timeliness::Interactive,
        ),
        token: g.correlation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `bare_iface_name` strips BOTH `+<hash>` and `@<semver>`, and the runtime-dep selection must agree with
    // `declared_deps`' `rsplit_once('+')` parse for EVERY import-name form — including the `+<hash>`-with-no-`@`
    // form that a `split('@')`-only strip under-matched (#2219). Pins all forms + a sibling non-match.
    #[test]
    fn bare_iface_name_strips_hash_and_version_for_every_form() {
        assert_eq!(
            bare_iface_name("cadenza:runtime/heap@0.0.0+abc123"),
            "cadenza:runtime/heap"
        );
        // `+<hash>` with NO `@<semver>` — the form the #2219 fix targets (split('@') alone left `+abc123`).
        assert_eq!(
            bare_iface_name("cadenza:runtime/heap+abc123"),
            "cadenza:runtime/heap"
        );
        // `@<semver>` with no `+<hash>`.
        assert_eq!(
            bare_iface_name("cadenza:runtime/heap@0.0.0"),
            "cadenza:runtime/heap"
        );
        // Already bare — unchanged.
        assert_eq!(
            bare_iface_name("cadenza:runtime/heap"),
            "cadenza:runtime/heap"
        );
        // A sibling interface must NOT collapse to the runtime name (the over-match `starts_with` allowed).
        assert_eq!(
            bare_iface_name("cadenza:runtime/heap2@0.0.0+def456"),
            "cadenza:runtime/heap2"
        );
    }

    // The shared WIT-boundary → kernel Effect converter (extracted from both fold impls; #2166). Pins the
    // full mapping: kind translated, a Some payload → an Inline kernel payload with the same bytes, and the
    // guest correlation token → the kernel Effect's continuation token. A companion case pins that a None
    // payload / None correlation stay None (fire-and-forget), so the Some/None distinction isn't collapsed.
    #[test]
    fn guest_effect_converts_to_kernel_effect_with_payload_and_token() {
        let g = EffectRequest {
            kind: EffectKind::Http,
            target: "https://example.test".to_string(),
            payload: Some(b"body".to_vec()),
            correlation: Some(b"tok-1".to_vec()),
        };
        let e = guest_effect_to_kernel_effect(g);
        assert_eq!(e.request.kind, crate::effect::EffectKind::Http);
        assert_eq!(e.request.target.as_ref(), "https://example.test");
        assert_eq!(e.request.timeliness, crate::effect::Timeliness::Interactive);
        match &e.request.payload {
            Some(crate::effect::Payload::Inline(b)) => assert_eq!(b.as_ref(), b"body"),
            other => panic!("expected an Inline payload with the guest bytes, got {other:?}"),
        }
        assert_eq!(e.token.as_deref(), Some(&b"tok-1"[..]));
    }

    #[test]
    fn guest_effect_none_payload_and_token_stay_none() {
        let g = EffectRequest {
            kind: EffectKind::Emit,
            target: "peer-session".to_string(),
            payload: None,
            correlation: None,
        };
        let e = guest_effect_to_kernel_effect(g);
        assert_eq!(e.request.kind, crate::effect::EffectKind::Emit);
        assert!(
            e.request.payload.is_none(),
            "None payload must stay None (not empty-Inline)"
        );
        assert!(
            e.token.is_none(),
            "None correlation must stay None (fire-and-forget)"
        );
    }

    // The host `kv` import is backed by the kernel KV THROUGH the transactional overlay: reads see the
    // guest's own uncommitted writes (read-your-writes), but the base isn't mutated until `commit`.
    #[test]
    fn host_kv_import_is_backed_by_the_kernel_kv() {
        use self::cadenza::agent_kernel::kv::Host;
        let mut host = ReducerHost::new(Kv::new());
        assert_eq!(host.get(b"k".to_vec()), None);
        host.put(b"k".to_vec(), b"v".to_vec());
        assert_eq!(host.get(b"k".to_vec()), Some(b"v".to_vec())); // read-your-writes via overlay
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
        assert!(host.delete(b"k".to_vec())); // existed (via overlay) → true
        assert_eq!(host.get(b"k".to_vec()), None); // tombstone hides it
        assert!(!host.delete(b"k".to_vec())); // already tombstoned → false
                                              // COMMIT persists the overlay into the base; only the two live puts survive (k was deleted).
        host.commit();
        assert_eq!(host.into_kv().len(), 2);
    }

    // Transactional atomicity at the host level: writes buffered in the overlay do NOT reach the base
    // KV unless `commit` runs. A host dropped WITHOUT commit (the errored-fold path) yields the exact
    // pre-fold base — this is what makes a trapped fold atomic (PR#1076/#1150).
    #[test]
    fn host_kv_writes_are_discarded_without_commit() {
        use self::cadenza::agent_kernel::kv::Host;
        let mut base = Kv::new();
        base.put(b"keep".to_vec(), b"1".to_vec());
        let mut host = ReducerHost::new(base);
        // The guest writes + deletes through the import...
        host.put(b"new".to_vec(), b"2".to_vec());
        assert!(host.delete(b"keep".to_vec()));
        assert_eq!(host.get(b"keep".to_vec()), None); // overlay shows the tombstone
                                                      // ...but WITHOUT commit, into_kv returns the untouched base: "new" absent, "keep" intact.
        let out = host.into_kv();
        assert_eq!(out.len(), 1);
        assert_eq!(out.get(b"keep"), Some(&b"1"[..]));
        assert_eq!(out.get(b"new"), None);
    }

    // The ComponentReducer CONSTRUCTION path wires up (Engine + Component + Linker + the generated
    // kv-import registration) on a real — if trivial — component. The `apply` fold against a real
    // fold-exporting guest is covered end-to-end in `tests/component_reducer_e2e.rs`.
    #[tokio::test(flavor = "current_thread")]
    async fn component_reducer_builds_engine_component_and_registers_kv_import() {
        // A valid, minimal component (exports nothing — enough to prove Component::new + the linker's
        // kv-import registration succeed; the fold against a real fold-exporting guest is the e2e test).
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
                .await
                .unwrap(),
            vec![]
        );
    }

    #[test]
    fn component_reducer_rejects_invalid_bytes() {
        // Garbage bytes are a MALFORMED-COMPONENT condition → `InvalidComponent`, and it must be that
        // variant SPECIFICALLY: the classification is about the bytes (the `Component::new` check), not
        // a host/engine-setup failure (which is `Instantiate` — `Engine::new` succeeds here). Pins the
        // C1 distinction so a future refactor can't collapse "bad bytes" into "host couldn't run it."
        // (Ok variant isn't Debug, so match rather than .unwrap_err().)
        match ComponentReducer::from_component_bytes(b"not a wasm component") {
            Err(ComponentError::InvalidComponent(_)) => {}
            Err(other) => panic!("expected InvalidComponent (bad bytes), got {other:?}"),
            Ok(_) => panic!("garbage bytes must not build a component"),
        }
    }

    // ComponentAuthorizer construction (§10 Cedar-as-wasm authz). A real allow/deny DECISION test needs a
    // policy component exporting the `authorizer` world (a cedar-policy wit-bindgen guest — v-agent-harness-
    // host's fixture, mirroring reducer-guest); these pin the construction contract kernel-side now.
    #[test]
    fn component_authorizer_rejects_garbage_and_a_non_authorizer_component() {
        use crate::authz::Authorize;
        // Garbage bytes → InvalidComponent (the bytes aren't a component at all).
        match ComponentAuthorizer::from_policy_bytes(b"not a policy component", "session-1") {
            Err(ComponentError::InvalidComponent(_)) => {}
            Err(other) => panic!("expected InvalidComponent, got {other:?}"),
            Ok(_) => panic!("garbage must not build a policy authorizer"),
        }
        // A valid component that does NOT export the `authorizer` world → Instantiate (pre-instantiation
        // type-checks the world's exports, so a non-policy component is rejected at construction).
        let empty = wat::parse_str("(component)").expect("empty component");
        match ComponentAuthorizer::from_policy_bytes(&empty, "session-1") {
            Err(ComponentError::Instantiate(_)) => {}
            Err(other) => panic!("expected Instantiate (no authorizer export), got {other:?}"),
            Ok(_) => panic!("a non-authorizer component must not build a policy authorizer"),
        }
        // (A ComponentAuthorizer over a real policy denies fail-closed on a policy trap — exercised e2e
        // once the Cedar policy guest exists; the trait impl's Err arms encode the fail-closed contract.)
        let _ = <ComponentAuthorizer as Authorize>::authorize; // name-check the impl exists
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

    // §23 dep-compose (slice ii): a dependency COMPONENT composed into a consumer's linker satisfies the
    // consumer's like-named interface import, and a call through the consumer reaches the dep — proving
    // The reducer-boundary marshalling adapter (operator ruling C): a synthetic `cadenza:runtime/heap`
    // stub exporting the BUILD + READ ops lets us drive `HeapHandle`'s marshalling methods without the real
    // (frozen-hash) runtime. BUILD ops return sentinel handles (box-int→100, sum-new→300, str-new→400;
    // arr-set threads the arr; arr-alloc→2 for len==0 [inline-unit] / 200 for len>0 [#2133 discriminating]).
    // READ ops return recognizable sentinels so a test can prove the host reaches each + reads the shape:
    // vec-len→0, vec-get→700, arr-get→800, sum-disc→1, sum-payload→900, get-int→42, str-get→"target".
    fn heap_stub_component() -> Vec<u8> {
        wat::parse_str(
            r#"(component
                 (core module $m
                   (memory (export "mem") 1)
                   (global $strret (mut i32) (i32.const 0))
                   (func (export "realloc") (param i32 i32 i32 i32) (result i32) (local.get 0))
                   (func (export "box-int") (param i64) (result i32) (i32.const 100))
                   ;; arr-alloc(len): len==0 → 2 (inline-unit sentinel); len>0 → 200 (#2133 discriminating).
                   (func (export "arr-alloc") (param $len i32) (result i32)
                     (select (i32.const 200) (i32.const 2) (local.get $len)))
                   (func (export "arr-set") (param i32 i32 i32) (result i32) (local.get 0))
                   (func (export "sum-new") (param i32 i32) (result i32) (i32.const 300))
                   (func (export "vec-len") (param i32) (result i32) (i32.const 0))
                   (func (export "str-new") (param i32 i32) (result i32) (i32.const 400))
                   ;; read ops: recognizable sentinels
                   (func (export "vec-get") (param i32 i32) (result i32) (i32.const 700))
                   (func (export "arr-get") (param i32 i32) (result i32) (i32.const 800))
                   (func (export "sum-disc") (param i32) (result i32) (i32.const 1))
                   (func (export "sum-payload") (param i32) (result i32) (i32.const 900))
                   (func (export "get-int") (param i32) (result i64) (i64.const 42))
                   ;; str-get returns "target" (6 bytes) — write to a fixed area, return (ptr,len).
                   (data (i32.const 8) "target")
                   ;; @100 a BOGUS bytes handle: header reports len=0xFFFFFFFF, no real bytes. read_bytes(100)
                   ;; must NOT eager-reserve ~4GB (alloc-abort) — the cap bounds it, then bytes-get walks past
                   ;; the single page and Traps cleanly (#2166 read-side DoS guard).
                   (data (i32.const 100) "\ff\ff\ff\ff")
                   (func (export "str-get") (param i32) (result i32)
                     (i32.store (i32.const 4096) (i32.const 8))
                     (i32.store (i32.const 4100) (i32.const 6))
                     (i32.const 4096))
                   ;; FUNCTIONAL bytes ops (real round-trip, not sentinels): a buffer HANDLE is a memory
                   ;; offset to [len:i32][bytes…], bump-allocated from 8192. So bytes_from→read_bytes and
                   ;; the marshalling can actually round-trip a byte payload through the stub.
                   (global $bnext (mut i32) (i32.const 8192))
                   (func (export "bytes-alloc") (param $len i32) (result i32)
                     (local $h i32)
                     (local.set $h (global.get $bnext))
                     (i32.store (local.get $h) (local.get $len))            ;; store len at handle
                     (global.set $bnext (i32.add (global.get $bnext) (i32.add (local.get $len) (i32.const 4))))
                     (local.get $h))
                   (func (export "bytes-set") (param $buf i32) (param $i i32) (param $v i32) (result i32)
                     (i32.store8 (i32.add (i32.add (local.get $buf) (i32.const 4)) (local.get $i)) (local.get $v))
                     (local.get $buf))                                      ;; thread the handle
                   (func (export "bytes-len") (param $buf i32) (result i32)
                     (i32.load (local.get $buf)))
                   (func (export "bytes-get") (param $buf i32) (param $i i32) (result i32)
                     (i32.load8_u (i32.add (i32.add (local.get $buf) (i32.const 4)) (local.get $i)))))
                 (core instance $i (instantiate $m))
                 (func $box-int (param "v" s64) (result u32) (canon lift (core func $i "box-int")))
                 (func $arr-alloc (param "len" u32) (result u32) (canon lift (core func $i "arr-alloc")))
                 (func $arr-set (param "arr" u32) (param "index" u32) (param "elem" u32) (result u32) (canon lift (core func $i "arr-set")))
                 (func $sum-new (param "disc" u32) (param "payload" u32) (result u32) (canon lift (core func $i "sum-new")))
                 (func $vec-len (param "v" u32) (result u32) (canon lift (core func $i "vec-len")))
                 (func $str-new (param "s" string) (result u32) (canon lift (core func $i "str-new") (memory $i "mem") (realloc (func $i "realloc"))))
                 (func $vec-get (param "v" u32) (param "index" u32) (result u32) (canon lift (core func $i "vec-get")))
                 (func $arr-get (param "arr" u32) (param "index" u32) (result u32) (canon lift (core func $i "arr-get")))
                 (func $sum-disc (param "handle" u32) (result u32) (canon lift (core func $i "sum-disc")))
                 (func $sum-payload (param "handle" u32) (result u32) (canon lift (core func $i "sum-payload")))
                 (func $get-int (param "handle" u32) (result s64) (canon lift (core func $i "get-int")))
                 (func $str-get (param "handle" u32) (result string) (canon lift (core func $i "str-get") (memory $i "mem") (realloc (func $i "realloc"))))
                 (func $bytes-alloc (param "len" u32) (result u32) (canon lift (core func $i "bytes-alloc")))
                 (func $bytes-set (param "buf" u32) (param "index" u32) (param "value" u32) (result u32) (canon lift (core func $i "bytes-set")))
                 (func $bytes-len (param "buf" u32) (result u32) (canon lift (core func $i "bytes-len")))
                 (func $bytes-get (param "buf" u32) (param "index" u32) (result u32) (canon lift (core func $i "bytes-get")))
                 (instance $heap
                   (export "box-int" (func $box-int))
                   (export "arr-alloc" (func $arr-alloc))
                   (export "arr-set" (func $arr-set))
                   (export "sum-new" (func $sum-new))
                   (export "vec-len" (func $vec-len))
                   (export "str-new" (func $str-new))
                   (export "vec-get" (func $vec-get))
                   (export "arr-get" (func $arr-get))
                   (export "sum-disc" (func $sum-disc))
                   (export "sum-payload" (func $sum-payload))
                   (export "get-int" (func $get-int))
                   (export "str-get" (func $str-get))
                   (export "bytes-alloc" (func $bytes-alloc))
                   (export "bytes-set" (func $bytes-set))
                   (export "bytes-len" (func $bytes-len))
                   (export "bytes-get" (func $bytes-get)))
                 (export "cadenza:runtime/heap" (instance $heap)))"#,
        )
        .expect("assemble heap stub component")
    }

    // HeapHandle binds each B1 heap op off a composed cadenza:runtime/heap instance + drives them — the
    // host-side marshalling foundation (option C). Proves: bind finds every op, and the B1 INPUT-marshal
    // sequence runs (build a content-type record `arr[0]=str-new(family), arr[1]=box-int(version)`, a None
    // option `sum-new(1, unit)`, and read an empty effect-list `vec-len==0`).
    #[test]
    fn heap_handle_binds_and_drives_the_b1_build_ops() {
        let bytes = heap_stub_component();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let component =
            wasmtime::component::Component::new(&engine, &bytes).expect("valid heap stub");
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("heap stub instantiates");
        // `.expect` would need HeapHandle: Debug (it holds non-Debug Func/Store) — match instead.
        let mut heap = match HeapHandle::bind(store, &instance) {
            Ok(h) => h,
            Err((e, _)) => panic!("bind HeapHandle to the heap iface: {e:?}"),
        };

        // B1 ct record: arr-alloc(2); arr[0]=str-new("demo"); arr[1]=box-int(1). The stub threads the arr
        // handle (arr-alloc→200), so the record handle is 200; the field handles are the op sentinels.
        let family = heap.str_new("demo").expect("str-new");
        assert_eq!(family, 400);
        let version = heap.box_int(1).expect("box-int");
        assert_eq!(version, 100);
        let ct = heap.arr_alloc(2).expect("arr-alloc");
        assert_eq!(ct, 200);
        assert_eq!(heap.arr_set(ct, 0, family).expect("arr-set 0"), 200); // threads the arr
        assert_eq!(heap.arr_set(ct, 1, version).expect("arr-set 1"), 200);
        // None option payload: sum-new(1=None disc, UNIT) — the unit is the inline-unit handle from
        // arr-alloc(0) (github-liaison #2122: NOT handle 0, which is NULL → a malformed sum).
        let unit = heap.unit().expect("unit (arr-alloc 0)");
        // #2133: unit() must call arr-alloc(0) — the stub returns 2 for len==0 (distinct from the len>0
        // sentinel 200), so this asserts unit() actually allocs LENGTH ZERO, not just any array.
        assert_eq!(
            unit, 2,
            "unit() must call arr-alloc(0), the inline-unit — not arr-alloc(non-zero)"
        );
        let none_payload = heap.sum_new(1, unit).expect("sum-new None");
        assert_eq!(none_payload, 300);
        // Read the (empty) effect-list result length.
        assert_eq!(heap.vec_len(999).expect("vec-len"), 0);
    }

    // HeapHandle slice-2 READ ops: project a returned effect-request list back to structural values. Drives
    // each read op off the stub (sentinels: vec-get→700, arr-get→800, sum-disc→1, sum-payload→900,
    // get-int→42, str-get→"target"), proving the host reaches every read op + gets the right value KIND
    // (u32 handle / s64 / String) — the B2/B3 effect-request-reading half.
    #[test]
    fn heap_handle_drives_the_read_ops() {
        let bytes = heap_stub_component();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let component =
            wasmtime::component::Component::new(&engine, &bytes).expect("valid heap stub");
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("heap stub instantiates");
        let mut heap = match HeapHandle::bind(store, &instance) {
            Ok(h) => h,
            Err((e, _)) => panic!("bind HeapHandle: {e:?}"),
        };
        // Walk a returned effect-list: vec-get(list, 0) → an effect-request record handle.
        assert_eq!(heap.vec_get(555, 0).expect("vec-get"), 700);
        // Project a record field by sorted index → a field handle.
        assert_eq!(heap.arr_get(700, 1).expect("arr-get"), 800);
        // Decode a boxed scalar field.
        assert_eq!(heap.get_int(800).expect("get-int"), 42);
        // Decode a String field (an effect target).
        assert_eq!(heap.str_get(800).expect("str-get"), "target");
        // Decode an Option/enum-kind discriminant + its payload.
        assert_eq!(heap.sum_disc(300).expect("sum-disc"), 1);
        assert_eq!(heap.sum_payload(300).expect("sum-payload"), 900);
    }

    // HeapHandle slice-2b BYTES ops: build a value-heap Bytes handle from a Rust slice + read it back —
    // for the reducer's Option<Bytes> payload/resumes args + emitted effect-request payload/correlation.
    // The stub's bytes ops are FUNCTIONAL (real memory-backed buffer), so this is a true ROUND-TRIP, not a
    // sentinel check: bytes_from(data) → read_bytes → data, incl. empty + arbitrary (non-UTF8) bytes.
    #[test]
    fn heap_handle_round_trips_bytes() {
        let bytes = heap_stub_component();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let component =
            wasmtime::component::Component::new(&engine, &bytes).expect("valid heap stub");
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("heap stub instantiates");
        let mut heap = match HeapHandle::bind(store, &instance) {
            Ok(h) => h,
            Err((e, _)) => panic!("bind HeapHandle: {e:?}"),
        };
        // Arbitrary bytes (incl. 0x00 and 0xFF, non-UTF8) round-trip exactly.
        let payload = [0xDE, 0xAD, 0x00, 0xFF, 0x2A];
        let h = heap.bytes_from(&payload).expect("bytes_from");
        assert_eq!(heap.bytes_len(h).expect("bytes-len"), 5);
        assert_eq!(heap.read_bytes(h).expect("read_bytes"), payload);
        // Empty bytes round-trip too (an empty payload).
        let empty = heap.bytes_from(&[]).expect("bytes_from empty");
        assert_eq!(
            heap.read_bytes(empty).expect("read empty"),
            Vec::<u8>::new()
        );
        // A distinct second buffer doesn't alias the first (bump-allocated).
        let other = heap.bytes_from(&[1, 2, 3]).expect("bytes_from other");
        assert_eq!(heap.read_bytes(other).expect("read other"), [1, 2, 3]);
        assert_eq!(heap.read_bytes(h).expect("re-read first"), payload); // first still intact
    }

    // DoS guard (reviewer, #2166 read-side): a bytes handle whose bytes-len reports a bogus huge length
    // (the forged @100 handle = 0xFFFFFFFF, no real bytes) must NOT abort the host via an eager ~4GB
    // Vec::with_capacity — the MAX_PREALLOC_BYTES cap bounds the reservation, then bytes-get walks past the
    // single-page memory and Traps cleanly. The test returning (not OOM-aborting) is the proof.
    #[test]
    fn read_bytes_with_bogus_huge_len_traps_cleanly_not_an_alloc_abort() {
        let bytes = heap_stub_component();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let component =
            wasmtime::component::Component::new(&engine, &bytes).expect("valid heap stub");
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("heap stub instantiates");
        let mut heap = match HeapHandle::bind(store, &instance) {
            Ok(h) => h,
            Err((e, _)) => panic!("bind HeapHandle: {e:?}"),
        };
        match heap.read_bytes(100) {
            Err(_) => {} // clean fail-loud (a bytes-get memory Trap), NOT a process abort
            Ok(v) => panic!(
                "a bogus u32::MAX bytes-len must not read as a real {}-byte buffer",
                v.len()
            ),
        }
    }

    // A runtime that doesn't export cadenza:runtime/heap → a clear Compose error naming it, not a trap.
    #[test]
    fn heap_handle_bind_errors_clearly_without_the_heap_interface() {
        let empty = wat::parse_str("(component)").expect("empty component");
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let linker = wasmtime::component::Linker::<()>::new(&engine);
        let component = wasmtime::component::Component::new(&engine, &empty).expect("valid");
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("instantiates");
        match HeapHandle::bind(store, &instance) {
            Err((ComponentError::Compose { import_name, .. }, _store)) => {
                assert_eq!(import_name, "cadenza:runtime/heap");
            }
            Err((e, _store)) => {
                panic!("expected a Compose error naming the heap interface, got {e:?}")
            }
            Ok(_) => panic!("expected bind to fail without the heap interface, but it succeeded"),
        }
    }

    // the runtime-agnostic linker-composition mechanism (mirrors cdz-run::run_with_peers). Synthetic
    // components (wat), so no wit-bindgen fixture toolchain needed.
    #[test]
    fn a_dependency_component_composed_into_the_linker_satisfies_a_consumer_import() {
        // Dep: exports interface `test:dep/api` with `answer: func() -> u32` returning 42.
        let dep_bytes = wat::parse_str(
            r#"(component
                 (core module $m (func (export "answer") (result i32) i32.const 42))
                 (core instance $i (instantiate $m))
                 (func $answer (result u32) (canon lift (core func $i "answer")))
                 (instance $api (export "answer" (func $answer)))
                 (export "test:dep/api" (instance $api)))"#,
        )
        .expect("assemble dep component");

        // Consumer: IMPORTS `test:dep/api` (answer: () -> u32) and EXPORTS `run: () -> u32` = answer()+1.
        let consumer_bytes = wat::parse_str(
            r#"(component
                 (import "test:dep/api" (instance $api (export "answer" (func (result u32)))))
                 (core module $m
                   (import "" "answer" (func $answer (result i32)))
                   (func (export "run") (result i32)
                     (i32.add (call $answer) (i32.const 1))))
                 (core func $answer_core (canon lower (func $api "answer")))
                 (core instance $shim (export "answer" (func $answer_core)))
                 (core instance $i (instantiate $m (with "" (instance $shim))))
                 (func $run (result u32) (canon lift (core func $i "run")))
                 (export "run" (func $run)))"#,
        )
        .expect("assemble consumer component");

        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let mut linker = wasmtime::component::Linker::<()>::new(&engine);

        // Compose the dep into the linker under the interface name the consumer imports.
        compose_dep_into_linker(
            &engine,
            &mut store,
            &mut linker,
            "test:dep/api",
            &dep_bytes,
            None,
        )
        .expect("compose dep into linker");

        // The consumer now instantiates (its import is satisfied) and `run()` reaches the dep → 43.
        let consumer = wasmtime::component::Component::new(&engine, &consumer_bytes)
            .expect("valid consumer component");
        let instance = linker
            .instantiate(&mut store, &consumer)
            .expect("consumer instantiates against the composed dep");
        let run = {
            let idx = instance
                .get_export_index(&mut store, None, "run")
                .expect("consumer exports run");
            instance.get_func(&mut store, idx).expect("run is a func")
        };
        let mut results = [wasmtime::component::Val::U32(0)];
        run.call(&mut store, &[], &mut results).expect("call run");
        assert_eq!(
            results[0],
            wasmtime::component::Val::U32(43),
            "run() = dep.answer()(42) + 1 — the call crossed the composed boundary"
        );
    }

    // §23 TRANSITIVE compose (FINDING#23): a dep that ITSELF imports the bare `cadenza:nfc/normalize`
    // (as the real value-heap runtime does) composes ONLY when a ComponentStore is provided to resolve nfc
    // by name (`runtime.toml`'s `nfc = "<hash>"`); WITHOUT a store the same dep fails to compose with a
    // clear `Compose` error naming nfc. Uses tiny wat components + a real temp store dir.
    #[test]
    fn a_dep_importing_bare_nfc_composes_via_the_store_and_fails_loud_without_it() {
        // NFC leaf: exports `cadenza:nfc/normalize` with `normalize: func(u32) -> u32` (echoes its arg).
        let nfc_bytes = wat::parse_str(
            r#"(component
                 (core module $m (func (export "normalize") (param i32) (result i32) (local.get 0)))
                 (core instance $i (instantiate $m))
                 (func $normalize (param "s" u32) (result u32) (canon lift (core func $i "normalize")))
                 (instance $nfc (export "normalize" (func $normalize)))
                 (export "cadenza:nfc/normalize" (instance $nfc)))"#,
        )
        .expect("assemble nfc leaf component");

        // Runtime-like dep: IMPORTS bare `cadenza:nfc/normalize`, EXPORTS `cadenza:runtime/heap` with
        // `str-nfc-normalize: func(u32)->u32` that calls the imported nfc (mirrors the real runtime's nfc
        // dependency). The consumer of THIS dep imports `cadenza:runtime/heap`.
        let runtime_bytes = wat::parse_str(
            r#"(component
                 (import "cadenza:nfc/normalize" (instance $nfc (export "normalize" (func (param "s" u32) (result u32)))))
                 (core func $nfc_core (canon lower (func $nfc "normalize")))
                 (core module $m
                   (import "" "normalize" (func $normalize (param i32) (result i32)))
                   (func (export "str-nfc-normalize") (param i32) (result i32) (call $normalize (local.get 0))))
                 (core instance $shim (export "normalize" (func $nfc_core)))
                 (core instance $i (instantiate $m (with "" (instance $shim))))
                 (func $snn (param "s" u32) (result u32) (canon lift (core func $i "str-nfc-normalize")))
                 (instance $heap (export "str-nfc-normalize" (func $snn)))
                 (export "cadenza:runtime/heap" (instance $heap)))"#,
        )
        .expect("assemble runtime-like dep that imports nfc");

        // A real temp store: `<hash>.wasm` for the nfc bytes + a `runtime.toml` with `nfc = "<hash>"`.
        // The hash is the SHA-256 content address (the store's algorithm — component_store verifies it).
        let store_dir =
            std::env::temp_dir().join(format!("cdzstore-nfc-compose-{}", std::process::id()));
        std::fs::create_dir_all(&store_dir).unwrap();
        let nfc_hex = {
            use sha2::{Digest, Sha256};
            let d = Sha256::digest(&nfc_bytes);
            let mut s = String::with_capacity(64);
            for b in d {
                use std::fmt::Write as _;
                let _ = write!(s, "{b:02x}");
            }
            s
        };
        std::fs::write(store_dir.join(format!("{nfc_hex}.wasm")), &nfc_bytes).unwrap();
        std::fs::write(
            store_dir.join("runtime.toml"),
            format!("nfc = \"{nfc_hex}\"\n"),
        )
        .unwrap();
        let comp_store = crate::component_store::ComponentStore::open(&store_dir);

        let engine = wasmtime::Engine::default();

        // WITHOUT a store → compose fails loud, naming the unresolved nfc import.
        {
            let mut store = wasmtime::Store::new(&engine, ());
            let mut linker = wasmtime::component::Linker::<()>::new(&engine);
            match compose_dep_into_linker(
                &engine,
                &mut store,
                &mut linker,
                "cadenza:runtime/heap",
                &runtime_bytes,
                None,
            ) {
                Err(ComponentError::Compose { import_name, .. }) => {
                    assert_eq!(
                        import_name, "cadenza:nfc/normalize",
                        "the error names the unresolved nfc dep"
                    )
                }
                other => {
                    panic!("expected a Compose error naming nfc without a store, got {other:?}")
                }
            }
        }

        // WITH the store → the runtime dep's nfc import is transitively composed, so it instantiates and
        // its `str-nfc-normalize` (which calls nfc) is reachable through the composed `cadenza:runtime/heap`.
        {
            let mut store = wasmtime::Store::new(&engine, ());
            let mut linker = wasmtime::component::Linker::<()>::new(&engine);
            let rt_instance = compose_dep_into_linker(
                &engine,
                &mut store,
                &mut linker,
                "cadenza:runtime/heap",
                &runtime_bytes,
                Some(&comp_store),
            )
            .expect("transitive nfc compose should succeed with a store");
            // Reach str-nfc-normalize on the composed runtime instance → it calls nfc(7) = 7.
            let idx = rt_instance
                .get_export_index(&mut store, None, "cadenza:runtime/heap")
                .expect("runtime exports its heap interface");
            let fidx = rt_instance
                .get_export_index(&mut store, Some(&idx), "str-nfc-normalize")
                .expect("heap exports str-nfc-normalize");
            let f = rt_instance
                .get_func(&mut store, fidx)
                .expect("str-nfc-normalize is a func");
            let mut results = [wasmtime::component::Val::U32(0)];
            f.call(
                &mut store,
                &[wasmtime::component::Val::U32(7)],
                &mut results,
            )
            .expect("call str-nfc-normalize");
            f.post_return(&mut store).ok();
            assert_eq!(
                results[0],
                wasmtime::component::Val::U32(7),
                "str-nfc-normalize(7) reached the transitively-composed nfc component"
            );
        }
        std::fs::remove_dir_all(&store_dir).ok();
    }

    // §23 dep-compose: composing bytes that DON'T export the imported interface fails with a clear
    // `Compose` error naming the interface — not an opaque instantiate trap later.
    #[test]
    fn composing_a_dep_that_lacks_the_imported_interface_errors_clearly() {
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let mut linker = wasmtime::component::Linker::<()>::new(&engine);
        // An empty component exports nothing → can't satisfy `test:dep/api`.
        let empty = wat::parse_str("(component)").expect("empty component");
        match compose_dep_into_linker(
            &engine,
            &mut store,
            &mut linker,
            "test:dep/api",
            &empty,
            None,
        ) {
            Err(ComponentError::Compose { import_name, .. }) => {
                assert_eq!(import_name, "test:dep/api");
            }
            other => panic!("expected Compose error naming the interface, got {other:?}"),
        }
    }

    // §19e fold-boundary rebind FOUNDATION: `compose_dep_into_linker` returns the dep's live `Instance`, and
    // for the value-heap runtime dep that returned instance is exactly what the host binds a `HeapHandle`
    // on — so the reducer (which calls the runtime through the linker) and the host's marshalling drive the
    // SAME heap instance. This composes the heap stub as a `cadenza:runtime/heap` dep, binds a HeapHandle on
    // the RETURNED instance, and drives an op — proving the returned instance is a usable, bindable runtime
    // (the store-sharing the handle-lowered `apply` relies on: build arg handles on this instance, then the
    // reducer's `apply` reads them off the same heap). No reducer here — that's the next slice; this pins
    // that the compose→bind seam works end to end.
    #[test]
    fn compose_returns_a_bindable_runtime_instance_for_the_heap_handle() {
        let bytes = heap_stub_component();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let mut linker = wasmtime::component::Linker::<()>::new(&engine);
        // Compose the heap stub under the runtime interface name a reducer imports.
        let runtime_instance = match compose_dep_into_linker(
            &engine,
            &mut store,
            &mut linker,
            "cadenza:runtime/heap",
            &bytes,
            None,
        ) {
            Ok(inst) => inst,
            Err(e) => panic!("compose the runtime dep: {e:?}"),
        };
        // Bind a HeapHandle on the SAME composed instance + store, then drive a build op — proving the
        // returned instance is the usable runtime the host marshals on (the stub's box-int → sentinel 100).
        let mut heap = match HeapHandle::bind(store, &runtime_instance) {
            Ok(h) => h,
            Err((e, _)) => panic!("bind HeapHandle to the composed runtime instance: {e:?}"),
        };
        assert_eq!(
            heap.box_int(7).expect("box-int on the composed runtime"),
            100,
            "the composed runtime instance's heap ops are drivable via HeapHandle"
        );
    }

    // #2256 ASYNC dep-compose regression: `compose_dep_into_linker_async` must compose a dep on an
    // `async_support` ENGINE without panicking. Before the async twins existed, the async fold path
    // (`AsyncComponentReducer::apply`) reused the SYNC composer, whose inner `Linker::instantiate` (and its
    // forwarded sync `Func::call`) PANIC on an async store ("must use async instantiation" / "must use
    // call_async") — the exact panic v-ah-host's live genesis E2E hit. This mirrors the sync
    // `compose_returns_a_bindable_runtime_instance_for_the_heap_handle` but on an async engine + the async
    // composer: compose the heap stub, then drive an op through the returned instance to prove it's usable.
    #[tokio::test(flavor = "current_thread")]
    async fn compose_dep_into_linker_async_composes_on_an_async_engine() {
        let bytes = heap_stub_component();
        // An async_support engine — the whole point (a sync compose would panic here).
        let mut config = wasmtime::Config::new();
        config.async_support(true);
        let engine = wasmtime::Engine::new(&config).expect("async engine");
        let mut store = wasmtime::Store::new(&engine, ());
        let mut linker = wasmtime::component::Linker::<()>::new(&engine);
        let runtime_instance = match compose_dep_into_linker_async(
            &engine,
            &mut store,
            &mut linker,
            "cadenza:runtime/heap",
            &bytes,
            None,
        )
        .await
        {
            Ok(inst) => inst,
            Err(e) => panic!("async-compose the runtime dep: {e:?}"),
        };
        // The returned instance is a usable runtime: drive one of its heap ops (box-int → stub sentinel 100)
        // via a plain async call, proving instantiate_async produced a live, callable instance.
        use wasmtime::component::Val;
        let idx = runtime_instance
            .get_export_index(&mut store, None, "cadenza:runtime/heap")
            .expect("heap iface export");
        let op_idx = runtime_instance
            .get_export_index(&mut store, Some(&idx), "box-int")
            .expect("box-int export");
        let box_int = runtime_instance
            .get_func(&mut store, op_idx)
            .expect("box-int func");
        let mut results = [Val::U32(0)];
        box_int
            .call_async(&mut store, &[Val::S64(7)], &mut results)
            .await
            .expect("call box-int on the async-composed runtime");
        assert_eq!(
            results[0],
            Val::U32(100),
            "the async-composed runtime instance's heap ops are drivable"
        );
    }

    // Content-addressed dep NAME MATCHING (real-reducer regression): a real Cadenza reducer IMPORTS its
    // runtime under the full content-addressed name `cadenza:runtime/heap@0.0.0+<hash>`, but the runtime
    // component EXPORTS the BARE `cadenza:runtime/heap`. compose_dep_into_linker must look the dep's export
    // up by the bare name (strip `@version+hash`) while forwarding into the linker under the full import
    // name — verified against the compiled reducer_b1 + its runtime, which the WAT stubs (same name both
    // sides) had masked. Here: compose the heap stub (exports bare `cadenza:runtime/heap`) under a FULL
    // `@version+hash` import name, and confirm the returned instance is bindable + drivable.
    #[test]
    fn compose_matches_a_bare_dep_export_against_a_versioned_hashed_import_name() {
        let bytes = heap_stub_component();
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(&engine, ());
        let mut linker = wasmtime::component::Linker::<()>::new(&engine);
        // Import name carries the @version+hash the bare-exporting stub does NOT have in its export name.
        let import_name = "cadenza:runtime/heap@0.0.0+39358be448eac4e8afe25add5977767f814c0f9a6cad714cb778d223839ad739";
        let runtime_instance = match compose_dep_into_linker(
            &engine,
            &mut store,
            &mut linker,
            import_name,
            &bytes,
            None,
        ) {
            Ok(inst) => inst,
            Err(e) => {
                panic!("compose must match the bare export against the versioned import: {e:?}")
            }
        };
        let mut heap = match HeapHandle::bind(store, &runtime_instance) {
            Ok(h) => h,
            Err((e, _)) => panic!("bind HeapHandle to the composed runtime: {e:?}"),
        };
        assert_eq!(heap.box_int(1).expect("box-int"), 100);
    }

    // §19e HANDLE-LOWERED fold END-TO-END: a synthetic reducer that IMPORTS `cadenza:runtime/heap` and
    // EXPORTS the interface-nested `cadenza:agent-kernel/fold`.`apply(u32,u32,u32)->u32` — the exact shape
    // rcdzc emits for a real Cadenza reducer (v-hb-confirmed). Its `apply` ignores its 3 arg handles and
    // returns `arr-alloc(0)` (an empty-list handle) — the heap stub's `vec-len` is 0, so the host's
    // `read_effect_requests` decodes ZERO effects (B1-shaped: empty-effects fold). This drives the WHOLE
    // rebind: compose the runtime dep → capture its instance → instantiate the reducer against it → reach
    // the nested apply → marshal (ct,payload,resumes)→3 handles → call apply → unmarshal the returned
    // list handle. The runtime + reducer share ONE store/heap (the point of compose-returns-instance).
    fn handle_lowered_reducer_component() -> Vec<u8> {
        wat::parse_str(
            r#"(component
                 ;; IMPORT the runtime — declare only the ops apply uses (arr-alloc). The composed heap
                 ;; stub satisfies this under the same interface name.
                 (import "cadenza:runtime/heap" (instance $rt
                   (export "arr-alloc" (func (param "len" u32) (result u32)))))
                 (core module $m
                   (import "" "arr-alloc" (func $arr_alloc (param i32) (result i32)))
                   ;; apply(ct, payload, resumes) -> list-handle : ignore args, return arr-alloc(0) = empty list.
                   (func (export "apply") (param i32 i32 i32) (result i32)
                     (call $arr_alloc (i32.const 0))))
                 (core func $arr_alloc_core (canon lower (func $rt "arr-alloc")))
                 (core instance $shim (export "arr-alloc" (func $arr_alloc_core)))
                 (core instance $i (instantiate $m (with "" (instance $shim))))
                 (func $apply (param "ct" u32) (param "payload" u32) (param "resumes" u32) (result u32)
                   (canon lift (core func $i "apply")))
                 ;; EXPORT apply THROUGH the fold interface instance (interface-nested, not top-level).
                 (instance $fold (export "apply" (func $apply)))
                 (export "cadenza:agent-kernel/fold" (instance $fold)))"#,
        )
        .expect("assemble handle-lowered reducer component")
    }

    #[test]
    fn apply_handle_lowered_drives_a_reducer_through_the_marshalled_boundary() {
        let reducer_bytes = handle_lowered_reducer_component();
        let runtime_bytes = heap_stub_component();
        // Build the reducer + attach the runtime as its resolved `cadenza:runtime/heap` dep (the fixture
        // imports it under that bare name — no +hash — so declared_deps won't auto-detect it; wire it
        // explicitly, exactly as CAS resolution would for a real reducer).
        let reducer = match ComponentReducer::from_component_bytes(&reducer_bytes) {
            Ok(r) => r,
            Err(e) => panic!("build handle-lowered reducer: {e:?}"),
        };
        let runtime_dep = ComponentDep {
            import_name: "cadenza:runtime/heap".to_string(),
            hash: Hash::of(b"heap-stub-runtime"),
        };
        let reducer = reducer.with_resolved_deps(vec![(runtime_dep, runtime_bytes)]);
        // Drive the handle-lowered fold: marshal a content-type + None payload/resumes, call apply, unmarshal.
        let ct = ContentType {
            family: "message".to_string(),
            version: 1,
        };
        match reducer.apply_handle_lowered(Kv::new(), ct, None, None) {
            Ok((effects, _kv)) => assert!(
                effects.is_empty(),
                "the reducer returned arr-alloc(0) (empty list) → zero effects, got {}",
                effects.len()
            ),
            Err((e, _kv)) => panic!("apply_handle_lowered should drive the reducer, got {e:?}"),
        }
    }

    // The generic multi-export invoke fixture (operator invoke-ABI ruling seq 107/108): a component whose
    // `run: func(list<u8>) -> list<record{kind:string, name:string, bytes:list<u8>}>` export emits a SET
    // of two artifacts. Synthetic WAT (no wit-bindgen toolchain): the core `run` lays out the record
    // array in linear memory and returns the (ptr,len) the canon lift reads. Exercises the WHOLE invoke
    // decode: navigate the export → call → lift the artifact list → decode records/strings/byte-lists.
    // 🪤 An exported func referencing a named record type requires that TYPE to be EXPORTED first (aliased),
    // else `Component::new` rejects it "func not valid to be used as export".
    fn two_artifact_component() -> Vec<u8> {
        wat::parse_str(
            r#"(component
                 (core module $m
                   (memory (export "mem") 1)
                   (global $next (mut i32) (i32.const 4096))
                   (func (export "realloc") (param $old i32) (param $oldsz i32) (param $align i32) (param $newsz i32) (result i32)
                     (local $ret i32)
                     (global.set $next
                       (i32.and
                         (i32.add (global.get $next) (i32.sub (local.get $align) (i32.const 1)))
                         (i32.xor (i32.sub (local.get $align) (i32.const 1)) (i32.const -1))))
                     (local.set $ret (global.get $next))
                     (global.set $next (i32.add (global.get $next) (local.get $newsz)))
                     (local.get $ret))
                   ;; static string/byte data: "wasm"@0 "prog"@4 DE AD@8 ; "diag"@16 "log"@20 pad@23 2A@24
                   (data (i32.const 0) "wasmprog\de\ad")
                   (data (i32.const 16) "diaglog\00\2a")
                   ;; run(inptr,inlen) -> retptr : ignore input, return 2 records. record{string,string,
                   ;; list<u8>} = 6 i32s (24 B) as (ptr,len) pairs; 2 records = 48 B; then an 8-B (ptr,len)
                   ;; return area for the list itself.
                   (func (export "run") (param $inptr i32) (param $inlen i32) (result i32)
                     (local $recs i32)
                     (local $ret i32)
                     (global.set $next (i32.and (i32.add (global.get $next) (i32.const 3)) (i32.const -4)))
                     (local.set $recs (global.get $next))
                     (global.set $next (i32.add (global.get $next) (i32.const 48)))
                     ;; rec0: kind="wasm"(0,4) name="prog"(4,4) bytes=(8,2)
                     (i32.store (i32.add (local.get $recs) (i32.const 0)) (i32.const 0))
                     (i32.store (i32.add (local.get $recs) (i32.const 4)) (i32.const 4))
                     (i32.store (i32.add (local.get $recs) (i32.const 8)) (i32.const 4))
                     (i32.store (i32.add (local.get $recs) (i32.const 12)) (i32.const 4))
                     (i32.store (i32.add (local.get $recs) (i32.const 16)) (i32.const 8))
                     (i32.store (i32.add (local.get $recs) (i32.const 20)) (i32.const 2))
                     ;; rec1: kind="diag"(16,4) name="log"(20,3) bytes=(24,1)
                     (i32.store (i32.add (local.get $recs) (i32.const 24)) (i32.const 16))
                     (i32.store (i32.add (local.get $recs) (i32.const 28)) (i32.const 4))
                     (i32.store (i32.add (local.get $recs) (i32.const 32)) (i32.const 20))
                     (i32.store (i32.add (local.get $recs) (i32.const 36)) (i32.const 3))
                     (i32.store (i32.add (local.get $recs) (i32.const 40)) (i32.const 24))
                     (i32.store (i32.add (local.get $recs) (i32.const 44)) (i32.const 1))
                     (global.set $next (i32.and (i32.add (global.get $next) (i32.const 3)) (i32.const -4)))
                     (local.set $ret (global.get $next))
                     (global.set $next (i32.add (global.get $next) (i32.const 8)))
                     (i32.store (local.get $ret) (local.get $recs))
                     (i32.store (i32.add (local.get $ret) (i32.const 4)) (i32.const 2))
                     (local.get $ret)))
                 (core instance $i (instantiate $m))
                 (type $artifact (record (field "kind" string) (field "name" string) (field "bytes" (list u8))))
                 (export $artifact-x "artifact" (type $artifact))
                 (func $run (param "input" (list u8)) (result (list $artifact-x))
                   (canon lift (core func $i "run") (memory $i "mem") (realloc (func $i "realloc"))))
                 (export "run" (func $run)))"#,
        )
        .expect("assemble two-artifact component")
    }

    // The generic multi-export invoke (operator seq 107/108): invoke a component's named export over an
    // AST-encoded arg and decode a SET of artifacts. Empty interface = top-level `run` export. Proves
    // export navigation + call + full artifact-list decode (records, strings, byte lists), multi-artifact.
    #[test]
    fn invoke_component_decodes_the_emitted_artifact_set() {
        let bytes = two_artifact_component();
        let artifacts = invoke_component(&bytes, "", "run", b"ignored-ast-arg", DEFAULT_FOLD_FUEL)
            .expect("two-artifact component invokes");
        assert_eq!(
            artifacts,
            vec![
                Artifact {
                    kind: "wasm".into(),
                    name: "prog".into(),
                    bytes: vec![0xDE, 0xAD],
                },
                Artifact {
                    kind: "diag".into(),
                    name: "log".into(),
                    bytes: vec![0x2A],
                },
            ],
            "invoke returns the exact multi-artifact set the component emitted (seq-108)"
        );
    }

    // A valid component that lacks the named export is InvokeExport (actionable "not invokable at
    // interface#func"), naming the interface + func — NOT a trap or InvalidComponent.
    #[test]
    fn invoke_of_a_missing_export_is_invoke_export_naming_interface_and_func() {
        let empty = wat::parse_str("(component)").expect("empty component");
        match invoke_component(&empty, "", "run", b"", DEFAULT_FOLD_FUEL) {
            Err(ComponentError::InvokeExport {
                interface,
                func,
                reason,
            }) => {
                assert_eq!(interface, "");
                assert_eq!(func, "run");
                // Empty interface → the message says "top-level func", not `interface "" exports no func`
                // (github-liaison #2050 LOW clarity fix).
                assert!(
                    reason.contains("top-level func"),
                    "empty-interface error should say top-level func, got {reason:?}"
                );
            }
            other => panic!("expected InvokeExport for a missing export, got {other:?}"),
        }
        // A missing INTERFACE (non-empty) is likewise InvokeExport, naming it.
        let bytes = two_artifact_component();
        match invoke_component(&bytes, "test:nope/api", "run", b"", DEFAULT_FOLD_FUEL) {
            Err(ComponentError::InvokeExport { interface, .. }) => {
                assert_eq!(interface, "test:nope/api");
            }
            other => panic!("expected InvokeExport for a missing interface, got {other:?}"),
        }
    }

    // Bytes that aren't a component at all are InvalidComponent (never parsed) — DISTINCT from
    // "valid component, wrong/missing export" (InvokeExport).
    #[test]
    fn invoke_of_non_component_bytes_is_invalid_component() {
        match invoke_component(b"not a wasm component", "", "run", b"", DEFAULT_FOLD_FUEL) {
            Err(ComponentError::InvalidComponent(_)) => {}
            other => panic!("expected InvalidComponent for garbage bytes, got {other:?}"),
        }
    }

    // A component whose `run` returns the WRONG shape (a plain `list<u8>`, not the artifact-record list)
    // is InvokeExport, and — github-liaison #2050 MED/DoS — the error message is BOUNDED (reports the
    // Val shape/variant + length, e.g. "u8"/"list(len N)"), NEVER a full `{:?}` of the untrusted guest
    // value. Proves the val_shape path: no matter how large the guest's wrong-shape output, the error
    // string stays small.
    #[test]
    fn invoke_of_a_wrong_shape_result_is_bounded_invoke_export_not_full_debug() {
        // `run: func(list<u8>) -> list<u8>` that returns 4096 bytes — the identity-style fixture, but its
        // result is a byte list, so decode_artifact_list sees list elements that are u8, not records.
        let bytes = wat::parse_str(
            r#"(component
                 (core module $m
                   (memory (export "mem") 1)
                   (global $next (mut i32) (i32.const 8192))
                   (func (export "realloc") (param $old i32) (param $oldsz i32) (param $align i32) (param $newsz i32) (result i32)
                     (local $ret i32)
                     (global.set $next
                       (i32.and (i32.add (global.get $next) (i32.sub (local.get $align) (i32.const 1)))
                                (i32.xor (i32.sub (local.get $align) (i32.const 1)) (i32.const -1))))
                     (local.set $ret (global.get $next))
                     (global.set $next (i32.add (global.get $next) (local.get $newsz)))
                     (local.get $ret))
                   ;; run -> a (ptr,len) list of 4096 bytes starting at 0 (memory is zero-filled)
                   (func (export "run") (param $ptr i32) (param $len i32) (result i32)
                     (local $ret i32)
                     (global.set $next (i32.and (i32.add (global.get $next) (i32.const 3)) (i32.const -4)))
                     (local.set $ret (global.get $next))
                     (global.set $next (i32.add (global.get $next) (i32.const 8)))
                     (i32.store (local.get $ret) (i32.const 0))
                     (i32.store (i32.add (local.get $ret) (i32.const 4)) (i32.const 4096))
                     (local.get $ret)))
                 (core instance $i (instantiate $m))
                 (func $run (param "input" (list u8)) (result (list u8))
                   (canon lift (core func $i "run") (memory $i "mem") (realloc (func $i "realloc"))))
                 (export "run" (func $run)))"#,
        )
        .expect("assemble wrong-shape (list<u8>) component");
        match invoke_component(&bytes, "", "run", b"", DEFAULT_FOLD_FUEL) {
            Err(ComponentError::InvokeExport { reason, .. }) => {
                // The message names the u8 shape (bounded) and is SHORT — not a 4096-element Debug dump.
                assert!(
                    reason.contains("u8"),
                    "should report the u8 shape, got {reason:?}"
                );
                assert!(
                    reason.len() < 200,
                    "error message must be bounded regardless of guest output size, got len {}",
                    reason.len()
                );
            }
            other => {
                panic!("expected a bounded InvokeExport for a wrong-shape result, got {other:?}")
            }
        }
    }

    // §23: `with_resolved_deps` attaches resolved dep bytes (import_name + bytes) that `apply` composes
    // per-fold. Pins the resolve→attach handoff: the builder records exactly what `resolve_deps` yields.
    #[test]
    fn with_resolved_deps_records_the_import_name_and_bytes_apply_will_compose() {
        // A dependency-free reducer (empty component) — we only exercise the builder plumbing here; the
        // compose-through-apply path is proven by the linker-composition test above.
        let bytes = wat::parse_str("(component)").expect("empty component");
        let reducer = match ComponentReducer::from_component_bytes(&bytes) {
            Ok(r) => r,
            Err(e) => panic!("valid component: {e:?}"),
        };
        assert!(reducer.resolved_deps.is_empty(), "none attached yet");
        // The empty component exports no `fold` world, so it can't be pre-instantiated — falls back to
        // per-fold instantiate (which would surface the missing-fold error at apply time, unchanged).
        assert!(!reducer.uses_cached_instance_pre());
        let dep = ComponentDep {
            import_name: "cadenza:runtime/heap@0.0.0+abc".to_string(),
            hash: Hash::of(b"a dep"),
        };
        let reducer = reducer.with_resolved_deps(vec![(dep, b"dep-bytes".to_vec())]);
        assert_eq!(reducer.resolved_deps.len(), 1);
        assert_eq!(reducer.resolved_deps[0].0, "cadenza:runtime/heap@0.0.0+abc");
        assert_eq!(reducer.resolved_deps[0].1, b"dep-bytes".to_vec());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_dep_bytes_fetches_from_cas_or_splits_missing_vs_store_error() {
        use crate::blob::{BlobStore, MemBlobStore};
        let mut blobs = MemBlobStore::new();
        let dep_bytes = b"pretend a content-addressed dependency component";
        let hash = blobs.put(dep_bytes).await.unwrap();
        let dep = ComponentDep {
            import_name: format!("cadenza:runtime/heap@0.0.0+{}", hash.to_hex()),
            hash,
        };
        // Present in CAS → fetched (the kernel resolves it generically — it's just a dep by hash).
        assert_eq!(
            resolve_dep_bytes(&dep, &blobs).await.unwrap(),
            dep_bytes.to_vec()
        );
        // Absent → DepMissing (PR#1013 #3: distinct from a store error — "publish it" vs "store broke").
        let missing = ComponentDep {
            import_name: "some:iface/x@0.0.0+…".into(),
            hash: Hash::of(b"a dep never stored"),
        };
        match resolve_dep_bytes(&missing, &blobs).await {
            Err(ComponentError::DepMissing { hash }) => assert_eq!(hash, missing.hash),
            other => panic!("expected DepMissing, got {other:?}"),
        }
    }
}
