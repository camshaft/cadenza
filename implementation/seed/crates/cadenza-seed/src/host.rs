//! The minimal host — a separate artifact from the interpreter and the compiler that
//! runs a *finished* (Cadenza-emitted) component and provides only the capability
//! operations the component's manifest enumerates (build-tool-interface.md §"The
//! Interpreter And The Host Are Distinct Artifacts"; §"The host … MUST provide only the
//! operations the component's manifest enumerates"; options/execution-model/).
//!
//! It uses the embeddable wasmtime crate in-process (the wasmtime CLI is not used). The
//! host NEVER produces or completes component bytes — those come wholly from the
//! Cadenza-authored compiler (spec/bootstrap.md §"The Compiler Is Authored In Cadenza,
//! Not In The Seed"); the host only instantiates and runs them, and validates them as an
//! out-of-band oracle.

use anyhow::{anyhow, Result};
use wasmtime::component::{Component, Linker};
pub use wasmtime::component::Val;
use wasmtime::{Engine, Store};

/// A host call a run made: the host function's name and its arguments rendered to canonical text
/// (the observable the corpus's `(host-calls (call NAME (: arg Type)…))` fixture pins).
#[derive(Debug, Clone, PartialEq)]
pub struct HostCall {
    pub name: String,
    pub args: Vec<String>,
}

/// Observation + resume state threaded through the store. A host function records its call here
/// and reads its response from the log (they must be Send+Sync, so we thread via the Store rather
/// than a captured Rc).
///
/// This is shaped for the THREE host-call resume modes, all served by the same synchronous
/// component bytes (the mode is a host execution choice — capabilities-and-effects.md §Suspension
/// Is Replay From The Host's Log): (A) immediate — a logged response is returned inline; (B)
/// suspend + abort — no logged response, the run unwinds and is later re-invoked from its entry
/// replaying the extended log; (C) suspend in place — a kept-alive async fiber resumes without
/// replay (a documented seam, wired when we run under wasmtime async). Modes B and C are
/// unobservable to the program: they produce identical host-call sequences.
#[derive(Default)]
pub struct HostState {
    /// Events the running component emitted, in order (kind, payload-as-string) — the legacy
    /// `emit-event` observation channel, kept for existing cases.
    pub events: Vec<(String, String)>,
    /// The ordered host calls the run made (name + rendered args) — the `(host-calls …)` observable.
    pub calls: Vec<HostCall>,
    /// The host's response log: for the i-th host call, `responses[i]` is the value the host feeds
    /// back (mode A). A call with no logged response is a suspension point (mode B); see `pending`.
    pub responses: Vec<Val>,
    /// Set when a host call had no logged response: the pending call the run suspended on. The
    /// caller turns this into `RunOutcome::Suspended`. (Mode B: extend the log and re-invoke;
    /// mode C: resume the kept-alive fiber — not yet wired.)
    pub pending: Option<HostCall>,
    /// The runtime's live heap-object count read AFTER `run()` returned, when the composed runtime
    /// exports `live-objects` (only the `debug-counters` build does). `Some(0)` proves the program's
    /// Perceus dup/drop discipline reclaimed every object; `Some(n>0)` is a LEAK. `None` when the
    /// runtime has no counter (default build) or the program did not compose a runtime. The leak-check
    /// harness (env `CADENZA_LEAK_CHECK`) asserts this is `Some(0)` for a heap-using run.
    pub live_after_run: Option<u32>,
}

/// The outcome of running a derived component's entry. The three arms mirror the entry's three
/// outcomes (component-abi.md §The Entry May Suspend On A Host Call): normal completion, a trap,
/// and a suspension carrying the pending host call.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    /// Normal termination with the entry's result rendered to a string for comparison.
    Value(String),
    /// The component trapped.
    Trap(String),
    /// The run suspended on a host call with no logged response. The continuation is the component
    /// + input + response log (capabilities-and-effects.md §A Durable Continuation Is Canonical
    /// Data); resuming = re-invoking the entry with the log extended by this call's response (mode
    /// B), or resuming a kept-alive async fiber (mode C, not yet wired). Not produced by any
    /// current corpus case (the resume cases are `needs effects`), but the API names the outcome.
    #[allow(dead_code)]
    Suspended(HostCall),
}

/// Validate that `component_bytes` is a well-formed WebAssembly component. This is the
/// out-of-band ORACLE check permitted by options/execution-model/ — it never produces or
/// alters the bytes, only confirms the Cadenza compiler emitted a real component.
pub fn validate_component(component_bytes: &[u8]) -> Result<()> {
    let engine = Engine::default();
    Component::new(&engine, component_bytes)
        .map(|_| ())
        .map_err(|e| anyhow!("component failed validation: {e}"))
}

/// Instantiate and run a derived component, binding ONLY the capabilities in `manifest`.
///
/// `manifest` is the exact set of capability operation names the component's manifest
/// enumerates; the host binds those and no others (host-interface-binding.md §"Imports
/// Mirror The Manifest Exactly"). The component's entry is `run` (options/execution-model/
/// §"Component entry shapes"); we detect its result arity and render it.
pub fn run_component(component_bytes: &[u8], manifest: &[String]) -> Result<(RunOutcome, HostState)> {
    run_component_with_responses(component_bytes, manifest, &[])
}

/// Run a component, seeding the host's response log with `responses` (the `(host-responses …)`
/// fixture, in call order). Each host call the run makes returns the next logged response (mode A);
/// a call past the end of the log suspends (mode B). This is the entry the behavior gate uses when
/// a case pins host calls/responses; `run_component` is the no-response wrapper.
pub fn run_component_with_responses(
    component_bytes: &[u8],
    manifest: &[String],
    responses: &[Val],
) -> Result<(RunOutcome, HostState)> {
    let engine = Engine::default();
    let component = Component::new(&engine, component_bytes)
        .map_err(|e| anyhow!("instantiate: component invalid: {e}"))?;

    let mut linker: Linker<HostState> = Linker::new(&engine);

    // Bind exactly the manifest's capability operations, and no others. `emit-event` is
    // the ignition capability; the set extends as the host interface grows. A manifest
    // that grants nothing binds nothing (a world with no import).
    for cap in manifest {
        match cap.as_str() {
            "emit-event" => {
                linker.root().func_wrap(
                    "emit-event",
                    |mut caller: wasmtime::StoreContextMut<'_, HostState>,
                     (kind, payload): (String, String)| {
                        caller.data_mut().events.push((kind, payload));
                        Ok(())
                    },
                )?;
            }
            other => return Err(anyhow!("host does not provide capability `{other}`")),
        }
    }

    // Bind each top-level host FUNCTION import the component declares (a `(import (host …))` the
    // compiler lowered to a component-level func import). Every such import is a capability
    // (capabilities-and-effects.md §A Host Import Is A Boundary Effect And The Manifest Is Its Row);
    // the value-heap runtime interface is the ONE exempt import and is handled separately below.
    // Each binding RECORDS the call (name + rendered args) and returns the logged response for that
    // call — mode A. A call with no logged response sets `pending` and traps to unwind the run
    // (mode B: the caller surfaces `Suspended`; resume = re-invoke with the extended log).
    bind_host_imports(&engine, &component, &mut linker)?;

    // A program that produces a runtime compound imports the value-heap runtime interface
    // `cadenza:runtime/heap` and the host COMPOSES it (component-abi.md §The Value-Heap Runtime
    // Crosses By A Well-Known Import; §The Host Resolves The Runtime By Content Address). This is
    // NOT a capability import — it adds nothing to the manifest (capabilities-and-effects.md §The
    // Value-Heap Runtime Is The One Import That Is Not A Capability). If the program imports the
    // runtime, satisfy that import by forwarding each heap function to a runtime instance.
    if component_imports_runtime(&engine, &component) {
        return run_with_runtime(&engine, &component, linker);
    }

    // Resolve the scalar ENTRY by its SIGNATURE — the sole nullary function export — not by a magic
    // name. The compiler exports every item under its SOURCE name verbatim (no `main`→`run` rename);
    // "which export is the entry" is the consumer's concern, and the entry ABI is `() -> scalar`, so
    // the host finds it by that shape. (The old compiler happened to name it `run`; this works for
    // both without baking a name in.)
    let entry_name = nullary_func_export(&engine, &component);

    let mut init = HostState::default();
    init.responses = responses.to_vec();
    let mut store = Store::new(&engine, init);
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| anyhow!("instantiate: {e}"))?;

    let run = match entry_name.as_deref().and_then(|n| instance.get_func(&mut store, n)) {
        Some(f) => f,
        // No nullary function export: a compound result crosses as the `cadenza:run/run` resource
        // owning `display()`. Call `make` → handle → `[method]value.display` → canonical text
        // (the host stays value-agnostic: it just invokes the value's own display method).
        None => return run_resource_display(&mut store, &instance),
    };

    // Determine result arity from the function type and call accordingly.
    let result_count = run.results(&store).len();
    let mut results = vec![Val::Bool(false); result_count];
    match run.call(&mut store, &[], &mut results) {
        Ok(()) => {
            let rendered = if results.is_empty() {
                "unit".to_string()
            } else {
                render_val(&results[0])
            };
            // wasmtime requires post_return after a successful call before re-use.
            let _ = run.post_return(&mut store);
            let state = std::mem::take(store.data_mut());
            Ok((RunOutcome::Value(rendered), state))
        }
        Err(e) => {
            let state = std::mem::take(store.data_mut());
            // A trap that carries a pending host call is a SUSPENSION (mode B), not a fault: the
            // run reached a host call with no logged response and unwound. Surface it as such so
            // the continuation (component + input + log) can be resumed by re-invocation.
            if let Some(pending) = state.pending.clone() {
                return Ok((RunOutcome::Suspended(pending), state));
            }
            Ok((RunOutcome::Trap(format!("{e}")), state))
        }
    }
}

/// Call ONE named scalar export of a no-import component and return its `i64` result, rendered to
/// the host comparison string. Unlike `run_component` (which resolves the single `run` entry), this
/// resolves an export BY NAME — the minimal machinery to prove a MULTI-export component presents and
/// individually invokes each of its exports. Test-scoped: no capabilities, no runtime composition.
pub fn call_scalar_export(component_bytes: &[u8], export: &str) -> Result<RunOutcome> {
    let engine = Engine::default();
    let component = Component::new(&engine, component_bytes)
        .map_err(|e| anyhow!("component invalid: {e}"))?;
    let linker: Linker<HostState> = Linker::new(&engine);
    let mut store = Store::new(&engine, HostState::default());
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| anyhow!("instantiate: {e}"))?;
    let func = instance
        .get_func(&mut store, export)
        .ok_or_else(|| anyhow!("component exports no `{export}`"))?;
    let result_count = func.results(&store).len();
    let mut results = vec![Val::Bool(false); result_count];
    match func.call(&mut store, &[], &mut results) {
        Ok(()) => {
            let rendered = if results.is_empty() { "unit".to_string() } else { render_val(&results[0]) };
            let _ = func.post_return(&mut store);
            Ok(RunOutcome::Value(rendered))
        }
        Err(e) => Ok(RunOutcome::Trap(format!("{e}"))),
    }
}

/// The name of the component's scalar ENTRY export, found by SIGNATURE: the sole top-level function
/// export taking no parameters (`() -> scalar`). The compiler names exports verbatim (no `main`→`run`
/// magic), so the host identifies the entry by its ABI shape rather than a well-known name. Returns
/// the first nullary function export's name, or `None` if there is none (then the result crosses as
/// the `cadenza:run/run` resource instead).
fn nullary_func_export(engine: &Engine, component: &Component) -> Option<String> {
    use wasmtime::component::types::ComponentItem;
    for (name, item) in component.component_type().exports(engine) {
        if let ComponentItem::ComponentFunc(f) = item {
            if f.params().len() == 0 {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Bind every top-level host FUNCTION import the component declares (skipping the value-heap
/// runtime interface, which is composed separately). Each binding records the call and returns the
/// logged response (mode A) or suspends (mode B). Uses the dynamic `func_new` API so ANY WIT-typed
/// host signature binds without a per-signature match arm — the vocabulary of host functions is the
/// target's concern (host-interface-binding.md §Which Host Functions Exist Is The Target's Concern).
fn bind_host_imports(
    engine: &Engine,
    component: &Component,
    linker: &mut Linker<HostState>,
) -> Result<()> {
    use wasmtime::component::types::ComponentItem;
    let ctype = component.component_type();
    // A delegated effect is imported as an INSTANCE (a WIT interface) whose exported functions are
    // its operations (the effect = the component namespace, the op = a function in it). Collect each
    // `(interface, op)` pair; the recorded call name is the flat `effect.op` (matching the corpus's
    // `(host-calls (call log.emit …))`). A top-level function import (legacy shape) is still bound
    // for robustness, recorded under its bare name.
    let mut instance_ops: Vec<(String, String)> = Vec::new(); // (interface, op)
    let mut toplevel_funcs: Vec<String> = Vec::new();
    for (name, item) in ctype.imports(engine) {
        if name == RUNTIME_IFACE {
            continue; // the one exempt import — composed, not a capability
        }
        match item {
            ComponentItem::ComponentInstance(inst) => {
                for (op, op_item) in inst.exports(engine) {
                    if let ComponentItem::ComponentFunc(_) = op_item {
                        instance_ops.push((name.to_string(), op.to_string()));
                    }
                }
            }
            ComponentItem::ComponentFunc(_) => toplevel_funcs.push(name.to_string()),
            _ => {}
        }
    }
    // Bind each interface's operations, recording the call as `effect.op`.
    for (iface, op) in instance_ops {
        let call_name = format!("{iface}.{op}");
        let mut inst = linker
            .instance(&iface)
            .map_err(|e| anyhow!("linker instance `{iface}`: {e}"))?;
        bind_recording_func(&mut inst, &op, call_name)?;
    }
    // Bind any top-level function import under its bare name (legacy / non-interface shape).
    for name in toplevel_funcs {
        let call_name = name.clone();
        bind_recording_func(&mut linker.root(), &name, call_name)?;
    }
    Ok(())
}

/// Bind `func_name` in `dst` (a linker root or interface) to a closure that RECORDS the call under
/// `call_name` (name + rendered args) and returns the next logged response (mode A), or — for a
/// value-returning call with no logged response — records it as the pending suspension and traps
/// (mode B). A unit-returning call with no response is fully serviced (nothing to feed back).
fn bind_recording_func(
    dst: &mut wasmtime::component::LinkerInstance<'_, HostState>,
    func_name: &str,
    call_name: String,
) -> Result<()> {
    dst.func_new(func_name, move |mut caller, args, results| {
        let idx = caller.data().calls.len();
        let rendered: Vec<String> = args.iter().map(render_val).collect();
        caller.data_mut().calls.push(HostCall { name: call_name.clone(), args: rendered.clone() });
        if let Some(resp) = caller.data().responses.get(idx).cloned() {
            if let Some(slot) = results.get_mut(0) {
                *slot = resp;
            }
            return Ok(());
        }
        if results.is_empty() {
            return Ok(());
        }
        caller.data_mut().pending = Some(HostCall { name: call_name.clone(), args: rendered });
        Err(anyhow!("host call suspended (no logged response): {call_name}"))
    })
}

/// The well-known value-heap runtime interface a runtime-compound program imports.
const RUNTIME_IFACE: &str = "cadenza:runtime/heap";

// The heap functions the host forwards into each emitted program's import (`RUNTIME_FUNCS`) are
// GENERATED from the runtime WIT by `xtask build` — the same source of truth as the compiler's
// envelope, so the two can never disagree. The set MUST cover every function the compiler-emitted
// program can import (RT_IMPORT_CONTENT); a program importing a function absent here fails to
// compose. Forwarded by name, in the compiler's import order.
use crate::runtime_funcs::RUNTIME_FUNCS;

/// Does `component` import the value-heap runtime interface? A program that produces a runtime
/// compound does; a scalar/const program does not (component-abi.md §The Value-Heap Runtime).
fn component_imports_runtime(engine: &Engine, component: &Component) -> bool {
    component
        .component_type()
        .imports(engine)
        .any(|(name, _)| name == RUNTIME_IFACE)
}

/// Locate the value-heap runtime component. For now the host resolves it from the content-
/// addressed store the xtask populates (component-abi.md §The Host Resolves The Runtime By
/// Content Address); the store directory is `$CADENZA_STORE` or the default build location, and
/// the concrete runtime is `$CADENZA_RUNTIME` if set, else the freshly-built runtime artifact.
/// (Phase B resolves the single current runtime; the hash-pinned lookup — read the required hash
/// the component records, find `<store>/<hash>.wasm` — layers on once the compiler bakes the hash.)
fn locate_runtime() -> Result<Vec<u8>> {
    if let Ok(path) = std::env::var("CADENZA_RUNTIME") {
        return std::fs::read(&path).map_err(|e| anyhow!("read CADENZA_RUNTIME `{path}`: {e}"));
    }
    // The runtime artifact the runtime crate builds (cargo component build --release).
    let candidates = [
        "implementation/seed/crates/cdz-runtime/target/wasm32-unknown-unknown/release/cdz_runtime.wasm",
    ];
    for c in candidates {
        if let Ok(bytes) = std::fs::read(c) {
            return Ok(bytes);
        }
    }
    Err(anyhow!(
        "value-heap runtime component not found (set CADENZA_RUNTIME, or build cdz-runtime)"
    ))
}

/// Run a program that imports the value-heap runtime by COMPOSING the two: instantiate the
/// runtime, then satisfy the program's `cadenza:runtime/heap` import by forwarding each heap
/// function to the runtime instance, then invoke the program's `run` (which returns the rendered
/// string — the program itself walks its result value through the runtime's accessors and
/// assembles the canonical text). The host holds no value structure; it reads back the string.
fn run_with_runtime(
    engine: &Engine,
    program: &Component,
    mut linker: Linker<HostState>,
) -> Result<(RunOutcome, HostState)> {
    let runtime_bytes = locate_runtime()?;
    let runtime = Component::new(engine, &runtime_bytes)
        .map_err(|e| anyhow!("value-heap runtime component invalid: {e}"))?;

    let mut store = Store::new(engine, HostState::default());

    // Instantiate the runtime; it exports the heap interface.
    let rt_linker: Linker<HostState> = Linker::new(engine);
    let rt_instance = rt_linker
        .instantiate(&mut store, &runtime)
        .map_err(|e| anyhow!("instantiate runtime: {e}"))?;
    let heap_idx = rt_instance
        .get_export_index(&mut store, None, RUNTIME_IFACE)
        .ok_or_else(|| anyhow!("runtime does not export {RUNTIME_IFACE}"))?;

    // Forward each heap function from the runtime instance into the program's import.
    {
        let mut iface = linker
            .instance(RUNTIME_IFACE)
            .map_err(|e| anyhow!("linker instance {RUNTIME_IFACE}: {e}"))?;
        for &fname in RUNTIME_FUNCS {
            let fidx = rt_instance
                .get_export_index(&mut store, Some(&heap_idx), fname)
                .ok_or_else(|| anyhow!("runtime missing `{fname}`"))?;
            let f = rt_instance
                .get_func(&mut store, &fidx)
                .ok_or_else(|| anyhow!("runtime export `{fname}` is not a func"))?;
            iface.func_new(fname, move |mut ctx, params, results| {
                f.call(&mut ctx, params, results)?;
                f.post_return(&mut ctx)?;
                Ok(())
            })?;
        }
    }

    // Resolve the entry by signature (the sole nullary function export) — same name-agnostic rule as
    // the scalar path; the compiler names exports verbatim.
    let entry_name = nullary_func_export(engine, program);
    let instance = linker
        .instantiate(&mut store, program)
        .map_err(|e| anyhow!("instantiate program (composed): {e}"))?;
    let run = entry_name
        .as_deref()
        .and_then(|n| instance.get_func(&mut store, n))
        .ok_or_else(|| anyhow!("composed program exports no nullary entry function"))?;

    let mut results = vec![Val::Bool(false)];
    match run.call(&mut store, &[], &mut results) {
        Ok(()) => {
            // `run` returns the value's canonical text, already assembled by the program's
            // type-directed renderer. Take it VERBATIM — the string IS the observable form
            // `(tuple 3 1)`, not a String value to be re-quoted.
            let rendered = match &results[0] {
                Val::String(s) => s.to_string(),
                other => render_val(other),
            };
            let _ = run.post_return(&mut store);
            // LEAK ORACLE: if the runtime exports `live-objects` (only the `debug-counters` build
            // does), read the live heap-object count AFTER the run to verify the program's Perceus
            // dup/drop discipline reclaimed everything (== 0). Absent on the default runtime, so this
            // is a no-op there; the leak-check harness composes the debug runtime and asserts 0.
            let live = read_live_objects(&mut store, &rt_instance, &heap_idx);
            store.data_mut().live_after_run = live;
            let state = std::mem::take(store.data_mut());
            Ok((RunOutcome::Value(rendered), state))
        }
        Err(e) => {
            let state = std::mem::take(store.data_mut());
            Ok((RunOutcome::Trap(format!("{e}")), state))
        }
    }
}

/// Invoke a compound result exported as the `cadenza:run/run` resource `value` owning
/// `display() -> string`: call `make` to get the resource handle, then the resource's own
/// `[method]value.display` to obtain its canonical text. The host never inspects the value's
/// structure — it holds only an opaque handle and reads back the string.
fn run_resource_display(
    store: &mut Store<HostState>,
    instance: &wasmtime::component::Instance,
) -> Result<(RunOutcome, HostState)> {
    let iface = instance
        .get_export_index(&mut *store, None, "cadenza:run/run")
        .ok_or_else(|| anyhow!("component exports neither `run` nor `cadenza:run/run`"))?;
    let make_idx = instance
        .get_export_index(&mut *store, Some(&iface), "make")
        .ok_or_else(|| anyhow!("resource interface has no `make`"))?;
    let disp_idx = instance
        .get_export_index(&mut *store, Some(&iface), "[method]value.display")
        .ok_or_else(|| anyhow!("resource interface has no `[method]value.display`"))?;
    let make = instance.get_func(&mut *store, &make_idx).ok_or_else(|| anyhow!("make not a func"))?;
    let display = instance.get_func(&mut *store, &disp_idx).ok_or_else(|| anyhow!("display not a func"))?;

    // make() -> value (a resource handle).
    let mut made = vec![Val::Bool(false)];
    if let Err(e) = make.call(&mut *store, &[], &mut made) {
        let state = std::mem::take(store.data_mut());
        return Ok((RunOutcome::Trap(format!("{e}")), state));
    }
    let _ = make.post_return(&mut *store);

    // value.display() -> string.
    let mut shown = vec![Val::Bool(false)];
    if let Err(e) = display.call(&mut *store, &made, &mut shown) {
        let state = std::mem::take(store.data_mut());
        return Ok((RunOutcome::Trap(format!("{e}")), state));
    }
    let _ = display.post_return(&mut *store);

    let text = match &shown[0] {
        Val::String(s) => s.clone(),
        other => return Err(anyhow!("display returned {other:?}, expected string")),
    };
    let state = std::mem::take(store.data_mut());
    Ok((RunOutcome::Value(text), state))
}

/// Run a component whose entry is a byte-transform — `func(list<u8>) -> list<u8>` — the ABI
/// a *compiled compiler* exports (its `compile : Bytes -> Bytes` seam). This is how the
/// self-hosting harness invokes the compiled compiler AS a compiler: feed it a program's
/// binary AST bytes and read back the component bytes it emits, so those can be compared
/// byte-for-byte against the host-interpreted compiler's output (self-hosting-and-bootstrap.md
/// §"A Derived Component Agrees With The Oracle").
///
/// The entry is sought by name — `compile` first, then `run` — and MUST have exactly the
/// `list<u8> -> list<u8>` shape; any other arity/type is reported as an ABI mismatch rather
/// than coerced, so the harness never mistakes a nullary-`run` component for a compiler. This
/// binds NO imports: a compiler is a pure byte transform (its manifest is empty).
/// The outcome of invoking a compiler component's `compile` export: the component bytes it
/// emitted (Ok), or the diagnostics it rejected/declined with (Err), each `(code, message)`.
/// Mirrors the WIT `result<list<u8>, list<diagnostic>>` the compiler world exports.
pub enum CompileOutcome {
    Ok(Vec<u8>),
    Diagnostics(Vec<(String, String)>),
}

/// Is `component_bytes` a DECLINE STUB — a component whose entry core function's body is a bare
/// `unreachable` (a defined trap with no computed result)? A Cadenza-authored compiler's only
/// failure channel today is `KError → unreachable` (it has no diagnostics ABI yet), so a program it
/// does not yet handle compiles to a VALID component that TRAPS when run — structurally identical for
/// every unhandled program (the `run:()->i64` body is just `unreachable; end`). `component-check`
/// byte-compares that stub against native's real output and would call it a `disagree`; this lets it
/// classify such a case as a `decline` instead, so the `disagree` count means real MISCOMPILES only
/// (the decline-vs-result discriminator the value harness already has, now in the byte gate).
///
/// Byte-level, no wasm parser: locate the embedded core module (core magic `00 61 73 6d 01 00 00 00`),
/// find its code section (id `10`), read the first function body, skip its local-declaration vector,
/// and check the FIRST instruction is `unreachable` (`0x00`). A real compiled `main` starts with a
/// computational op (`i64.const`, `local.get`, `call`, …), never `unreachable`.
pub fn is_decline_stub(component_bytes: &[u8]) -> bool {
    const CORE_MAGIC: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    // The embedded core module begins at the core magic (a component preamble precedes it; a
    // compiler-emitted program has exactly one embedded module — the entry's).
    let core_start = match component_bytes.windows(CORE_MAGIC.len()).position(|w| w == CORE_MAGIC) {
        Some(p) => p,
        None => return false,
    };
    let m = &component_bytes[core_start..];
    let mut i = CORE_MAGIC.len();
    // Walk the core module's sections: each is `<id:u8> <size:uleb> <contents>`. Find id 10 (code).
    while i < m.len() {
        let id = m[i];
        i += 1;
        let (size, adv) = match read_uleb(&m[i..]) {
            Some(x) => x,
            None => return false,
        };
        i += adv;
        let body = match m.get(i..i + size as usize) {
            Some(b) => b,
            None => return false,
        };
        if id == 10 {
            // Code section: `<count:uleb> <func>*`, `func = <body-size:uleb> <locals-vec> <code>`.
            let mut j = 0;
            let (_count, a) = match read_uleb(&body[j..]) { Some(x) => x, None => return false };
            j += a;
            // First function body.
            let (_fsize, a) = match read_uleb(&body[j..]) { Some(x) => x, None => return false };
            j += a;
            // Skip the local-declaration vector: `<n-groups:uleb> (<count:uleb> <valtype:u8>)*`.
            let (groups, a) = match read_uleb(&body[j..]) { Some(x) => x, None => return false };
            j += a;
            for _ in 0..groups {
                let (_gc, a) = match read_uleb(&body[j..]) { Some(x) => x, None => return false };
                j += a + 1; // + the valtype byte
            }
            // The first instruction: `unreachable` (0x00) ⇒ a decline stub.
            return body.get(j) == Some(&0x00);
        }
        i += size as usize;
    }
    false
}

/// Read a LEB128 unsigned int from the front of `b`; returns `(value, bytes-consumed)`.
fn read_uleb(b: &[u8]) -> Option<(u64, usize)> {
    let (mut val, mut shift, mut n) = (0u64, 0u32, 0usize);
    loop {
        let byte = *b.get(n)?;
        val |= u64::from(byte & 0x7f) << shift;
        n += 1;
        if byte & 0x80 == 0 {
            return Some((val, n));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Invoke a compiler component's `compile : list<u8> -> result<list<u8>, list<diagnostic>>`
/// export over the program's binary AST. This is how the host runs cdz-rustc-as-a-component
/// (and, later, the Cadenza-authored compiler's component) — the SAME entry point, so the two
/// are interchangeable. The export is found by its interface-qualified name
/// (`cadenza:compiler/compile`); the component binds NO imports (a compiler is a pure byte
/// transform with an empty manifest).
pub fn run_compiler_component(component_bytes: &[u8], input: &[u8]) -> Result<CompileOutcome, String> {
    let engine = Engine::default();
    let component = Component::new(&engine, component_bytes)
        .map_err(|e| format!("instantiate: component invalid: {e}"))?;
    let mut store = Store::new(&engine, HostState::default());
    let mut linker: Linker<HostState> = Linker::new(&engine);

    // A Cadenza-authored compiler PRODUCES runtime compounds, so it imports the value-heap runtime
    // `cadenza:runtime/heap` (its `compile` marshals bytes through `bytes-*` handles). COMPOSE the
    // runtime the same way the `run` path does — forward each heap function from a runtime instance
    // into this component's import — so the harness can drive a real compiler, not only an
    // import-free scalar one. A component with no runtime import (the current scalar-only cdz-rustc
    // build) instantiates directly.
    if component_imports_runtime(&engine, &component) {
        let runtime_bytes = locate_runtime().map_err(|e| e.to_string())?;
        let runtime = Component::new(&engine, &runtime_bytes)
            .map_err(|e| format!("value-heap runtime component invalid: {e}"))?;
        let rt_linker: Linker<HostState> = Linker::new(&engine);
        let rt_instance = rt_linker
            .instantiate(&mut store, &runtime)
            .map_err(|e| format!("instantiate runtime: {e}"))?;
        let heap_idx = rt_instance
            .get_export_index(&mut store, None, RUNTIME_IFACE)
            .ok_or_else(|| format!("runtime does not export {RUNTIME_IFACE}"))?;
        let mut iface = linker
            .instance(RUNTIME_IFACE)
            .map_err(|e| format!("linker instance {RUNTIME_IFACE}: {e}"))?;
        for &fname in RUNTIME_FUNCS {
            let fidx = rt_instance
                .get_export_index(&mut store, Some(&heap_idx), fname)
                .ok_or_else(|| format!("runtime missing `{fname}`"))?;
            let f = rt_instance
                .get_func(&mut store, &fidx)
                .ok_or_else(|| format!("runtime export `{fname}` is not a func"))?;
            iface
                .func_new(fname, move |mut ctx, params, results| {
                    f.call(&mut ctx, params, results)?;
                    f.post_return(&mut ctx)?;
                    Ok(())
                })
                .map_err(|e| format!("forward `{fname}`: {e}"))?;
        }
    }

    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| format!("instantiate: {e}"))?;

    // Find the `compile` func: try the interface-qualified export first (the component-model
    // shape cargo-component produces), then a bare `compile`/`run` (older shapes).
    let entry = find_compile_export(&mut store, &instance)
        .ok_or_else(|| "component exports no `compile` function".to_string())?;

    // Build the input argument to match `compile`'s parameter type. The bytes/result ABIs take a
    // bare `list<u8>` (the AST). The kinded-artifact ABI (ask-41) takes a `list<artifact>` where the
    // AST is one artifact `{bytes: <input>, kind: "ast"}` (field order = the record's declared
    // order; wasmtime matches by position, so mirror the WIT's sorted order: bytes, kind).
    use wasmtime::component::Type;
    let params = entry.params(&store);
    let input_is_artifact_list = matches!(
        params.first().map(|(_, t)| t),
        Some(Type::List(l)) if matches!(l.ty(), Type::Record(_))
    );
    let arg = if input_is_artifact_list {
        let ast_bytes = Val::List(input.iter().map(|b| Val::U8(*b)).collect());
        let artifact = Val::Record(vec![
            ("bytes".to_string(), ast_bytes),
            ("kind".to_string(), Val::String("ast".to_string())),
        ]);
        Val::List(vec![artifact])
    } else {
        Val::List(input.iter().map(|b| Val::U8(*b)).collect())
    };
    let mut results = vec![Val::Bool(false)];
    entry
        .call(&mut store, &[arg], &mut results)
        .map_err(|e| format!("running the compiled compiler: {e}"))?;
    let _ = entry.post_return(&mut store);

    decode_compile_result(&results[0])
}

/// Locate the `compile` export, whether nested in the `cadenza:compiler/compile` interface
/// instance or exported directly. Uses the two-step index lookup: resolve the interface
/// instance's export index, then `compile` within it.
fn find_compile_export(
    store: &mut Store<HostState>,
    instance: &wasmtime::component::Instance,
) -> Option<wasmtime::component::Func> {
    // Interface-qualified: `cadenza:compiler/compile` instance → its `compile` func.
    if let Some(iface_idx) =
        instance.get_export_index(&mut *store, None, "cadenza:compiler/compile")
    {
        if let Some(func_idx) =
            instance.get_export_index(&mut *store, Some(&iface_idx), "compile")
        {
            if let Some(f) = instance.get_func(&mut *store, &func_idx) {
                return Some(f);
            }
        }
    }
    // Fall back to a top-level `compile`/`run`.
    instance
        .get_func(&mut *store, "compile")
        .or_else(|| instance.get_func(&mut *store, "run"))
}

/// Decode a `compile`-export return value into a `CompileOutcome`. Three shapes are accepted, in
/// order of preference:
///  1. `compile-output` record `{ artifacts: list<artifact>, diagnostics: list<diagnostic> }` — the
///     kinded-artifact ABI (Amendment 0.8.0). Success/failure is read from the produced artifacts and
///     diagnostics, not an in-band sentinel: an error-severity diagnostic denies the component; the
///     component artifact (kind `"component"`, or the sole artifact as a fallback) carries the bytes.
///  2. `result<list<u8>, list<diagnostic>>` — the earlier two-arm ABI.
///  3. a bare `list<u8>` — a compiler that returns component bytes directly.
fn decode_compile_result(val: &Val) -> Result<CompileOutcome, String> {
    // (1) The kinded-artifact record: `{ artifacts, diagnostics }`.
    if let Val::Record(fields) = val {
        let field = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);
        if let (Some(Val::List(arts)), Some(Val::List(diags))) =
            (field("artifacts"), field("diagnostics"))
        {
            return decode_compile_output(arts, diags);
        }
    }
    match val {
        Val::Result(Ok(Some(inner))) => match inner.as_ref() {
            Val::List(items) => {
                let bytes: Result<Vec<u8>, String> = items
                    .iter()
                    .map(|v| match v {
                        Val::U8(b) => Ok(*b),
                        other => Err(format!("non-u8 in component bytes: {other:?}")),
                    })
                    .collect();
                Ok(CompileOutcome::Ok(bytes?))
            }
            other => Err(format!("ok arm is not list<u8>: {other:?}")),
        },
        Val::Result(Err(Some(inner))) => match inner.as_ref() {
            Val::List(diags) => {
                let out = diags.iter().filter_map(decode_diagnostic).collect();
                Ok(CompileOutcome::Diagnostics(out))
            }
            other => Err(format!("err arm is not list<diagnostic>: {other:?}")),
        },
        // A bare list<u8> (a compiler that returns bytes directly, no result wrapper).
        Val::List(items) => {
            let bytes: Result<Vec<u8>, String> = items
                .iter()
                .map(|v| match v {
                    Val::U8(b) => Ok(*b),
                    other => Err(format!("non-u8 in component bytes: {other:?}")),
                })
                .collect();
            Ok(CompileOutcome::Ok(bytes?))
        }
        other => Err(format!("compile returned {other:?}, expected result<list<u8>, _>")),
    }
}

/// Decode a `compile-output` record into a `CompileOutcome`. Per the build-tool-interface contract,
/// a successful derivation is signalled by a component artifact present in the output together with
/// NO error-severity diagnostic; a failure by the absence of a component artifact together with at
/// least one error-severity diagnostic. So: if any diagnostic is error-severity, report
/// `Diagnostics`; otherwise the component artifact's bytes are the `Ok` output (warnings, being
/// non-error, ride alongside a produced component and do not deny it).
fn decode_compile_output(arts: &[Val], diags: &[Val]) -> Result<CompileOutcome, String> {
    let decoded_diags: Vec<(String, String)> = diags.iter().filter_map(decode_diagnostic).collect();
    let has_error = diags.iter().any(diagnostic_is_error);
    if has_error {
        return Ok(CompileOutcome::Diagnostics(decoded_diags));
    }
    // The component artifact: kind `"component"` if labelled, else the sole artifact.
    let component = artifact_bytes(arts, "component")
        .or_else(|| if arts.len() == 1 { artifact_bytes(arts, "") } else { None });
    match component {
        Some(bytes) => Ok(CompileOutcome::Ok(bytes)),
        None => Ok(CompileOutcome::Diagnostics(decoded_diags)),
    }
}

/// The bytes of the first `artifact` record whose `kind` field equals `want` (or, when `want` is
/// empty, the first artifact regardless of kind — the single-artifact fallback).
fn artifact_bytes(arts: &[Val], want: &str) -> Option<Vec<u8>> {
    for a in arts {
        if let Val::Record(fields) = a {
            let kind = fields.iter().find(|(n, _)| n == "kind").and_then(|(_, v)| match v {
                Val::String(s) => Some(s.as_str()),
                _ => None,
            });
            if want.is_empty() || kind == Some(want) {
                if let Some((_, Val::List(items))) = fields.iter().find(|(n, _)| n == "bytes") {
                    let bytes: Option<Vec<u8>> = items
                        .iter()
                        .map(|v| match v {
                            Val::U8(b) => Some(*b),
                            _ => None,
                        })
                        .collect();
                    if let Some(b) = bytes {
                        return Some(b);
                    }
                }
            }
        }
    }
    None
}

/// Is this `diagnostic` record error-severity? The `severity` field is an enum `{error, warning}`;
/// a missing/unrecognized severity is treated as an error (fail closed).
fn diagnostic_is_error(val: &Val) -> bool {
    if let Val::Record(fields) = val {
        match fields.iter().find(|(n, _)| n == "severity").map(|(_, v)| v) {
            Some(Val::Enum(s)) => s == "error",
            Some(Val::String(s)) => s == "error",
            None => true, // no severity field: the two-field diagnostic — treat as an error
            _ => true,
        }
    } else {
        false
    }
}

/// Decode a `diagnostic` record `{code: string, message: string}` (a `severity` field, if present,
/// is not part of the host's `(code, message)` outcome tuple).
fn decode_diagnostic(val: &Val) -> Option<(String, String)> {
    if let Val::Record(fields) = val {
        let get = |k: &str| {
            fields.iter().find(|(n, _)| n == k).and_then(|(_, v)| match v {
                Val::String(s) => Some(s.clone()),
                _ => None,
            })
        };
        Some((get("code").unwrap_or_default(), get("message").unwrap_or_default()))
    } else {
        None
    }
}

/// Read the runtime's `live-objects` debug export (heap-object count), if present. Returns `None`
/// when the runtime does not export it (the default, zero-cost build) — so this is a no-op there.
/// Only the `debug-counters` runtime build carries a real counter; the leak-check harness composes
/// that build and asserts the result is `Some(0)` after a run, proving the compiler's Perceus dup/drop
/// discipline leaves no live objects.
fn read_live_objects(
    store: &mut Store<HostState>,
    rt_instance: &wasmtime::component::Instance,
    heap_idx: &wasmtime::component::ComponentExportIndex,
) -> Option<u32> {
    let idx = rt_instance.get_export_index(&mut *store, Some(heap_idx), "live-objects")?;
    let f = rt_instance.get_func(&mut *store, &idx)?;
    let mut out = vec![Val::U32(0)];
    f.call(&mut *store, &[], &mut out).ok()?;
    let _ = f.post_return(&mut *store);
    match out.first() {
        Some(Val::U32(n)) => Some(*n),
        _ => None,
    }
}

/// Render a component-model value to the corpus-comparable string form.
fn render_val(v: &Val) -> String {
    match v {
        Val::S64(i) => i.to_string(),
        Val::U64(i) => i.to_string(),
        Val::S32(i) => i.to_string(),
        Val::U32(i) => i.to_string(),
        Val::Bool(b) => b.to_string(),
        // Closed-escape-set render (NOT `{:?}`), so a non-printable scalar renders VERBATIM and reads
        // back to the same value — the same renderer the compiled component and the const path use
        // (collections-and-text.md §A String Literal's Escapes Are A Closed Set; the round-trip fix).
        Val::String(s) => cdz_compiler::codegen::string_canonical_text(s),
        Val::Float64(f) => display_float(*f),
        other => format!("{other:?}"),
    }
}

/// Canonical float rendering, matching the corpus value form: `-0.0`, `NaN`, and integral
/// floats as `N.0`. Kept here so the host has no dependency on any interpreter module.
pub fn display_float(f: f64) -> String {
    if f == 0.0 && f.is_sign_negative() {
        "-0.0".into()
    } else if f.is_nan() {
        "NaN".into()
    } else if f.fract() == 0.0 && f.is_finite() {
        // `{:.0}` prints the exact integer value of the whole float, injectively — unlike
        // `f as i64`, which saturates at i64::MAX so every whole float ≥ 2^63 collapsed to one
        // string (violating deterministic-value-form.md injectivity). Kept in lock-step with
        // codegen::display_float_text.
        format!("{f:.0}.0")
    } else {
        format!("{}", f)
    }
}
