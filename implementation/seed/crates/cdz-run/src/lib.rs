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
}

/// Validate `component_bytes` as a well-formed component — the cheap structural check before a run.
pub fn validate(component_bytes: &[u8]) -> Result<()> {
    let engine = engine();
    Component::new(&engine, component_bytes)
        .map(|_| ())
        .map_err(|e| anyhow!("invalid component: {e}"))
}

/// Instantiate `component_bytes`, compose the value-heap runtime if imported, invoke the chosen
/// export with the (coerced) arguments, and return the rendered outcome.
pub fn run(component_bytes: &[u8], opts: &RunOpts) -> Result<Outcome> {
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

    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| anyhow!("instantiate: {e}"))?;

    // The RESOURCE ESCAPE (`DESIGN-value-heap-rcdzc.md` §3a): a program whose result is a COMPOUND
    // exports no bare function — it publishes a `cadenza:run/run` instance carrying `make : () -> own<t>`
    // + `encode : (own<t>) -> list<u8>`. Call `make` then `encode`, DECODE the canonical binary value
    // form with the shared codec, and pretty-print `(: value type)` — the value crossing the boundary as
    // a strongly-typed resource, rendered by the host (not spelled out in wasm). Taken when no explicit
    // `--call` names a bare function.
    if opts.export.is_none()
        && sole_func_export(&engine, &component).is_none()
        && has_run_instance(&engine, &component)
    {
        return run_resource_escape(&mut store, &instance);
    }

    // Resolve the export to call: the named one, or the sole function export found by signature.
    let export_name = match &opts.export {
        Some(name) => name.clone(),
        None => sole_func_export(&engine, &component).ok_or_else(|| {
            anyhow!("no --call given and the component has no single function export to default to")
        })?,
    };
    let func = instance
        .get_func(&mut store, &export_name)
        .ok_or_else(|| anyhow!("component exports no function `{export_name}`"))?;

    // Coerce the raw argument strings to the export's declared parameter types.
    let param_types: Vec<Type> = func.params(&store).iter().map(|(_, t)| t.clone()).collect();
    let args = coerce_args(&opts.args, &param_types)?;

    let result_count = func.results(&store).len();
    let mut results = vec![Val::Bool(false); result_count];
    match func.call(&mut store, &args, &mut results) {
        Ok(()) => {
            let rendered = match results.first() {
                None => "unit".to_string(),
                // A compound program's entry returns its result ALREADY rendered to canonical text
                // (the program walked its value through the runtime and assembled the string); take a
                // returned string verbatim rather than re-quoting it. A scalar result renders directly.
                Some(Val::String(s)) => s.clone(),
                Some(other) => render_val(other),
            };
            let _ = func.post_return(&mut store);
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
    let runtime = Component::new(engine, runtime_bytes)
        .map_err(|e| anyhow!("value-heap runtime component invalid: {e}"))?;

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
}
