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
    bind_host_imports(&engine, &component, &mut linker, opts, &observed)?;

    let outcome = run_export(&engine, &component, &mut store, &linker, opts)?;
    let calls = observed.lock().expect("observed calls mutex").clone();
    Ok((outcome, calls))
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

    // The RESOURCE ESCAPE (`DESIGN-value-heap-rcdzc.md` §3a): a program whose result is a COMPOUND
    // exports no bare function — it publishes a `cadenza:run/run` instance carrying `make : () -> own<t>`
    // + `encode : (own<t>) -> list<u8>`. Call `make` then `encode`, DECODE the canonical binary value
    // form with the shared codec, and pretty-print `(: value type)` — the value crossing the boundary as
    // a strongly-typed resource, rendered by the host (not spelled out in wasm). Taken when no explicit
    // `--call` names a bare function.
    if opts.export.is_none()
        && sole_func_export(engine, component).is_none()
        && has_run_instance(engine, component)
    {
        return run_resource_escape(&mut *store, &instance);
    }

    // The CLOSURE ESCAPE (`DESIGN-closure-host-resource-rcdzc.md`, C-HOST-1): a program whose result is a
    // closure exports the `cadenza:closure/exports` instance (`make`/`call`), not a bare function. Call
    // `make()` → the closure handle, then `call(handle, args…)` with the caller's arguments, rendering the
    // result. Taken when the closure interface is present AND there is no bare function to call directly —
    // even when a `--call <name>` was given (the corpus names the entry `main`, but a closure export has no
    // bare `main` function; the args are the closure's arguments).
    if sole_func_export(engine, component).is_none()
        && has_closure_instance(engine, component)
        && opts
            .export
            .as_deref()
            .map(|name| instance.get_func(&mut *store, name).is_none())
            .unwrap_or(true)
    {
        return run_closure_resource(&mut *store, &instance, &opts.args);
    }

    // Resolve the export to call: the named one, or the sole function export found by signature.
    let export_name = match &opts.export {
        Some(name) => name.clone(),
        None => sole_func_export(engine, component).ok_or_else(|| {
            anyhow!("no --call given and the component has no single function export to default to")
        })?,
    };
    let func = instance
        .get_func(&mut *store, &export_name)
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
        Err(e) => Ok(Outcome::Trap(format!("{e}"))),
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
    let runtime_bytes = opts.runtime.as_deref().ok_or_else(|| {
        anyhow!(
            "component requires the value-heap runtime {} but none was provided (the host resolves \
             it by content address from the store; build it with `cargo xtask build`)",
            req.hash
        )
    })?;
    let runtime = load_runtime_component(engine, runtime_bytes, &req.hash, opts)?;

    // Discover the heap functions the runtime exports (name-by-name), off its component type. The
    // runtime component exports the interface under its plain, un-pinned name `cadenza:runtime/heap`.
    let heap_func_names = heap_interface_funcs(engine, &runtime)?;

    let rt_linker: Linker<()> = Linker::new(engine);
    let rt_instance = rt_linker
        .instantiate(&mut *store, &runtime)
        .map_err(|e| anyhow!("instantiate runtime: {e}"))?;
    let heap_idx = rt_instance
        .get_export_index(&mut *store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;

    // Bind under the program's exact (hashed) import name, not the bare interface — that is the name
    // the program declared, and the linker matches names verbatim.
    let mut iface = linker
        .instance(&req.import_name)
        .map_err(|e| anyhow!("linker instance {}: {e}", req.import_name))?;
    for fname in &heap_func_names {
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
            if is_runtime_import_name(name) {
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
            iface.func_new(&fname, move |_ctx, _params, results| {
                // OBSERVE the call — append its dotted `E.op` in call order (so the gate can verify the
                // sequence against `(host-calls …)`).
                observed
                    .lock()
                    .expect("observed calls mutex")
                    .push(op_label.clone());
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

/// Run a CLOSURE-resource program: reach `make`/`call` inside the `cadenza:closure/exports` instance,
/// call `make()` → the closure resource handle, then `call(handle, args…)` with the caller's arguments
/// (coerced to `call`'s declared parameter types) → the closure's result, rendered. The host acts as the
/// closure's custodian: it holds the opaque handle and invokes the guest's `call` method (which dispatches
/// the closure via the guest's own `call_indirect`). `own<t>` consumes the handle, so this is one call per
/// `make` (the corpus drives a single `(call …)` per case).
fn run_closure_resource(
    store: &mut Store<()>,
    instance: &wasmtime::component::Instance,
    arg_strs: &[String],
) -> Result<Outcome> {
    let iface = instance
        .get_export_index(&mut *store, None, CLOSURE_INTERFACE)
        .ok_or_else(|| anyhow!("closure escape: no `{CLOSURE_INTERFACE}` instance export"))?;
    let make_idx = instance
        .get_export_index(&mut *store, Some(&iface), "make")
        .ok_or_else(|| anyhow!("closure escape: `{CLOSURE_INTERFACE}` exports no `make`"))?;
    let call_idx = instance
        .get_export_index(&mut *store, Some(&iface), "call")
        .ok_or_else(|| anyhow!("closure escape: `{CLOSURE_INTERFACE}` exports no `call`"))?;
    let make = instance
        .get_func(&mut *store, make_idx)
        .ok_or_else(|| anyhow!("closure escape: `make` is not a function"))?;
    let call = instance
        .get_func(&mut *store, call_idx)
        .ok_or_else(|| anyhow!("closure escape: `call` is not a function"))?;

    let mut handle = [Val::Bool(false)];
    if let Err(e) = make.call(&mut *store, &[], &mut handle) {
        return Ok(Outcome::Trap(format!("{e}")));
    }
    let _ = make.post_return(&mut *store);
    // `call`'s params are `(self, args…)`; coerce the caller's arg strings to the DECLARED arg types
    // (skipping the leading `self` handle param).
    let param_types: Vec<Type> = call.params(&*store).iter().map(|(_, t)| t.clone()).collect();
    let arg_types = param_types.get(1..).unwrap_or(&[]);
    let coerced = coerce_args(arg_strs, arg_types)?;
    let mut call_args = vec![handle[0].clone()];
    call_args.extend(coerced);
    let mut out = [Val::Bool(false)];
    match call.call(&mut *store, &call_args, &mut out) {
        Ok(()) => {
            let _ = call.post_return(&mut *store);
            Ok(Outcome::Value(match out.first() {
                None => "unit".to_string(),
                Some(Val::String(s)) => s.clone(),
                Some(other) => render_val(other),
            }))
        }
        Err(e) => Ok(Outcome::Trap(format!("{e}"))),
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

    let mut handle = [Val::Bool(false)];
    if let Err(e) = make.call(&mut *store, &[], &mut handle) {
        return Ok(Outcome::Trap(format!("{e}")));
    }
    let _ = make.post_return(&mut *store);
    let mut out = [Val::Bool(false)];
    if let Err(e) = encode.call(&mut *store, &handle, &mut out) {
        return Ok(Outcome::Trap(format!("{e}")));
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
        other => {
            return Err(anyhow!(
                "argument `{s}`: compound parameter type {other:?} is not supported by cdz-run yet"
            ));
        }
    })
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
}
