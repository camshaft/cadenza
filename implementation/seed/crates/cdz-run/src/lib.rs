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
use wasmtime::{Engine, Store};

mod render;
pub use render::render_val;

/// The well-known import a compound-returning program declares; the host composes the value-heap
/// runtime to satisfy it (component-abi.md §The Value-Heap Runtime Crosses By A Well-Known Import).
const RUNTIME_IFACE: &str = "cadenza:runtime/heap";

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The export returned; its result rendered to canonical text (`unit` for a no-result export).
    Value(String),
    /// The export trapped at run time (message).
    Trap(String),
}

/// How to run a component: which export, what arguments, and where the value-heap runtime lives.
#[derive(Debug, Default, Clone)]
pub struct RunOpts {
    /// The export to invoke. `None` selects the sole function export (by signature) — the common
    /// case for a scalar entry, whose ABI is `() -> scalar` and whose name the compiler emits verbatim.
    pub export: Option<String>,
    /// Raw, still-untyped argument strings from the CLI; coerced to the export's declared param types.
    pub args: Vec<String>,
    /// The value-heap runtime component bytes, if the caller resolved one. Required only when the
    /// component imports `cadenza:runtime/heap`.
    pub runtime: Option<Vec<u8>>,
}

/// Validate `component_bytes` as a well-formed component — the cheap structural check before a run.
pub fn validate(component_bytes: &[u8]) -> Result<()> {
    let engine = Engine::default();
    Component::new(&engine, component_bytes).map(|_| ()).map_err(|e| anyhow!("invalid component: {e}"))
}

/// Instantiate `component_bytes`, compose the value-heap runtime if imported, invoke the chosen
/// export with the (coerced) arguments, and return the rendered outcome.
pub fn run(component_bytes: &[u8], opts: &RunOpts) -> Result<Outcome> {
    let engine = Engine::default();
    let component =
        Component::new(&engine, component_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;

    let mut linker: Linker<()> = Linker::new(&engine);

    // If the component imports the value-heap runtime, satisfy that import by forwarding every
    // function the runtime's `cadenza:runtime/heap` instance exports. The function set is DISCOVERED
    // from the runtime component's own type — never a hard-coded list — so it can never drift from
    // the runtime the caller actually supplied.
    let mut store = Store::new(&engine, ());
    if component_imports_runtime(&engine, &component) {
        compose_runtime(&engine, &mut store, &mut linker, opts)?;
    }

    let instance =
        linker.instantiate(&mut store, &component).map_err(|e| anyhow!("instantiate: {e}"))?;

    // Resolve the export to call: the named one, or the sole function export found by signature.
    let export_name = match &opts.export {
        Some(name) => name.clone(),
        None => sole_func_export(&engine, &component)
            .ok_or_else(|| anyhow!("no --call given and the component has no single function export to default to"))?,
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

/// Does `component` import the value-heap runtime interface?
fn component_imports_runtime(engine: &Engine, component: &Component) -> bool {
    component.component_type().imports(engine).any(|(name, _)| name == RUNTIME_IFACE)
}

/// Compose the value-heap runtime: instantiate the runtime component, then forward each function its
/// `cadenza:runtime/heap` export exposes into the program's matching import. The function names are
/// read off the runtime's own instance type, so the composition always matches the supplied runtime.
fn compose_runtime(
    engine: &Engine,
    store: &mut Store<()>,
    linker: &mut Linker<()>,
    opts: &RunOpts,
) -> Result<()> {
    let runtime_bytes = opts
        .runtime
        .as_deref()
        .ok_or_else(|| anyhow!("component imports {RUNTIME_IFACE} but no runtime was provided (pass --runtime, or build the store with `cargo xtask build`)"))?;
    let runtime = Component::new(engine, runtime_bytes)
        .map_err(|e| anyhow!("value-heap runtime component invalid: {e}"))?;

    // Discover the heap functions the runtime exports (name-by-name), off its component type.
    let heap_func_names = heap_interface_funcs(engine, &runtime)?;

    let rt_linker: Linker<()> = Linker::new(engine);
    let rt_instance =
        rt_linker.instantiate(&mut *store, &runtime).map_err(|e| anyhow!("instantiate runtime: {e}"))?;
    let heap_idx = rt_instance
        .get_export_index(&mut *store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;

    let mut iface =
        linker.instance(RUNTIME_IFACE).map_err(|e| anyhow!("linker instance {RUNTIME_IFACE}: {e}"))?;
    for fname in &heap_func_names {
        let fidx = rt_instance
            .get_export_index(&mut *store, Some(&heap_idx), fname)
            .ok_or_else(|| anyhow!("runtime missing `{fname}`"))?;
        let f = rt_instance
            .get_func(&mut *store, &fidx)
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
                .filter_map(|(fname, i)| matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string()))
                .collect());
        }
    }
    Err(anyhow!("runtime component does not export the {RUNTIME_IFACE} interface"))
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
    raw.iter().zip(types).map(|(s, t)| coerce_one(s, t)).collect()
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
        Type::Char => parse(s.chars().next().filter(|_| s.chars().count() == 1).map(Val::Char))?,
        Type::String => Val::String(s.to_string()),
        other => {
            return Err(anyhow!(
                "argument `{s}`: compound parameter type {other:?} is not supported by cdz-run yet"
            ));
        }
    })
}

/// Shared: this is a component that imports the value-heap runtime (so a caller knows a runtime is
/// required before running).
pub fn needs_runtime(component_bytes: &[u8]) -> Result<bool> {
    let engine = Engine::default();
    let component =
        Component::new(&engine, component_bytes).map_err(|e| anyhow!("invalid component: {e}"))?;
    Ok(component_imports_runtime(&engine, &component))
}

// A tiny witness that the crate wires together; real behavior is exercised end-to-end against
// components built by `xtask`/`rcdzc` (see the integration checks).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_is_invalid() {
        assert!(validate(&[]).is_err());
    }
}
