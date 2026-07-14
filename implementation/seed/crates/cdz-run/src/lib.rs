//! The generic wasm-component runner.
//!
//! One job: instantiate a finished component, compose the value-heap runtime when the component
//! imports it, invoke a chosen export with typed arguments, and render the result to canonical text.
//! Everything wasmtime lives here; callers hand in bytes and get back a [`Outcome`].
//!
//! The compiler is never in this crate's dependency graph — running a component needs no compiler —
//! so `cdz-run` stays a pure consumer of finished artifacts (component-abi.md).

use anyhow::{Result, anyhow};
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Linker, Type, Val};
use wasmtime::{Config, Engine, OptLevel, Store};

/// The wasmtime engine for a run. `cdz-run` is a ONE-SHOT tool: it JIT-compiles the component, invokes
/// an export ONCE, and exits — so Cranelift's optimizing backend (the `Engine::default()` `OptLevel::
/// Speed`) spends compile time that the single execution never repays. `OptLevel::None` skips the
/// optimization passes: the generated code is slower per-instruction, but the compile is much faster,
/// and for a tiny gate program (which runs once) the total `Component::new`→run time drops. This is the
/// dominant per-invocation cost across the gate's ~1000 spawns (cdz-run was the slowest pipeline stage).
fn engine() -> Engine {
    let mut cfg = Config::new();
    cfg.cranelift_opt_level(OptLevel::None);
    // A fresh `Config` can only fail to build an `Engine` on an unsupported target/feature combination,
    // which this host supports; fall back to the default engine if that ever changes rather than panic.
    Engine::new(&cfg).unwrap_or_default()
}

/// Load the value-heap runtime as a `Component`, reusing a CACHED compiled artifact when possible.
///
/// JIT-compiling the ~67KB runtime component is ~75ms, and it is BYTE-IDENTICAL for every heap program
/// (fixed by its content hash), yet `cdz-run` spawns fresh per program — so the gate recompiled the
/// SAME runtime hundreds of times. With `opts.runtime_cache_dir` set, the first run compiles + writes
/// `<dir>/<hash>-<wt>.cwasm` (wasmtime version fingerprint `<wt>` in the name), and every later run
/// `deserialize`s that (~0.25ms — a ~300× drop on runtime composition).
///
/// Safety: `Component::deserialize` is `unsafe` because arbitrary bytes could be malformed, but it
/// VALIDATES its own header (engine config + wasmtime version) and returns `Err` on any mismatch rather
/// than misbehaving — so a stale/incompatible `.cwasm` is REJECTED, not misread. We additionally key
/// the filename on the wasmtime version and only ever read a file THIS binary itself wrote, and any
/// `deserialize` error falls straight through to a fresh `Component::new`. So the cache can only make a
/// run faster, never change what it does.
fn load_runtime_component(
    engine: &Engine,
    runtime_bytes: &[u8],
    hash: &str,
    opts: &RunOpts,
) -> Result<Component> {
    let Some(dir) = opts.runtime_cache_dir.as_deref() else {
        // No cache configured — compile directly.
        return Component::new(engine, runtime_bytes)
            .map_err(|e| anyhow!("value-heap runtime component invalid: {e}"));
    };
    // `<hash>-<wasmtime-version>.cwasm`: the runtime's content address pins the SOURCE, the version pins
    // the COMPILER, so a cache file is only ever consulted for the exact runtime+wasmtime it was made
    // for. (`hash` is empty only for an unpinned import, which errors earlier; guard anyway.)
    let cache_path = (!hash.is_empty()).then(|| {
        dir.join(format!(
            "{hash}-wt{}.cwasm",
            env!("CARGO_PKG_VERSION_MAJOR") // cdz-run's own version — bumps if we change wasmtime deps
        ))
    });

    // Fast path: a cached artifact that deserializes cleanly.
    if let Some(path) = &cache_path
        && let Ok(bytes) = std::fs::read(path)
    {
        // SAFETY: bytes were produced by THIS binary's `Component::serialize` (below) for this exact
        // engine config + wasmtime version; `deserialize` re-checks that header and errs on mismatch,
        // so a corrupt/foreign file is rejected here rather than trusted.
        match unsafe { Component::deserialize(engine, &bytes) } {
            Ok(c) => return Ok(c),
            Err(_) => { /* stale/incompatible — fall through to recompile + rewrite */ }
        }
    }

    // Slow path: compile once, then persist the compiled artifact for next time (best-effort — a write
    // failure just means the next run recompiles, never an error).
    let component = Component::new(engine, runtime_bytes)
        .map_err(|e| anyhow!("value-heap runtime component invalid: {e}"))?;
    if let Some(path) = &cache_path
        && let Ok(serialized) = component.serialize()
    {
        // Write to a temp sibling then rename, so a concurrent reader never sees a half-written file
        // (the gate runs cdz-run in parallel). A collision on the temp name is harmless — last writer
        // wins and the content is identical.
        let tmp = path.with_extension(format!("cwasm.tmp.{}", std::process::id()));
        if std::fs::write(&tmp, &serialized).is_ok() {
            let _ = std::fs::rename(&tmp, path); // ignore: a lost race just recompiles next time
        }
    }
    Ok(component)
}

mod render;
pub use render::render_val;

/// The fixed identity of the value-heap runtime interface — the same for every program a generation
/// emits (component-abi.md §The Value-Heap Runtime Crosses By A Well-Known Import: the interface
/// identity is fixed at the declared-default location). A program imports it under this name plus a
/// content-address suffix (below), so this is the PREFIX the runtime import is recognized by.
const RUNTIME_IFACE: &str = "cadenza:runtime/heap";

/// The required runtime a component records: the exact import name it declares, and the content
/// address (hash) of the runtime that satisfies it. Per component-abi.md §The Emitted Component
/// Records Its Required Runtime, a program's runtime import name is `cadenza:runtime/heap@0.0.0+<hash>`
/// — the fixed interface plus the runtime's content address as semver build-metadata. The host reads
/// the hash back to resolve the exact runtime (§The Host Resolves The Runtime By Content Address).
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeReq {
    /// The verbatim import name the component declares — the linker MUST bind under exactly this.
    pub import_name: String,
    /// The content address (lowercase hex SHA-256) the component requires, extracted from the name.
    pub hash: String,
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The export returned; its result rendered to canonical text (`unit` for a no-result export).
    Value(String),
    /// The export trapped at run time (message).
    Trap(String),
}

/// Render a run error to a trap MESSAGE that surfaces the wasm trap REASON, not just the outer
/// "error while executing at wasm backtrace:" wrapper anyhow prints. wasmtime attaches a
/// [`wasmtime::Trap`] code to a trapping error's chain; its `Display` is the canonical reason
/// (`integer divide by zero`, `integer overflow`, `wasm 'unreachable' instruction executed`, `out of
/// bounds memory access`, …). Surface that reason FIRST so a reason-matching consumer (the behavior
/// gate) can recognize the trap, then the full error for a human. A non-trap error (no `Trap` in the
/// chain) renders as before.
fn trap_message(e: &anyhow::Error) -> String {
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(trap) => format!("{trap}: {e:?}"),
        None => format!("{e}"),
    }
}

/// How to run a component: which export, what arguments, and the value-heap runtime to compose.
#[derive(Debug, Default, Clone)]
pub struct RunOpts {
    /// The export to invoke. `None` selects the sole function export (by signature) — the common
    /// case for a scalar entry, whose ABI is `() -> scalar` and whose name the compiler emits verbatim.
    pub export: Option<String>,
    /// Raw, still-untyped argument strings from the CLI; coerced to the export's declared param types.
    pub args: Vec<String>,
    /// The value-heap runtime component bytes the caller resolved BY CONTENT ADDRESS. Required only
    /// when the component records a required runtime (see [`required_runtime`]); the caller is
    /// responsible for having fetched the runtime whose content address matches, and for binding it
    /// under the component's exact import name.
    pub runtime: Option<Vec<u8>>,
    /// Directory to cache the COMPILED runtime artifact in (normally the content-addressed store). JIT-
    /// compiling the 67KB runtime component is ~75ms and it is BYTE-IDENTICAL across every heap program
    /// — so, when set, `compose_runtime` writes `<dir>/<hash>.cwasm` on the first compile and
    /// `deserialize`s it (~0.25ms) on every later run. `None` disables the cache (always JIT). The
    /// cache is keyed by the runtime's content hash AND a wasmtime-version fingerprint in the filename,
    /// and a `deserialize` failure falls back to a fresh compile — so a version/config mismatch can
    /// never load an incompatible artifact.
    pub runtime_cache_dir: Option<std::path::PathBuf>,
    /// The HOST-CALL RESPONSES (E2h) — the values the host returns to a program's delegated host calls,
    /// in call order. A program that delegates an effect to the host (`(host (E…) …)`) imports each
    /// operation as a boundary func; when it performs one, the bound host func returns the next response
    /// here (`capabilities-and-effects.md` §A Run Is A Deterministic Function Of Its Input And
    /// Responses). Empty for a program that makes no host call. Coerced to each call's declared result
    /// type at binding. The corpus `(host-responses …)` fixture supplies these.
    pub host_responses: Vec<HostResponse>,
}

/// One recorded host-call RESPONSE — the operation it answers and the value the host returns. The
/// operation name (`E.op`, dotted) pairs a response with its call for the ordered-consume model; the
/// value is a raw text form (`(: 10 Int64)`) coerced to the op's boundary result type at binding.
#[derive(Debug, Clone)]
pub struct HostResponse {
    /// The dotted operation name the response answers (e.g. `ask.ask`) — for the ordered model + a
    /// mismatch diagnostic. This increment consumes responses purely in ORDER (the op name is recorded
    /// for the diagnostic, not yet matched).
    pub op: String,
    /// The response value in canonical text form (`(: 10 Int64)` or a bare `10`) — coerced to the op's
    /// declared boundary result type when the host func is bound.
    pub value: String,
}

/// Validate `component_bytes` as a well-formed component — the cheap structural check before a run.
pub fn validate(component_bytes: &[u8]) -> Result<()> {
    let engine = engine();
    Component::new(&engine, component_bytes)
        .map(|_| ())
        .map_err(|e| anyhow!("invalid component: {e}"))
}

/// Instantiate `component_bytes`, compose the value-heap runtime if imported, invoke the chosen
/// export with the (coerced) arguments, and return the rendered outcome. The OBSERVED host calls are
/// discarded; use [`run_capturing`] to also get the ordered list of host operations the run performed.
pub fn run(component_bytes: &[u8], opts: &RunOpts) -> Result<Outcome> {
    run_capturing(component_bytes, opts).map(|(o, _calls)| o)
}

/// [`run`], additionally returning the ordered list of HOST OPERATIONS the run performed (each a dotted
/// `E.op`, in call order) — so a caller (the corpus gate) can verify the observed host-call sequence
/// against a case's recorded `(host-calls …)`. Empty for a program that makes no host call.
pub fn run_capturing(component_bytes: &[u8], opts: &RunOpts) -> Result<(Outcome, Vec<String>)> {
    use std::sync::{Arc, Mutex};
    let engine = engine();
    let component =
        Component::new(&engine, component_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;

    let mut linker: Linker<()> = Linker::new(&engine);

    // If the component records a required runtime, satisfy that import by forwarding every function
    // the runtime's heap interface exports. The linker binds under the component's EXACT import name
    // (the hashed one), while the function set is DISCOVERED from the runtime component's own type —
    // never a hard-coded list — so it can never drift from the runtime the caller supplied.
    let mut store = Store::new(&engine, ());
    if let Some(req) = find_runtime_req(&engine, &component) {
        compose_runtime(&engine, &mut store, &mut linker, &req, opts)?;
    }

    // Bind every HOST import (a delegated effect's operations, E2h) so a program's host calls are
    // satisfied by the recorded responses, consumed in call order. Each performed call APPENDS its
    // dotted `E.op` to `observed`, so the caller can compare the observed sequence against the case's
    // recorded `(host-calls …)`. Inert for a program with no host import (the common case).
    let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    bind_host_imports(&engine, &component, &mut linker, opts, &observed, &[])?;

    let outcome = run_export(&engine, &component, &mut store, &linker, opts)?;
    let calls = observed.lock().expect("observed calls mutex").clone();
    Ok((outcome, calls))
}

/// One PEER component a consumer binds across the component boundary (X4,
/// `DESIGN-cross-component-interop-rcdzc.md`): its finished bytes and the INTERFACE it exports that the
/// consumer imports under the same name (`cadenza:<pkg>/<iface>`). The peer is a SEPARATELY-compiled
/// artifact — not merged into the consumer — so `run_with_peers` instantiates it and forwards its
/// exported interface funcs into the consumer's like-named import (component-abi.md §Cross-Component
/// Value Exchange; cross-component-interop.md).
#[derive(Debug, Clone)]
pub struct Peer {
    /// The peer component's bytes.
    pub bytes: Vec<u8>,
    /// The interface the peer EXPORTS and the consumer IMPORTS under this exact name.
    pub interface: String,
}

/// Run a CONSUMER component composed with a set of PEER components across the live component boundary.
/// All components share ONE `wasmtime` store (so a value one produces is meaningful to another — the
/// prerequisite for the shared-runtime handle transport X5 adds), and — when the consumer imports the
/// value-heap runtime — ONE runtime instance (component-abi.md §A Cross-Component Handle Is Meaningful
/// Only In The Shared Runtime Instance).
///
/// Each peer is instantiated first; the consumer's import of `peer.interface` is then bound by
/// forwarding every function the peer's exported interface offers (discovered off the peer instance's
/// type, never a hard-coded list — the same discipline `compose_runtime` uses for the runtime). When the
/// consumer OR any peer imports the value-heap runtime, ONE runtime instance is composed and bound into
/// EVERY component that imports it (X5), so a `value` handle one produces is meaningful to another (they
/// index the same heap — component-abi.md §A Cross-Component Handle Is Meaningful Only In The Shared
/// Runtime Instance). SCOPE: scalar peer ops today; a `value`-handle op rides this shared instance.
///
/// This is the host binding every composed component's value-heap runtime import to the ONE shared
/// instance: the consumer and each peer all pin the same runtime (same content hash → same import name),
/// so their handles index one heap and none is handed a handle into a heap it does not share. (The
/// `value`-handle crossing that USES this shared heap is X5b; X5a establishes the shared instance.)
//= spec/contracts/component-abi.md#a-cross-component-handle-is-meaningful-only-in-the-shared-runtime-instance
//# A host that composes Cadenza components which exchange values by handle MUST bind every such component's value-heap runtime import to the one shared runtime instance, so that the components' handles index one heap and a component cannot be handed a handle into a heap it does not share.
pub fn run_with_peers(consumer_bytes: &[u8], peers: &[Peer], opts: &RunOpts) -> Result<Outcome> {
    use std::sync::{Arc, Mutex};
    let engine = engine();
    let consumer = Component::new(&engine, consumer_bytes)
        .map_err(|e| anyhow!("invalid consumer component: {e}"))?;
    let mut store = Store::new(&engine, ());
    let mut linker: Linker<()> = Linker::new(&engine);

    // The runtime import each component may declare — the consumer and each peer. They all pin the SAME
    // runtime (same content hash → same import name), so ONE runtime instance serves them all. Instantiate
    // it once here (if anyone needs it), then bind it into every importing component's linker below.
    let peer_components: Vec<Component> = peers
        .iter()
        .map(|p| {
            Component::new(&engine, &p.bytes)
                .map_err(|e| anyhow!("invalid peer component `{}`: {e}", p.interface))
        })
        .collect::<Result<_>>()?;
    let consumer_req = find_runtime_req(&engine, &consumer);
    let any_req = consumer_req.clone().or_else(|| {
        peer_components
            .iter()
            .find_map(|c| find_runtime_req(&engine, c))
    });
    let shared_runtime = match &any_req {
        Some(req) => Some(instantiate_runtime(&engine, &mut store, req, opts)?),
        None => None,
    };

    // Bind the shared runtime into the CONSUMER's import (if it declares one).
    if let (Some(req), Some((rt_instance, names))) = (&consumer_req, &shared_runtime) {
        bind_runtime_into(
            &engine,
            &mut store,
            &mut linker,
            &req.import_name,
            rt_instance,
            names,
        )?;
    }

    // Instantiate each peer and forward its exported interface funcs into the consumer's like-named import.
    // A peer that imports the runtime gets the SAME shared instance bound into its linker (so its handles
    // index the one shared heap); its funcs live in the SHARED store. A peer may ALSO import ANOTHER peer's
    // interface (an A→B→C chain, where B binds A and publishes its own for C, U11): peers are given in
    // DEPENDENCY order, so each peer's linker is pre-bound with the interfaces of every EARLIER-instantiated
    // peer. The extracted interface funcs (`(iface, [(fname, Func)])`) are collected as we go, so a later
    // peer's linker and finally the consumer's linker bind against them.
    let mut peer_ifaces: Vec<(String, Vec<(String, wasmtime::component::Func)>)> = Vec::new();
    for (peer, peer_component) in peers.iter().zip(peer_components.iter()) {
        let mut peer_linker: Linker<()> = Linker::new(&engine);
        if let (Some(req), Some((rt_instance, names))) =
            (find_runtime_req(&engine, peer_component), &shared_runtime)
        {
            bind_runtime_into(
                &engine,
                &mut store,
                &mut peer_linker,
                &req.import_name,
                rt_instance,
                names,
            )?;
        }
        // Bind every EARLIER peer's interface into this peer's linker (dependency order): a peer that
        // imports `cadenza:pairs/api` sees it because the peer providing it was given first.
        bind_peer_ifaces_into(&mut peer_linker, &peer_ifaces)?;
        let peer_instance = peer_linker
            .instantiate(&mut store, peer_component)
            .map_err(|e| anyhow!("instantiate peer `{}`: {e}", peer.interface))?;
        let iface_idx = peer_instance
            .get_export_index(&mut store, None, &peer.interface)
            .ok_or_else(|| anyhow!("peer does not export the interface `{}`", peer.interface))?;
        // The interface's function names, read off the peer instance's exported interface type.
        let func_names: Vec<String> = peer_component
            .component_type()
            .exports(&engine)
            .find(|(n, _)| *n == peer.interface)
            .and_then(|(_, item)| match item {
                ComponentItem::ComponentInstance(inst) => Some(
                    inst.exports(&engine)
                        .filter_map(|(fname, i)| {
                            matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string())
                        })
                        .collect(),
                ),
                _ => None,
            })
            .ok_or_else(|| {
                anyhow!(
                    "peer export `{}` is not an interface instance",
                    peer.interface
                )
            })?;
        let mut funcs = Vec::new();
        for fname in &func_names {
            let fidx = peer_instance
                .get_export_index(&mut store, Some(&iface_idx), fname)
                .ok_or_else(|| anyhow!("peer `{}` missing `{fname}`", peer.interface))?;
            let f = peer_instance
                .get_func(&mut store, fidx)
                .ok_or_else(|| anyhow!("peer export `{fname}` is not a func"))?;
            funcs.push((fname.clone(), f));
        }
        peer_ifaces.push((peer.interface.clone(), funcs));
    }

    // Bind every peer's exported interface into the CONSUMER's linker (the top of the chain imports them).
    bind_peer_ifaces_into(&mut linker, &peer_ifaces)?;

    // Bind the consumer's HOST-effect imports (if any), skipping the peer interfaces already bound above
    // so a peer interface is never double-bound as a host effect.
    let peer_names: Vec<String> = peers.iter().map(|p| p.interface.clone()).collect();
    let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    bind_host_imports(
        &engine,
        &consumer,
        &mut linker,
        opts,
        &observed,
        &peer_names,
    )?;

    run_export(&engine, &consumer, &mut store, &linker, opts)
}

/// Instantiate the linked component and invoke its chosen export (or the resource-escape path), returning
/// the rendered outcome. Split out of [`run_capturing`] so the host-call observation wraps it.
fn run_export(
    engine: &Engine,
    component: &Component,
    store: &mut Store<()>,
    linker: &Linker<()>,
    opts: &RunOpts,
) -> Result<Outcome> {
    let instance = linker
        .instantiate(&mut *store, component)
        .map_err(|e| anyhow!("instantiate: {e}"))?;

    // Whether `name` (or its kebab-normalized form) resolves to a TOP-LEVEL bare component func. Shared by
    // the escape/closure dispatch below: a `--call <name>` that names a real bare func takes the plain path;
    // a name that does NOT (a compound/closure result carries no bare func under that name) routes to the
    // resource/closure escape instead.
    let names_a_top_level_func = |store: &mut Store<()>, name: &str| -> bool {
        instance.get_func(&mut *store, name).is_some() || {
            let kebab = cadenza_syntax::extern_name::kebab_extern_name(name);
            kebab != name && instance.get_func(&mut *store, &kebab).is_some()
        }
    };

    // The RESOURCE ESCAPE (`DESIGN-value-heap-rcdzc.md` §3a): a program whose result is a COMPOUND
    // exports no bare function — it publishes a `cadenza:run/run` instance carrying `make : () -> own<t>`
    // + `encode : (own<t>) -> list<u8>`. Call `make` then `encode`, DECODE the canonical binary value
    // form with the shared codec, and pretty-print `(: value type)` — the value crossing the boundary as
    // a strongly-typed resource, rendered by the host (not spelled out in wasm). Taken when the run instance
    // is present AND the named export (if any) is NOT a top-level bare func — a compound export carries no
    // bare func under its name, so `(call greet)` on a `String`-returning `greet` routes here (the escape
    // has ONE compound result; its make/encode take no export name). No `--call` (the corpus's nullary
    // `main`) also routes here.
    if has_run_instance(engine, component)
        && sole_func_export(engine, component).is_none()
        && opts
            .export
            .as_deref()
            .map(|name| !names_a_top_level_func(&mut *store, name))
            .unwrap_or(true)
    {
        return run_resource_escape(&mut *store, &instance, &opts.args);
    }

    // The CLOSURE ESCAPE (`DESIGN-closure-host-resource-rcdzc.md`, C-HOST-1): a program whose result is a
    // closure exports the `cadenza:closure/exports` instance (`make`/`call`), not a bare function. Call
    // `make()` → the closure handle, then `call(handle, args…)` with the caller's arguments, rendering the
    // result. Taken when the closure interface is present AND the named export is NOT a TOP-LEVEL bare func
    // (so the args are the closure's arguments). A MIXED program (a closure export ALONGSIDE a plain export)
    // has BOTH the closure interface and top-level funcs — `--call <plain>` resolves as a bare func and
    // falls through to the plain path below; `--call <closure>` (or no `--call`, the corpus's `main`) has no
    // top-level func and routes here.
    if has_closure_instance(engine, component)
        && opts
            .export
            .as_deref()
            .map(|name| !names_a_top_level_func(&mut *store, name))
            .unwrap_or(true)
    {
        return run_closure_resource(
            engine,
            component,
            &mut *store,
            &instance,
            opts.export.as_deref(),
            &opts.args,
        );
    }

    // Resolve the export to call: the named one, or the sole function export found by signature.
    let export_name = match &opts.export {
        Some(name) => name.clone(),
        None => sole_func_export(engine, component).ok_or_else(|| {
            anyhow!("no --call given and the component has no single function export to default to")
        })?,
    };
    // The component's extern name is KEBAB-CASE, but a caller names the export by its SOURCE identifier
    // (`--call fA`), which may not be kebab (`fA`, `my_func`). The compiler normalized the extern name at
    // emit (`kebab_extern_name`); resolve the SOURCE name through the SAME deterministic rule so a caller
    // still uses the source name. Try the verbatim name first (already-kebab / core-level exports match
    // it unchanged), then the normalized form.
    let func = instance
        .get_func(&mut *store, &export_name)
        .or_else(|| {
            let kebab = cadenza_syntax::extern_name::kebab_extern_name(&export_name);
            (kebab != export_name)
                .then(|| instance.get_func(&mut *store, &kebab))
                .flatten()
        })
        .ok_or_else(|| anyhow!("component exports no function `{export_name}`"))?;

    // Coerce the raw argument strings to the export's declared parameter types.
    let param_types: Vec<Type> = func
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let args = coerce_args(&opts.args, &param_types)?;

    let result_count = func.results(&*store).len();
    let mut results = vec![Val::Bool(false); result_count];
    match func.call(&mut *store, &args, &mut results) {
        Ok(()) => {
            let rendered = match results.first() {
                None => "unit".to_string(),
                // A compound program's entry returns its result ALREADY rendered to canonical text
                // (the program walked its value through the runtime and assembled the string); take a
                // returned string verbatim rather than re-quoting it. A scalar result renders directly.
                Some(Val::String(s)) => s.clone(),
                Some(other) => render_val(other),
            };
            let _ = func.post_return(&mut *store);
            Ok(Outcome::Value(rendered))
        }
        Err(e) => Ok(Outcome::Trap(trap_message(&e))),
    }
}

/// The required runtime a `component` records, if any: its runtime import (recognized by the fixed
/// `cadenza:runtime/heap` interface prefix) and the content address carried in that import name.
/// A component with no such import produces `None` (a scalar/const program needs no runtime).
pub fn required_runtime(component_bytes: &[u8]) -> Result<Option<RuntimeReq>> {
    let engine = engine();
    let component =
        Component::new(&engine, component_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;
    Ok(find_runtime_req(&engine, &component))
}

/// Find the runtime import on `component` and parse its content-address suffix into a [`RuntimeReq`].
fn find_runtime_req(engine: &Engine, component: &Component) -> Option<RuntimeReq> {
    component
        .component_type()
        .imports(engine)
        .map(|(name, _)| name.to_string())
        .find(|name| import_is_runtime(name))
        .map(|import_name| {
            let hash = hash_from_import(&import_name);
            RuntimeReq { import_name, hash }
        })
}

/// Is `name` the value-heap runtime import? It is `cadenza:runtime/heap` optionally followed by a
/// version/build-metadata suffix (`@…`) — so match the interface up to the version boundary.
fn import_is_runtime(name: &str) -> bool {
    name == RUNTIME_IFACE || name.starts_with(&format!("{RUNTIME_IFACE}@"))
}

/// The content address recorded in a runtime import name. The name is
/// `cadenza:runtime/heap@<semver>+<hash>`; the hash is the semver build-metadata (after `+`). An
/// import with no `+<hash>` (an unpinned interface) yields an empty string — no content address recorded.
fn hash_from_import(name: &str) -> String {
    name.rsplit_once('+')
        .map(|(_, h)| h.to_string())
        .unwrap_or_default()
}

/// Compose the value-heap runtime: instantiate the runtime component, then forward each function its
/// heap interface exports into the program's import — bound under the program's EXACT import name
/// (`req.import_name`, which carries the content-address suffix). The function names are read off the
/// runtime's own instance type, so the composition always matches the supplied runtime.
fn compose_runtime(
    engine: &Engine,
    store: &mut Store<()>,
    linker: &mut Linker<()>,
    req: &RuntimeReq,
    opts: &RunOpts,
) -> Result<()> {
    let (rt_instance, heap_func_names) = instantiate_runtime(engine, store, req, opts)?;
    bind_runtime_into(
        engine,
        store,
        linker,
        &req.import_name,
        &rt_instance,
        &heap_func_names,
    )
}

/// Instantiate the value-heap runtime component ONCE in `store`, returning its instance + the heap-op
/// function names (read off its own type). Split out of [`compose_runtime`] so a SHARED runtime instance
/// can be bound into SEVERAL components' imports (X5: consumer + peers share one heap so a `value` handle
/// one produces is meaningful to another — component-abi.md §A Cross-Component Handle Is Meaningful Only
/// In The Shared Runtime Instance).
fn instantiate_runtime(
    engine: &Engine,
    store: &mut Store<()>,
    req: &RuntimeReq,
    opts: &RunOpts,
) -> Result<(wasmtime::component::Instance, Vec<String>)> {
    let runtime_bytes = opts.runtime.as_deref().ok_or_else(|| {
        anyhow!(
            "component requires the value-heap runtime {} but none was provided (the host resolves \
             it by content address from the store; build it with `cargo xtask build`)",
            req.hash
        )
    })?;
    let runtime = load_runtime_component(engine, runtime_bytes, &req.hash, opts)?;
    let heap_func_names = heap_interface_funcs(engine, &runtime)?;
    let rt_linker: Linker<()> = Linker::new(engine);
    let rt_instance = rt_linker
        .instantiate(&mut *store, &runtime)
        .map_err(|e| anyhow!("instantiate runtime: {e}"))?;
    Ok((rt_instance, heap_func_names))
}

/// Forward each already-extracted PEER interface into `linker`, so a component importing `cadenza:pkg/iface`
/// resolves it to the like-named peer's exported funcs (U11 chain support). Each entry is
/// `(interface, [(fname, Func)])` — the funcs pulled off an earlier-instantiated peer instance. Shared by
/// each peer's linker (dependency order) and the consumer's linker. A `func_new` closure calls the peer
/// func then its `post_return`, exactly as the inline single-pass binding did.
fn bind_peer_ifaces_into(
    linker: &mut Linker<()>,
    peer_ifaces: &[(String, Vec<(String, wasmtime::component::Func)>)],
) -> Result<()> {
    for (interface, funcs) in peer_ifaces {
        let mut iface = linker
            .instance(interface)
            .map_err(|e| anyhow!("linker instance {interface}: {e}"))?;
        for (fname, f) in funcs {
            let f = *f;
            iface.func_new(fname, move |mut ctx, params, results| {
                f.call(&mut ctx, params, results)?;
                f.post_return(&mut ctx)?;
                Ok(())
            })?;
        }
    }
    Ok(())
}

/// Forward each heap-op function of an already-instantiated runtime instance into `linker` under
/// `import_name` (the exact hashed name the importing component declared). Reused to bind ONE runtime
/// instance into multiple components' imports (X5).
fn bind_runtime_into(
    engine: &Engine,
    store: &mut Store<()>,
    linker: &mut Linker<()>,
    import_name: &str,
    rt_instance: &wasmtime::component::Instance,
    heap_func_names: &[String],
) -> Result<()> {
    let _ = engine;
    let heap_idx = rt_instance
        .get_export_index(&mut *store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;
    // Bind under the program's exact (hashed) import name, not the bare interface — that is the name
    // the program declared, and the linker matches names verbatim.
    let mut iface = linker
        .instance(import_name)
        .map_err(|e| anyhow!("linker instance {import_name}: {e}"))?;
    for fname in heap_func_names {
        let fidx = rt_instance
            .get_export_index(&mut *store, Some(&heap_idx), fname)
            .ok_or_else(|| anyhow!("runtime missing `{fname}`"))?;
        let f = rt_instance
            .get_func(&mut *store, fidx)
            .ok_or_else(|| anyhow!("runtime export `{fname}` is not a func"))?;
        iface.func_new(fname, move |mut ctx, params, results| {
            f.call(&mut ctx, params, results)?;
            f.post_return(&mut ctx)?;
            Ok(())
        })?;
    }
    Ok(())
}

/// Bind every HOST-effect import the component declares (E2h) so its delegated operations resolve to the
/// recorded responses, consumed in call order. A host effect is imported as an INSTANCE (the interface);
/// each function in it is a delegated operation. We enumerate the imported instances OFF THE COMPONENT
/// TYPE (never a hard-coded list — `host-interface-binding.md` §Which Host Functions Exist Is The
/// Target's Concern), skipping the value-heap runtime instance (bound by `compose_runtime`), and bind
/// each func via a dynamic closure that pops the next response and coerces it to the func's declared
/// result type. Responses are shared through an `Rc<RefCell<_>>` cursor (a one-shot single-threaded run).
fn bind_host_imports(
    engine: &Engine,
    component: &Component,
    linker: &mut Linker<()>,
    opts: &RunOpts,
    observed: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    // Interface names ALREADY bound (as cross-component PEERS, X4) — skip them here so a peer interface
    // is not also bound as a host effect (a double-bind is a linker error). Empty for a plain run.
    skip: &[String],
) -> Result<()> {
    use std::sync::{Arc, Mutex};
    // The shared response cursor — every bound host func pops the next response in order. `Arc<Mutex>`
    // (not `Rc`) because wasmtime requires the host closure be `Send + Sync`; a run is single-threaded,
    // so the mutex is uncontended.
    let cursor = Arc::new(Mutex::new(0usize));
    let responses = Arc::new(opts.host_responses.clone());

    // Enumerate the imported instances (host effect interfaces) off the component type. The runtime
    // interface (if imported) is bound elsewhere — skip it here. EVERY func is bound (including a
    // unit-result op like `log.emit`, which returns nothing) so a delegated call is always satisfied.
    // One entry per imported instance: its interface name + its ops (each `(op-name, result-type?)`).
    type HostIface = (String, Vec<(String, Option<Type>)>);
    let imports: Vec<HostIface> = component
        .component_type()
        .imports(engine)
        .filter_map(|(name, item)| {
            if is_runtime_import_name(name) || skip.iter().any(|s| s == name) {
                return None;
            }
            if let ComponentItem::ComponentInstance(inst) = item {
                let funcs: Vec<(String, Option<Type>)> = inst
                    .exports(engine)
                    .filter_map(|(fname, i)| match i {
                        // The op's declared result type, if any — a unit-result op (`func()`) has none,
                        // and consumes NO response (it is still bound + observed).
                        ComponentItem::ComponentFunc(f) => {
                            Some((fname.to_string(), f.results().next()))
                        }
                        _ => None,
                    })
                    .collect();
                Some((name.to_string(), funcs))
            } else {
                None
            }
        })
        .collect();

    for (iface_name, funcs) in imports {
        let mut iface = linker
            .instance(&iface_name)
            .map_err(|e| anyhow!("linker instance {iface_name}: {e}"))?;
        for (fname, ret_ty) in funcs {
            let cursor = Arc::clone(&cursor);
            let responses = Arc::clone(&responses);
            let observed = Arc::clone(observed);
            let op_label = format!("{iface_name}.{fname}");
            iface.func_new(&fname, move |_ctx, params, results| {
                // OBSERVE the call — append its dotted `E.op` in call order (so the gate can verify the
                // sequence against `(host-calls …)`). When the call carries STRING arguments (a
                // `report.fail("msg")` / `log.emit("…")`), append them after a TAB so a consumer that
                // wants the message (`cdz test`, whose failure path emits the assertion text) can read it —
                // WITHOUT polluting the op field: `main.rs` splits the entry on the first tab, so the
                // `host-call\t<op>` line the gate parses keeps a clean `<op>`, and the message rides a
                // separate `host-arg` line. A non-string arg (a scalar) is not captured (nothing reads it).
                let str_args: Vec<String> = params
                    .iter()
                    .filter_map(|v| match v {
                        Val::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                let entry = if str_args.is_empty() {
                    op_label.clone()
                } else {
                    format!("{op_label}\t{}", str_args.join(" "))
                };
                observed.lock().expect("observed calls mutex").push(entry);
                // A unit-result op returns nothing and consumes no response. A scalar-result op pops the
                // next recorded response, coerces it to the result type, and returns it.
                if let Some(slot) = results.get_mut(0) {
                    let ret_ty = ret_ty
                        .clone()
                        .expect("a result slot implies a declared result type");
                    let mut idx = cursor.lock().expect("host response cursor mutex");
                    let resp = responses.get(*idx).ok_or_else(|| {
                        anyhow!(
                            "host call `{op_label}` has no recorded response (only {} supplied)",
                            responses.len()
                        )
                    })?;
                    *idx += 1;
                    *slot = coerce_one(&scalar_of_value_form(&resp.value), &ret_ty)?;
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

/// Extract the bare scalar text from a response value form: `(: 10 Int64)` → `10`, or a bare `10` → `10`.
/// The corpus records a response as a typed `(: value Type)` form; the runner coerces the value to the
/// op's declared boundary result type, so only the value text is needed here.
fn scalar_of_value_form(form: &str) -> String {
    let t = form.trim();
    if let Some(inner) = t.strip_prefix("(:").and_then(|s| s.strip_suffix(')')) {
        // `(: <value> <Type>)` — the value is the first whitespace-delimited token after `:`.
        let mut it = inner.split_whitespace();
        if let Some(v) = it.next() {
            return v.to_string();
        }
    }
    t.to_string()
}

/// Whether `name` is the value-heap runtime import (recognized by the fixed interface prefix) — bound by
/// `compose_runtime`, so `bind_host_imports` skips it.
fn is_runtime_import_name(name: &str) -> bool {
    name.starts_with(RUNTIME_IFACE)
}

/// The names of the functions the runtime's `cadenza:runtime/heap` interface exports, read off the
/// component type — the source of truth for what to forward, so nothing is hard-coded.
fn heap_interface_funcs(engine: &Engine, runtime: &Component) -> Result<Vec<String>> {
    for (name, item) in runtime.component_type().exports(engine) {
        if name != RUNTIME_IFACE {
            continue;
        }
        if let ComponentItem::ComponentInstance(inst) = item {
            return Ok(inst
                .exports(engine)
                .filter_map(|(fname, i)| {
                    matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string())
                })
                .collect());
        }
    }
    Err(anyhow!(
        "runtime component does not export the {RUNTIME_IFACE} interface"
    ))
}

/// The name of the component's sole top-level FUNCTION export, if there is exactly one — the default
/// entry when `--call` is omitted. Interface/instance exports are ignored; only bare functions count.
fn sole_func_export(engine: &Engine, component: &Component) -> Option<String> {
    let mut only = None;
    for (name, item) in component.component_type().exports(engine) {
        if let ComponentItem::ComponentFunc(_) = item {
            if only.is_some() {
                return None; // more than one — ambiguous, require --call
            }
            only = Some(name.to_string());
        }
    }
    only
}

/// The well-known instance a resource-escape program exports its result through (`make`/`encode` live
/// inside it). `cdz-run` recognizes this instance to take the resource-decode path.
const RUN_INTERFACE: &str = "cadenza:run/run";

/// Whether `component` exports a `cadenza:run/run` INSTANCE — the marker of a resource-escape program
/// (its compound result crosses as a resource with a `make`/`encode` pair, not a bare function).
fn has_run_instance(engine: &Engine, component: &Component) -> bool {
    component
        .component_type()
        .exports(engine)
        .any(|(name, item)| {
            name == RUN_INTERFACE && matches!(item, ComponentItem::ComponentInstance(_))
        })
}

/// The interface a CLOSURE-resource export publishes under (`make`/`call` live inside it) —
/// `DESIGN-closure-host-resource-rcdzc.md`, C-HOST-1. A closure crossing the boundary becomes a resource
/// the host holds + invokes; `cdz-run` recognizes this instance to take the closure-call path.
const CLOSURE_INTERFACE: &str = "cadenza:closure/exports";

/// Whether `component` exports a `cadenza:closure/exports` INSTANCE — the marker of a closure-resource
/// program (its result is a closure crossing as a resource with a `make`/`call` pair).
fn has_closure_instance(engine: &Engine, component: &Component) -> bool {
    component
        .component_type()
        .exports(engine)
        .any(|(name, item)| {
            name == CLOSURE_INTERFACE && matches!(item, ComponentItem::ComponentInstance(_))
        })
}

/// The FUNCTION names the `cadenza:closure/exports` instance exports — used to distinguish a round-trip
/// component (named producer + consumer funcs, NO `call` method) from the single/multi-export shape
/// (which has a `call`). Read off the component type, so nothing is hard-coded.
fn closure_interface_funcs(engine: &Engine, component: &Component) -> Vec<String> {
    for (name, item) in component.component_type().exports(engine) {
        if name != CLOSURE_INTERFACE {
            continue;
        }
        if let ComponentItem::ComponentInstance(inst) = item {
            return inst
                .exports(engine)
                .filter_map(|(fname, i)| {
                    matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string())
                })
                .collect();
        }
    }
    Vec::new()
}

/// Run a ROUND-TRIP closure program (C-HOST-4): the host produces a closure handle from a PRODUCER export,
/// then threads it BACK into a CONSUMER export that applies it. Recognized when the closure interface has
/// NO `call` method (the single/multi-export shape) but a named CONSUMER (a func whose FIRST param is the
/// resource handle). The corpus names the CONSUMER in `(call <consumer> args…)`; the driver finds the sole
/// PRODUCER (the other func, whose result is the resource — every non-consumer func), calls it with the
/// LEADING args (its own params), then the consumer with the produced handle + the REMAINING args. So
/// `(call apply-it 10 5)` → `make-adder(10)` → handle → `apply-it(handle, 5)`.
fn run_roundtrip_closure(
    store: &mut Store<()>,
    instance: &wasmtime::component::Instance,
    iface: &wasmtime::component::ComponentExportIndex,
    consumer_name: &str,
    iface_funcs: &[String],
    arg_strs: &[String],
) -> Result<Outcome> {
    let get = |store: &mut Store<()>, name: &str| -> Result<wasmtime::component::Func> {
        let idx = instance
            .get_export_index(&mut *store, Some(iface), name)
            .ok_or_else(|| {
                anyhow!("round-trip closure: `{CLOSURE_INTERFACE}` exports no `{name}`")
            })?;
        instance
            .get_func(&mut *store, idx)
            .ok_or_else(|| anyhow!("round-trip closure: `{name}` is not a function"))
    };
    let consumer = get(&mut *store, consumer_name)?;
    // The consumer's params, in SOURCE ORDER — each is either a CLOSURE the host threads a produced handle
    // into (`Type::Own`/`Type::Borrow` — a resource) or a SCALAR taken from the arg strings. A closure param
    // may sit anywhere and there may be several; each gets its OWN fresh handle from the PRODUCER whose
    // RESULT resource type MATCHES that param (a distinct-sig round trip has several producers, one per
    // resource type — the first non-consumer func with a matching own<t> result).
    let cons_params: Vec<Type> = consumer
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    // The producer func matching a given resource type — a func (≠ the consumer) whose sole result is
    // `own<rt>`/`borrow<rt>`. Returns (func, its param types).
    let find_producer = |store: &mut Store<()>,
                         want: &wasmtime::component::ResourceType|
     -> Result<(wasmtime::component::Func, Vec<Type>)> {
        for name in iface_funcs {
            if name == consumer_name {
                continue;
            }
            let f = get(&mut *store, name)?;
            let matches_res = matches!(
                f.results(&*store).first(),
                Some(Type::Own(rt)) | Some(Type::Borrow(rt)) if rt == want
            );
            if matches_res {
                let params = f.params(&*store).iter().map(|(_, t)| t.clone()).collect();
                return Ok((f, params));
            }
        }
        Err(anyhow!(
            "round-trip closure: no producer mints the resource `{consumer_name}` expects"
        ))
    };
    // The corpus supplies the producer args for EACH closure param (in param order), then the consumer's
    // scalar args. Walk the consumer params: a closure param consumes its producer's arity from the front;
    // scalars come after all producer args.
    let n_closure_params = cons_params
        .iter()
        .filter(|t| matches!(t, Type::Own(_) | Type::Borrow(_)))
        .count();
    // Compute total producer-arg count (sum over each closure param's matching producer arity).
    let mut prod_specs: Vec<(wasmtime::component::Func, Vec<Type>)> = Vec::new();
    for t in &cons_params {
        if let Type::Own(rt) | Type::Borrow(rt) = t {
            prod_specs.push(find_producer(&mut *store, rt)?);
        }
    }
    let n_prod_args_total: usize = prod_specs.iter().map(|(_, p)| p.len()).sum();
    if arg_strs.len() < n_prod_args_total {
        return Err(anyhow!(
            "round-trip closure: producing {n_closure_params} closure(s) needs {n_prod_args_total} \
             producer argument(s) but only {} supplied",
            arg_strs.len()
        ));
    }
    // Produce one handle per closure param, each from the next slice of producer args.
    let mut handles: Vec<Val> = Vec::new();
    let mut arg_off = 0usize;
    for (producer, prod_params) in &prod_specs {
        let prod_args = coerce_args(&arg_strs[arg_off..arg_off + prod_params.len()], prod_params)?;
        arg_off += prod_params.len();
        let mut handle = [Val::Bool(false)];
        if let Err(e) = producer.call(&mut *store, &prod_args, &mut handle) {
            return Ok(Outcome::Trap(trap_message(&e)));
        }
        let _ = producer.post_return(&mut *store);
        handles.push(handle[0].clone());
    }
    // Build the consumer's args IN ORDER: a closure param → the next produced handle; a scalar → the next
    // scalar arg string.
    let scalar_strs = &arg_strs[n_prod_args_total..];
    let mut cons_args: Vec<Val> = Vec::new();
    let mut next_handle = 0usize;
    let mut next_scalar = 0usize;
    for t in &cons_params {
        if matches!(t, Type::Own(_) | Type::Borrow(_)) {
            cons_args.push(handles[next_handle].clone());
            next_handle += 1;
        } else {
            let s = scalar_strs.get(next_scalar).ok_or_else(|| {
                anyhow!(
                    "round-trip closure: consumer `{consumer_name}` needs more scalar arguments"
                )
            })?;
            cons_args.push(coerce_one(s, t)?);
            next_scalar += 1;
        }
    }
    let mut out = [Val::Bool(false)];
    match consumer.call(&mut *store, &cons_args, &mut out) {
        Ok(()) => {
            let _ = consumer.post_return(&mut *store);
            Ok(Outcome::Value(render_closure_call_result(out.first())))
        }
        Err(e) => Ok(Outcome::Trap(trap_message(&e))),
    }
}

/// Run a CLOSURE-resource program: reach `make`/`call` inside the `cadenza:closure/exports` instance,
/// call `make(make-args…)` → the closure resource handle, then `call(handle, call-args…)` → the closure's
/// result, rendered. The host acts as the closure's custodian: it holds the opaque handle and invokes the
/// guest's `call` method (which dispatches the closure via the guest's own `call_indirect`). `own<t>`
/// consumes the handle, so this is one `make`+`call` per case.
///
/// The caller supplies ONE flat arg list (`(call name a b c …)`); it is SPLIT by `make`'s declared arity —
/// the first N go to `make` (the EXPORT's parameters, e.g. `adder`'s `k`), the rest to `call` (the
/// CLOSURE's own arguments, e.g. `x`). A nullary export (N=0) sends all args to `call`. So
/// `(call adder (: 10 Int64) (: 5 Int64))` → `make(10)` then `call(5)` = 15.
fn run_closure_resource(
    engine: &Engine,
    component: &Component,
    store: &mut Store<()>,
    instance: &wasmtime::component::Instance,
    export: Option<&str>,
    arg_strs: &[String],
) -> Result<Outcome> {
    let iface = instance
        .get_export_index(&mut *store, None, CLOSURE_INTERFACE)
        .ok_or_else(|| anyhow!("closure escape: no `{CLOSURE_INTERFACE}` instance export"))?;
    let iface_funcs = closure_interface_funcs(engine, component);
    // ROUND-TRIP (C-HOST-4): producer + consumer exports, NO `call` method AND NO per-signature `call-g<n>`
    // (a distinct-sig program also lacks a bare `call` but has `call-g0` — handled below). The corpus
    // `(call <consumer> args…)` names the consumer; the sole PRODUCER (the other func) mints the closure.
    let has_call_g = iface_funcs.iter().any(|f| f == "call-g0");
    if !iface_funcs.iter().any(|f| f == "call") && !has_call_g {
        let consumer = export.ok_or_else(|| {
            anyhow!("round-trip closure: no --call given (name the CONSUMER export)")
        })?;
        // The public export name is KEBAB (the compiler normalized it at emit); a caller names the
        // consumer by its SOURCE identifier (`appA`, `my_func`). Resolve through the SAME rule so both
        // sides agree — `iface_funcs` are the actual (kebab) export names, so the comparison inside
        // `run_roundtrip_closure` (a func ≠ the consumer is a producer) must see the kebab consumer name.
        let consumer = cadenza_syntax::extern_name::kebab_extern_name(consumer);
        return run_roundtrip_closure(
            &mut *store,
            instance,
            &iface,
            &consumer,
            &iface_funcs,
            arg_strs,
        );
    }
    // DISTINCT-SIGNATURE multi-export: no bare `call`, but per-signature `call-g<n>` functions (each bound
    // to its own resource type). The corpus `(call <name> …)` names a closure export → `make-<name>`; the
    // matching call is the `call-g<n>` whose `self` param resource type equals `make-<name>`'s RESULT
    // resource type.
    if has_call_g {
        let name = export.ok_or_else(|| {
            anyhow!("distinct-sig closure: no --call given (name a closure export)")
        })?;
        // Public export names are KEBAB; a caller names the closure by its source identifier. Normalize
        // `make-<src>` the same way emit did so the lookup matches (`make-mkA` → `make-mk-a`).
        let make_name = cadenza_syntax::extern_name::kebab_extern_name(&format!("make-{name}"));
        let make_idx = instance
            .get_export_index(&mut *store, Some(&iface), &make_name)
            .ok_or_else(|| anyhow!("distinct-sig closure: no `{make_name}`"))?;
        let make = instance
            .get_func(&mut *store, make_idx)
            .ok_or_else(|| anyhow!("distinct-sig closure: `{make_name}` is not a function"))?;
        // `make`'s result resource type — pair it with the `call-g<n>` whose first param is that same type.
        let make_result = make.results(&*store).first().cloned();
        let want_res = match &make_result {
            Some(Type::Own(rt)) | Some(Type::Borrow(rt)) => *rt,
            other => {
                return Err(anyhow!(
                    "distinct-sig closure: `{make_name}` does not return a resource ({other:?})"
                ));
            }
        };
        // Find the matching call among `call-g<n>` funcs.
        let call_name = iface_funcs
            .iter()
            .filter(|f| f.starts_with("call-g"))
            .find(|cn| {
                let Some(idx) = instance.get_export_index(&mut *store, Some(&iface), cn) else {
                    return false;
                };
                let Some(cf) = instance.get_func(&mut *store, idx) else {
                    return false;
                };
                matches!(cf.params(&*store).first().map(|(_, t)| t.clone()),
                    Some(Type::Own(rt)) | Some(Type::Borrow(rt)) if rt == want_res)
            })
            .cloned()
            .ok_or_else(|| {
                anyhow!("distinct-sig closure: no `call-g<n>` matches `{make_name}`'s resource")
            })?;
        let call_idx = instance
            .get_export_index(&mut *store, Some(&iface), &call_name)
            .ok_or_else(|| anyhow!("distinct-sig closure: no `{call_name}`"))?;
        let call = instance
            .get_func(&mut *store, call_idx)
            .ok_or_else(|| anyhow!("distinct-sig closure: `{call_name}` is not a function"))?;
        // Split args by make's arity (as the multi-export path does).
        let make_param_types: Vec<Type> = make
            .params(&*store)
            .iter()
            .map(|(_, t)| t.clone())
            .collect();
        let n_make = make_param_types.len();
        if arg_strs.len() < n_make {
            return Err(anyhow!(
                "distinct-sig closure: `{make_name}` needs {n_make} arg(s)"
            ));
        }
        let make_args = coerce_args(&arg_strs[..n_make], &make_param_types)?;
        let mut handle = [Val::Bool(false)];
        if let Err(e) = make.call(&mut *store, &make_args, &mut handle) {
            return Ok(Outcome::Trap(trap_message(&e)));
        }
        let _ = make.post_return(&mut *store);
        let param_types: Vec<Type> = call
            .params(&*store)
            .iter()
            .map(|(_, t)| t.clone())
            .collect();
        let coerced = coerce_args(&arg_strs[n_make..], param_types.get(1..).unwrap_or(&[]))?;
        let mut call_args = vec![handle[0].clone()];
        call_args.extend(coerced);
        let mut out = [Val::Bool(false)];
        return match call.call(&mut *store, &call_args, &mut out) {
            Ok(()) => {
                let _ = call.post_return(&mut *store);
                Ok(Outcome::Value(render_closure_call_result(out.first())))
            }
            Err(e) => Ok(Outcome::Trap(trap_message(&e))),
        };
    }
    // The make function to call: a single-export program publishes a bare `make`; a MULTI-EXPORT program
    // publishes `make-<name>` per closure export, and the corpus `(call <name> …)` picks which. Try
    // `make-<export>` first (multi), then the bare `make` (single) — so a single-export case with a `--call
    // main` still resolves `make`.
    // Public export names are KEBAB; normalize `make-<src>` the same way emit did (`make-mkAdder` →
    // `make-mk-adder`) so a multi-export lookup by source name matches.
    let make_name = match export {
        Some(name)
            if {
                let mk = cadenza_syntax::extern_name::kebab_extern_name(&format!("make-{name}"));
                instance
                    .get_export_index(&mut *store, Some(&iface), &mk)
                    .is_some()
            } =>
        {
            cadenza_syntax::extern_name::kebab_extern_name(&format!("make-{name}"))
        }
        _ => "make".to_string(),
    };
    let make_idx = instance
        .get_export_index(&mut *store, Some(&iface), &make_name)
        .ok_or_else(|| anyhow!("closure escape: `{CLOSURE_INTERFACE}` exports no `{make_name}`"))?;
    let call_idx = instance
        .get_export_index(&mut *store, Some(&iface), "call")
        .ok_or_else(|| anyhow!("closure escape: `{CLOSURE_INTERFACE}` exports no `call`"))?;
    let make = instance
        .get_func(&mut *store, make_idx)
        .ok_or_else(|| anyhow!("closure escape: `make` is not a function"))?;
    let call = instance
        .get_func(&mut *store, call_idx)
        .ok_or_else(|| anyhow!("closure escape: `call` is not a function"))?;

    // SPLIT the flat arg list by `make`'s arity: the first `make.params().len()` go to `make` (the export
    // params), the rest to `call` (after its leading `self`).
    let make_param_types: Vec<Type> = make
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let n_make = make_param_types.len();
    if arg_strs.len() < n_make {
        return Err(anyhow!(
            "closure escape: `make` needs {n_make} argument(s) but only {} supplied",
            arg_strs.len()
        ));
    }
    let make_args = coerce_args(&arg_strs[..n_make], &make_param_types)?;
    let mut handle = [Val::Bool(false)];
    if let Err(e) = make.call(&mut *store, &make_args, &mut handle) {
        return Ok(Outcome::Trap(trap_message(&e)));
    }
    let _ = make.post_return(&mut *store);
    // `call`'s params are `(self, args…)`; coerce the REMAINING arg strings to the DECLARED arg types
    // (skipping the leading `self` handle param).
    let param_types: Vec<Type> = call
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let arg_types = param_types.get(1..).unwrap_or(&[]);
    let coerced = coerce_args(&arg_strs[n_make..], arg_types)?;
    let mut call_args = vec![handle[0].clone()];
    call_args.extend(coerced);
    let mut out = [Val::Bool(false)];
    match call.call(&mut *store, &call_args, &mut out) {
        Ok(()) => {
            let _ = call.post_return(&mut *store);
            Ok(Outcome::Value(render_closure_call_result(out.first())))
        }
        Err(e) => Ok(Outcome::Trap(trap_message(&e))),
    }
}

/// Run a resource-escape program: reach `make`/`encode` inside the `cadenza:run/run` instance, call
/// `make()` → a resource handle, `encode(handle)` → the canonical binary value form as `list<u8>`,
/// then DECODE those bytes to `Arenas` and print `(: value type)`. The type travels WITH the value (the
/// encoded s-expression is `(: <value> <type>)`), so the host spells no type name — it decodes and
/// prints. Mirrors the compiler's `constant_value_form`/resource-envelope emission
/// ([[rcdzc-r1-resource-encode-linking-findings]]).
fn run_resource_escape(
    store: &mut Store<()>,
    instance: &wasmtime::component::Instance,
    args: &[String],
) -> Result<Outcome> {
    let iface = instance
        .get_export_index(&mut *store, None, RUN_INTERFACE)
        .ok_or_else(|| anyhow!("resource escape: no `{RUN_INTERFACE}` instance export"))?;
    let make_idx = instance
        .get_export_index(&mut *store, Some(&iface), "make")
        .ok_or_else(|| anyhow!("resource escape: `{RUN_INTERFACE}` exports no `make`"))?;
    let encode_idx = instance
        .get_export_index(&mut *store, Some(&iface), "encode")
        .ok_or_else(|| anyhow!("resource escape: `{RUN_INTERFACE}` exports no `encode`"))?;
    let make = instance
        .get_func(&mut *store, make_idx)
        .ok_or_else(|| anyhow!("resource escape: `make` is not a function"))?;
    let encode = instance
        .get_func(&mut *store, encode_idx)
        .ok_or_else(|| anyhow!("resource escape: `encode` is not a function"))?;

    // `make` forwards the escaping export's parameters: a NULLARY export takes no args (`make()`); a
    // PARAMETERIZED export (`(def (main (: a Int64)) …)`) takes the `(call …)` args, so the host computes
    // the heap value from its inputs. Coerce the raw arg strings to `make`'s declared param types.
    let make_param_types: Vec<Type> = make
        .params(&*store)
        .iter()
        .map(|(_, t)| t.clone())
        .collect();
    let make_args = coerce_args(args, &make_param_types)?;
    let mut handle = [Val::Bool(false)];
    if let Err(e) = make.call(&mut *store, &make_args, &mut handle) {
        return Ok(Outcome::Trap(trap_message(&e)));
    }
    let _ = make.post_return(&mut *store);
    let mut out = [Val::Bool(false)];
    if let Err(e) = encode.call(&mut *store, &handle, &mut out) {
        return Ok(Outcome::Trap(trap_message(&e)));
    }
    let _ = encode.post_return(&mut *store);

    let bytes: Vec<u8> = match &out[0] {
        Val::List(items) => items
            .iter()
            .map(|v| match v {
                Val::U8(b) => Ok(*b),
                o => Err(anyhow!(
                    "resource escape: encode returned a non-u8 element {o:?}"
                )),
            })
            .collect::<Result<_>>()?,
        o => {
            return Err(anyhow!(
                "resource escape: encode returned {o:?}, expected list<u8>"
            ));
        }
    };
    let arenas = cadenza_syntax::codec::decode(&bytes).ok_or_else(|| {
        anyhow!("resource escape: encode bytes are not a valid canonical value form")
    })?;
    Ok(Outcome::Value(
        cadenza_syntax::sexpr::print(&arenas).trim().to_string(),
    ))
}

/// Render a closure `call`'s result value. A scalar/String comes back directly; a `list<u8>` may be EITHER
/// a raw byte-rope result (a `Bytes`/`String` closure — render the bare byte sequence `(5 6)`) OR the
/// canonical VALUE FORM of a compound result (tuple/record/sum — decode + pretty-print `(: value T)`). The
/// two are disambiguated by TRYING to decode: `codec::decode` is total and refuses any bytes whose 8-byte
/// schema header it does not recognize, so a raw byte-rope (which lacks that header) declines and falls
/// through to the raw-list render — no ambiguity, no flag needed.
fn render_closure_call_result(v: Option<&Val>) -> String {
    match v {
        None => "unit".to_string(),
        Some(Val::String(s)) => s.clone(),
        Some(Val::List(items)) => {
            // Try the value-form decode first (a compound result); fall back to the raw byte-rope render.
            let bytes: Option<Vec<u8>> = items
                .iter()
                .map(|e| match e {
                    Val::U8(b) => Some(*b),
                    _ => None,
                })
                .collect();
            if let Some(bytes) = bytes
                && let Some(arenas) = cadenza_syntax::codec::decode(&bytes)
            {
                return cadenza_syntax::sexpr::print(&arenas).trim().to_string();
            }
            render_val(v.unwrap())
        }
        Some(other) => render_val(other),
    }
}

/// Coerce each raw CLI argument string to the corresponding declared parameter type. The arity must
/// match; each scalar type parses from its natural text form. Compound param types are not yet
/// supported (no export takes them today) and are an explicit error rather than a silent guess.
fn coerce_args(raw: &[String], types: &[Type]) -> Result<Vec<Val>> {
    if raw.len() != types.len() {
        return Err(anyhow!(
            "argument count mismatch: the export takes {} argument(s), {} given",
            types.len(),
            raw.len()
        ));
    }
    raw.iter()
        .zip(types)
        .map(|(s, t)| coerce_one(s, t))
        .collect()
}

fn coerce_one(s: &str, t: &Type) -> Result<Val> {
    let parse = |ok: Option<Val>| ok.ok_or_else(|| anyhow!("cannot parse `{s}` as {t:?}"));
    Ok(match t {
        Type::Bool => parse(s.parse::<bool>().ok().map(Val::Bool))?,
        Type::S8 => parse(s.parse::<i8>().ok().map(Val::S8))?,
        Type::U8 => parse(s.parse::<u8>().ok().map(Val::U8))?,
        Type::S16 => parse(s.parse::<i16>().ok().map(Val::S16))?,
        Type::U16 => parse(s.parse::<u16>().ok().map(Val::U16))?,
        Type::S32 => parse(s.parse::<i32>().ok().map(Val::S32))?,
        Type::U32 => parse(s.parse::<u32>().ok().map(Val::U32))?,
        Type::S64 => parse(s.parse::<i64>().ok().map(Val::S64))?,
        Type::U64 => parse(s.parse::<u64>().ok().map(Val::U64))?,
        Type::Float32 => parse(s.parse::<f32>().ok().map(Val::Float32))?,
        Type::Float64 => parse(s.parse::<f64>().ok().map(Val::Float64))?,
        Type::Char => parse(
            s.chars()
                .next()
                .filter(|_| s.chars().count() == 1)
                .map(Val::Char),
        )?,
        Type::String => Val::String(s.to_string()),
        // A FIXED-SHAPE tuple argument (the direct-call compound-arg path): the host supplies it as a
        // component `tuple<…>` value, which the canonical ABI flattens into the guest's core params. The
        // corpus writes it as `(tuple <f0> <f1> …)` (an optional leading `tuple` head, else a bare
        // `(<f0> <f1> …)`); parse the paren-wrapped, whitespace-separated fields and coerce each against
        // the tuple's element types. Fields must be scalars (this increment supports a fixed-shape SCALAR
        // tuple; a nested compound field would recurse, a later widening).
        Type::Tuple(tt) => {
            let elem_types: Vec<Type> = tt.types().collect();
            let mut fields = parse_tuple_fields(s).ok_or_else(|| {
                anyhow!("argument `{s}`: expected a tuple literal like `(tuple 3 4)` or `(3 4)`")
            })?;
            if fields.len() != elem_types.len() {
                return Err(anyhow!(
                    "argument `{s}`: tuple has {} field(s), the parameter type expects {}",
                    fields.len(),
                    elem_types.len()
                ));
            }
            // A RECORD closure argument erases to a `tuple<…>` whose fields are laid in canonical SORTED-name
            // order (`tuple_field_abi` / `Core::Record`: a `BTreeMap` over field names). The corpus writes the
            // record value `(record (z 100) (a 3))` in SOURCE order, so when EVERY field is a `(name value)`
            // group, sort the fields by name to match the boundary tuple's positions before coercing. A plain
            // positional tuple (bare scalar fields) is left untouched.
            if fields.iter().all(|f| named_field(f).is_some()) {
                fields.sort_by(|a, b| named_field(a).unwrap().0.cmp(&named_field(b).unwrap().0));
            }
            let vals: Result<Vec<Val>> = fields
                .iter()
                .zip(&elem_types)
                .map(|(f, ft)| coerce_one(&unwrap_named_field(f), ft))
                .collect();
            Val::Tuple(vals?)
        }
        other => {
            return Err(anyhow!(
                "argument `{s}`: compound parameter type {other:?} is not supported by cdz-run yet"
            ));
        }
    })
}

/// If `field` is a RECORD-field group `(name value)` — a 2-element paren group whose first element is a bare
/// field NAME (an identifier, not a number/bool) — return `(name, value)`. A record closure argument erases to
/// a component `tuple<…>` at the boundary, so the corpus's record VALUE `(record (x 10) (y 3))` presents each
/// field this way; the driver reorders + unwraps them. Returns `None` for a plain scalar or a positional
/// tuple field (so those stay untouched).
fn named_field(field: &str) -> Option<(String, String)> {
    let parts = parse_tuple_fields(field)?;
    if parts.len() == 2
        && !parts[0].is_empty()
        && parts[0]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        && parts[0].parse::<i128>().is_err()
        && parts[0].parse::<f64>().is_err()
        && parts[0] != "true"
        && parts[0] != "false"
    {
        Some((parts[0].clone(), parts[1].clone()))
    } else {
        None
    }
}

/// Unwrap a record-field group `(name value)` to its VALUE (`(x 10)` → `10`); leave a plain scalar or a
/// positional/nested-compound field unchanged so nested tuples still coerce recursively.
fn unwrap_named_field(field: &str) -> String {
    named_field(field)
        .map(|(_, v)| v)
        .unwrap_or_else(|| field.to_string())
}

/// Parse a corpus tuple argument literal into its field texts. Accepts `(tuple f0 f1 …)` (the canonical
/// value-form spelling the corpus renders) or a bare `(f0 f1 …)`; the outer parens are required. Fields are
/// split on whitespace at the TOP level (a nested `(…)` field stays one token so a nested compound can be
/// coerced recursively later). Returns `None` if `s` is not a paren-wrapped group. This is a minimal
/// scalar-field splitter — sufficient for a fixed-shape SCALAR tuple, where every field is a bare token.
fn parse_tuple_fields(s: &str) -> Option<Vec<String>> {
    let inner = s.trim().strip_prefix('(')?.strip_suffix(')')?.trim();
    // Split on whitespace, respecting nested parens (a nested `(…)` field is one token).
    let mut fields = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    fields.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        fields.push(cur);
    }
    // Drop an optional leading `tuple`/`record` head token (the canonical value-form spelling).
    if let Some(first) = fields.first()
        && (first == "tuple" || first == "record")
    {
        fields.remove(0);
    }
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_is_invalid() {
        assert!(validate(&[]).is_err());
    }

    #[test]
    fn hash_extracted_from_pinned_import() {
        let name = "cadenza:runtime/heap@0.0.0+abc123";
        assert!(import_is_runtime(name));
        assert_eq!(hash_from_import(name), "abc123");
    }

    #[test]
    fn bare_interface_import_has_no_hash() {
        assert!(import_is_runtime(RUNTIME_IFACE));
        assert_eq!(hash_from_import(RUNTIME_IFACE), "");
    }

    #[test]
    fn non_runtime_import_is_not_matched() {
        assert!(!import_is_runtime("cadenza:host/emit-event"));
    }

    #[test]
    fn host_response_scalar_extracted_from_value_form() {
        // A `(: v T)` form yields the bare value; a bare value passes through; whitespace is tolerated.
        assert_eq!(scalar_of_value_form("(: 10 Int64)"), "10");
        assert_eq!(scalar_of_value_form("(: 42 Int64)"), "42");
        assert_eq!(scalar_of_value_form("7"), "7");
        assert_eq!(scalar_of_value_form("  3  "), "3");
    }

    #[test]
    fn runtime_import_name_recognized() {
        // The host-import binder skips the value-heap runtime instance (bound elsewhere).
        assert!(is_runtime_import_name("cadenza:runtime/heap@0.0.0+abc"));
        assert!(!is_runtime_import_name("ask"));
    }

    #[test]
    fn tuple_fields_split_at_top_level() {
        // Bare and `tuple`-headed spellings both split into their scalar fields; a nested group stays whole.
        assert_eq!(parse_tuple_fields("(10 3)").unwrap(), vec!["10", "3"]);
        assert_eq!(parse_tuple_fields("(tuple 10 3)").unwrap(), vec!["10", "3"]);
        assert_eq!(
            parse_tuple_fields("(record (x 10) (y 3))").unwrap(),
            vec!["(x 10)", "(y 3)"]
        );
        assert!(parse_tuple_fields("10").is_none()); // not a paren group
    }

    #[test]
    fn named_record_field_detected_and_unwrapped() {
        // A `(name value)` group is recognized as a record field and unwraps to its value.
        assert_eq!(
            named_field("(x 10)"),
            Some(("x".to_string(), "10".to_string()))
        );
        // A named field with a Bool value (the name is a real identifier).
        assert_eq!(
            named_field("(flag true)"),
            Some(("flag".to_string(), "true".to_string()))
        );
        assert_eq!(named_field("(10 3)"), None); // numeric head → a positional tuple, not a record field
        assert_eq!(named_field("(true false)"), None); // a positional Bool tuple, not a named field
        assert_eq!(named_field("10"), None); // a bare scalar
        assert_eq!(unwrap_named_field("(x 10)"), "10");
        assert_eq!(unwrap_named_field("10"), "10");
    }
}
