//! Envelope code generation — the runtime WIT is the source of truth.
//!
//! The compiler emits WebAssembly components by wrapping a per-program core module in a FIXED
//! component-model envelope. The envelope's bytes (`RT_HEAD`/`RT_TAIL`), the import section it puts
//! inside the compiler-built core module (`RT_IMPORT_CONTENT`), the heap-import core signatures
//! (`rt_import_types`), the import indices (`mod himport`), and the host's forwarding list
//! (`RUNTIME_FUNCS`) all encode ONE thing: the value-heap runtime interface. That interface's source
//! of truth is the runtime's own `wit/runtime.wit`. This module reads that WIT, takes the compiler's
//! ordered allow-list of the functions it lowers, and DERIVES every one of those artifacts — building
//! the reference component with `wasm-encoder`, self-validating it with `wasmparser`, and splitting it
//! at the embedded core-module boundary (a Rust port of the throwaway `/tmp` splitter that used to do
//! this by hand). The output is ordinary Rust source the compiler `mod`-includes.
//!
//! `wasm-encoder`/`wit-parser`/`wasmparser` live only HERE (xtask), never in the shipped compiler —
//! a dev-desk oracle, exactly as the RT_HEAD comment has always claimed. Adding a runtime op the
//! compiler wants is now: add its name to `HEAP_ALLOWLIST`, run `xtask build`, re-verify the gates.

use std::path::Path;
use wasm_encoder::{
    CanonicalOption, ComponentBuilder, ComponentExportKind, ComponentTypeRef, ComponentValType,
    EntityType, ExportKind, InstanceType, PrimitiveValType,
};
use wit_parser::{Resolve, Type as WitType};

/// The heap-interface functions the compiler lowers into every emitted component, IN ORDER.
///
/// This is the ONE compiler-specific input to generation: the runtime WIT declares the full
/// contract, and this list selects the ordered subset the compiler's envelope actually imports and
/// lowers. The order here IS `himport`'s index assignment and `RT_IMPORT_CONTENT`'s order; the
/// runtime resolves imports BY NAME, so this order may (and does) diverge from the WIT's own order.
///
/// It skips two classes of WIT functions the envelope cannot / does not yet lower:
///   * `str-new`/`str-get` — `string`-typed, needing a heavier canon than the envelope provides.
///   * `reset`/`arr-alloc-reuse`/`sum-new-reuse` — the Perceus reuse ops, not yet emitted (Phase D).
/// Adding one is a one-line append here (plus wiring its codegen); `build_heap_envelope` errors if a
/// name is absent from the WIT or is `string`-typed (unlowerable), so the allow-list cannot drift
/// from the contract silently.
pub const HEAP_ALLOWLIST: &[&str] = &[
    "box-int", "get-int", "box-bool", "get-bool", "box-float", "get-float",
    "arr-alloc", "arr-set", "arr-get", "arr-len",
    "sum-new", "sum-disc", "sum-payload",
    "bytes-alloc", "bytes-set", "bytes-get", "bytes-len",
    "dup", "drop",
    "map-alloc", "map-set", "map-key", "map-val", "map-len",
    "vec-empty", "vec-len", "vec-get", "vec-push", "vec-update",
    "bytes-concat", "bytes-slice", "bytes-compact",
    // CHAMP persistent map (WIT 37–45): the REAL key→value map the `Map.*` surface lowers to
    // (the `map-alloc`/`map-set`/… above are the vestigial positional stub). Appended LAST so every
    // existing himport index is frozen (this list's ORDER is the index assignment; a name-resolving
    // runtime tolerates the divergence from WIT order). The 5 core ops + the 4-op cursor: the cursor
    // drives the type-directed renderer's map walk (`map-iter`/`-next`/`-key`/`-val`).
    "map-empty", "map-insert", "map-lookup", "map-remove", "map-size",
    "map-iter", "map-iter-next", "map-iter-key", "map-iter-val",
    // `vec-concat` (WIT 55): O(log N) list concatenation over the RRB trie, `List.concat`'s lowering.
    // Appended LAST so every himport index above stays frozen. `vec-split` (WIT 56) returns a
    // `tuple<u32,u32>` — a component multi-return the envelope's canon cannot lower yet — so it is
    // NOT allow-listed; `List.concat` needs only the single-handle `vec-concat`.
    "vec-concat",
    // CHAMP persistent SET (WIT 46–53): the `Set.*` surface's lowering — a canonical unordered
    // collection of handle elements (not a Map<E,Unit>). Appended LAST so every himport index above
    // stays frozen. All take/return `u32`/`bool` (lowerable). The 6 core ops (`of` builds via repeated
    // `insert` from `empty`), the 3 algebra ops (union/intersection/difference), + the 2-op cursor the
    // renderer walks (`set-iter`/`-next`/`-elem`).
    "set-empty", "set-insert", "set-contains", "set-remove", "set-size",
    "set-iter", "set-iter-next", "set-iter-elem",
    "set-union", "set-intersection", "set-difference",
];

/// The interface name the runtime exports (also the WIT world's import name the program uses).
const HEAP_INTERFACE: &str = "cadenza:runtime/heap";

/// A logical valtype, resolved from a WIT type. Every heap handle is a `u32` (component) / `i32`
/// (core); the scalar leaves carry `s64`/`bool`/`f64`. This is the whole type universe the heap
/// interface uses — richer WIT types (records, lists, strings) are rejected as unlowerable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LogTy {
    U32,
    S64,
    Bool,
    F64,
}

impl LogTy {
    /// From a WIT primitive; `None` for a type the envelope's canon cannot lower (e.g. `string`).
    fn from_wit(t: WitType) -> Option<LogTy> {
        match t {
            WitType::U32 => Some(LogTy::U32),
            WitType::S64 => Some(LogTy::S64),
            WitType::Bool => Some(LogTy::Bool),
            WitType::F64 => Some(LogTy::F64),
            _ => None,
        }
    }

    /// The component-model primitive valtype (for the import instance-type).
    fn comp(self) -> PrimitiveValType {
        match self {
            LogTy::U32 => PrimitiveValType::U32,
            LogTy::S64 => PrimitiveValType::S64,
            LogTy::Bool => PrimitiveValType::Bool,
            LogTy::F64 => PrimitiveValType::F64,
        }
    }

    /// The core wasm valtype byte a lowered handle/scalar carries (matches `Kind::core_valtype`).
    fn core_byte(self) -> u8 {
        match self {
            LogTy::U32 | LogTy::Bool => 0x7F, // i32
            LogTy::S64 => 0x7E,               // i64
            LogTy::F64 => 0x7C,               // f64
        }
    }

    /// The `wasm_encoder::ValType` for the core stub module's import signatures.
    fn core_valtype(self) -> wasm_encoder::ValType {
        match self {
            LogTy::U32 | LogTy::Bool => wasm_encoder::ValType::I32,
            LogTy::S64 => wasm_encoder::ValType::I64,
            LogTy::F64 => wasm_encoder::ValType::F64,
        }
    }
}

/// One heap function resolved from the WIT: its name plus the logical param/result types.
struct HeapFn {
    name: String,
    params: Vec<LogTy>,
    result: Option<LogTy>,
}

/// Resolve the compiler's allow-list against the runtime WIT, in allow-list order. Errors if a name
/// is missing from the WIT's `heap` interface or carries a type the envelope cannot lower.
fn resolve_iface(wit_path: &Path) -> Result<Vec<HeapFn>, String> {
    let mut resolve = Resolve::default();
    let pkg = resolve
        .push_file(wit_path)
        .map_err(|e| format!("parse {}: {e}", wit_path.display()))?;
    // Find the `heap` interface in the pushed package.
    let iface_id = resolve.packages[pkg]
        .interfaces
        .iter()
        .find(|(name, _)| name.as_str() == "heap")
        .map(|(_, id)| *id)
        .ok_or_else(|| "runtime WIT has no `heap` interface".to_string())?;
    let iface = &resolve.interfaces[iface_id];

    let mut out = Vec::with_capacity(HEAP_ALLOWLIST.len());
    for &want in HEAP_ALLOWLIST {
        let f = iface
            .functions
            .get(want)
            .ok_or_else(|| format!("allow-list names `{want}`, absent from the runtime `heap` interface"))?;
        let mut params = Vec::with_capacity(f.params.len());
        for (pname, ty) in &f.params {
            params.push(LogTy::from_wit(*ty).ok_or_else(|| {
                format!("`{want}` param `{pname}` has a type the envelope cannot lower (e.g. string)")
            })?);
        }
        let result = match f.result {
            Some(ty) => Some(LogTy::from_wit(ty).ok_or_else(|| {
                format!("`{want}` result has a type the envelope cannot lower (e.g. string)")
            })?),
            None => None,
        };
        out.push(HeapFn { name: want.to_string(), params, result });
    }
    Ok(out)
}

// ─── The reference heap component (wasm-encoder) ─────────────────────────────────────────────────

/// Build the reference heap-envelope component: imports `cadenza:runtime/heap` (an instance whose
/// exports are the allow-list functions), canon-lowers each, embeds a STUB core module (imports the
/// lowered funcs at their core signatures, plus a stub `run: ()->i32` and `cabi_realloc: (i32×4)->i32`
/// so it type-checks), instantiates it threading the lowered funcs in by name, aliases
/// `run`/`memory`/`cabi_realloc`, and lifts+exports `run: () -> string`. This is the exact shape the
/// hand-authored reference WAT had; wasm-encoder + wasmparser make it derived and self-validated
/// instead of pasted. Returns the whole component's bytes (later split into HEAD/TAIL).
fn build_heap_reference(iface: &[HeapFn]) -> Vec<u8> {
    let n = iface.len() as u32;
    let mut c = ComponentBuilder::default();

    // (1) The import instance-type: one func type + export per heap function, in order.
    let mut it = InstanceType::new();
    for (i, f) in iface.iter().enumerate() {
        let params: Vec<(&str, ComponentValType)> = f
            .params
            .iter()
            .enumerate()
            .map(|(j, p)| (param_name(&f.name, j), ComponentValType::Primitive(p.comp())))
            .collect();
        {
            // The func-type encoder holds a mutable borrow of `it`; scope it so `export` (below)
            // can borrow `it` again. `params`/`result` must both be called before it drops.
            let mut ft = it.ty().function();
            ft.params(params.iter().map(|(n, t)| (*n, *t)));
            ft.result(f.result.map(|r| ComponentValType::Primitive(r.comp())));
        }
        it.export(&f.name, ComponentTypeRef::Func(i as u32));
    }
    c.type_instance(&it); // component type 0

    // (2) Import the heap instance of that type.
    c.import(HEAP_INTERFACE, ComponentTypeRef::Instance(0)); // instance 0

    // (3) Alias each export out of the imported instance + canon-lower it → core funcs 0..n.
    for f in iface {
        let comp_fn = c.alias_export(0, &f.name, ComponentExportKind::Func);
        c.lower_func(comp_fn, []); // core func i
    }

    // (4) The stub core module: imports the heap funcs at their CORE signatures, plus stub
    //     `run: ()->i32` (core func n) and `cabi_realloc: (i32×4)->i32` (core func n+1).
    let core = build_stub_core_module(iface);
    let module_idx = c.core_module_raw(&core); // core module 0

    // (5) Instantiate it, threading the lowered heap funcs (core funcs 0..n) in as an instance
    //     named "heap" (matching the stub's import module name).
    let heap_args: Vec<(&str, ExportKind, u32)> =
        iface.iter().enumerate().map(|(i, f)| (f.name.as_str(), ExportKind::Func, i as u32)).collect();
    let heap_inst = c.core_instantiate_exports(heap_args); // core instance 0
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);

    // (6) Alias `run`/`memory`/`cabi_realloc` out of the instantiated program.
    let run_core = c.core_alias_export(prog_inst, "run", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);

    // (7) Lift `run` as `() -> string` with utf8 + the aliased memory/realloc, and export it.
    let (run_ty, mut enc) = c.type_function();
    enc.params::<[(&str, ComponentValType); 0], _>([]).result(Some(ComponentValType::Primitive(
        PrimitiveValType::String,
    )));
    let run_comp = c.lift_func(
        run_core,
        run_ty,
        [
            CanonicalOption::UTF8,
            CanonicalOption::Memory(mem),
            CanonicalOption::Realloc(realloc),
        ],
    );
    c.export("run", ComponentExportKind::Func, run_comp, None);

    let _ = n;
    let bytes = c.finish();
    validate_component(&bytes, "heap reference");
    bytes
}

/// Build the reference `compile`-envelope component: the SAME heap-import + core-module surround as
/// `build_heap_reference`, but the entry is lifted+exported as `compile: func(list<u8>) -> list<u8>`
/// (the `cadenza:compiler/compile` seam — bootstrap.md §"The Compiler Is Authored In Cadenza") instead
/// of the nullary `run: () -> string`. A `bytes → bytes` entry (a Cadenza-authored compiler, or any
/// `(def (compile b) …)` over runtime Bytes) is emitted through this envelope so the host's
/// `run_compiler_component`/`component-check` harness can drive it over the whole corpus. The embedded
/// core module is a stub whose `compile: (i32 ptr, i32 len) -> i32 retptr` has the canonical-ABI shape
/// a `list<u8> -> list<u8>` lift expects; the compiler substitutes its real core module between the
/// generated HEAD and TAIL. Split into COMPILE_HEAD / COMPILE_TAIL.
fn build_compile_reference(iface: &[HeapFn]) -> Vec<u8> {
    let mut c = ComponentBuilder::default();

    // (1) The heap import instance-type — identical to the heap reference (same interface, same order).
    let mut it = InstanceType::new();
    for (i, f) in iface.iter().enumerate() {
        let params: Vec<(&str, ComponentValType)> = f
            .params
            .iter()
            .enumerate()
            .map(|(j, p)| (param_name(&f.name, j), ComponentValType::Primitive(p.comp())))
            .collect();
        {
            let mut ft = it.ty().function();
            ft.params(params.iter().map(|(n, t)| (*n, *t)));
            ft.result(f.result.map(|r| ComponentValType::Primitive(r.comp())));
        }
        it.export(&f.name, ComponentTypeRef::Func(i as u32));
    }
    c.type_instance(&it); // component type 0
    c.import(HEAP_INTERFACE, ComponentTypeRef::Instance(0)); // instance 0

    // (2) Alias + canon-lower each heap export → core funcs 0..n.
    for f in iface {
        let comp_fn = c.alias_export(0, &f.name, ComponentExportKind::Func);
        c.lower_func(comp_fn, []); // core func i
    }

    // (3) The stub core module: heap imports at their core signatures + `compile: (i32,i32)->i32`
    //     (the canonical-ABI `list<u8> -> list<u8>` core shape: (ptr, len) -> retptr) + `cabi_realloc`,
    //     exporting `memory`/`compile`/`cabi_realloc`.
    let core = build_compile_stub_core_module(iface);
    let module_idx = c.core_module_raw(&core); // core module 0

    // (4) Instantiate, threading the lowered heap funcs in as an instance named "heap".
    let heap_args: Vec<(&str, ExportKind, u32)> =
        iface.iter().enumerate().map(|(i, f)| (f.name.as_str(), ExportKind::Func, i as u32)).collect();
    let heap_inst = c.core_instantiate_exports(heap_args); // core instance 0
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);

    // (5) Alias `compile`/`memory`/`cabi_realloc` out of the instantiated program.
    let compile_core = c.core_alias_export(prog_inst, "compile", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);

    // (6) Lift `compile` as `func(list<u8>) -> list<u8>` with memory + realloc, and export it.
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (compile_ty, mut enc) = c.type_function();
    enc.params([("ast", ComponentValType::Type(list_u8))])
        .result(Some(ComponentValType::Type(list_u8)));
    let compile_comp = c.lift_func(
        compile_core,
        compile_ty,
        [CanonicalOption::Memory(mem), CanonicalOption::Realloc(realloc)],
    );
    c.export("compile", ComponentExportKind::Func, compile_comp, None);

    let bytes = c.finish();
    validate_component(&bytes, "compile reference");
    bytes
}

/// Build the reference `compile`-envelope component whose entry is lifted as
/// `compile: func(list<u8>) -> result<list<u8>, list<diagnostic>>` — the DIAGNOSTICS ABI
/// (ask-30 sub-gap 2, compiler-pipeline.md / build-tool-interface.md). Same heap-import + core-module
/// surround as `build_compile_reference`, but the result type distinguishes a well-typed program that
/// produced a component (`Ok(list<u8>)`) from an ill-typed one refused with machine-readable
/// diagnostics (`Err(list<diagnostic>)`), rather than an opaque byte sequence or a trap. The embedded
/// stub core module's `compile: (i32 ptr, i32 len) -> i32 retptr` has the canonical-ABI shape the
/// non-flat `result<…>` return lowers to (a retptr into the result's `[discriminant, payload…]`
/// layout); the compiler substitutes its real core module between the generated HEAD and TAIL. Split
/// into COMPILE_RESULT_HEAD / COMPILE_RESULT_TAIL.
fn build_compile_result_reference(iface: &[HeapFn]) -> Vec<u8> {
    let mut c = ComponentBuilder::default();

    // (1) Heap import instance-type (identical to the other references).
    let mut it = InstanceType::new();
    for (i, f) in iface.iter().enumerate() {
        let params: Vec<(&str, ComponentValType)> = f
            .params
            .iter()
            .enumerate()
            .map(|(j, p)| (param_name(&f.name, j), ComponentValType::Primitive(p.comp())))
            .collect();
        {
            let mut ft = it.ty().function();
            ft.params(params.iter().map(|(n, t)| (*n, *t)));
            ft.result(f.result.map(|r| ComponentValType::Primitive(r.comp())));
        }
        it.export(&f.name, ComponentTypeRef::Func(i as u32));
    }
    c.type_instance(&it);
    c.import(HEAP_INTERFACE, ComponentTypeRef::Instance(0));

    // (2) Alias + canon-lower each heap export → core funcs 0..n.
    for f in iface {
        let comp_fn = c.alias_export(0, &f.name, ComponentExportKind::Func);
        c.lower_func(comp_fn, []);
    }

    // (3) Stub core module: `compile: (i32,i32)->i32 retptr` (the result<…> return lowers to a
    //     retptr, same core shape as the bytes→bytes case) + `cabi_realloc`.
    let core = build_compile_stub_core_module(iface);
    let module_idx = c.core_module_raw(&core);

    // (4) Instantiate, threading the lowered heap funcs in as instance "heap".
    let heap_args: Vec<(&str, ExportKind, u32)> =
        iface.iter().enumerate().map(|(i, f)| (f.name.as_str(), ExportKind::Func, i as u32)).collect();
    let heap_inst = c.core_instantiate_exports(heap_args);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);

    // (5) Alias `compile`/`memory`/`cabi_realloc`.
    let compile_core = c.core_alias_export(prog_inst, "compile", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);

    // (6) Build the result type: `result<list<u8>, list<diagnostic>>` where
    //     `diagnostic = record { code: string, message: string }`.
    // ⚠ An exported func whose signature references a RECORD (a non-primitive named type) requires
    // that record to itself be an EXPORTED NAMED type (wasmparser's `all_valtypes_named`) — otherwise
    // "func not valid to be used as export". A `list<u8>` alone (list of a primitive) needs no name,
    // which is why the plain `compile: (list<u8>)->list<u8>` reference did not hit this. So export the
    // `diagnostic` record under a name; the result/list-of-diagnostic wrappers then reference a named
    // type and the `compile` func export validates.
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (diagnostic, ddef) = c.type_defined();
    ddef.record([
        ("code", ComponentValType::Primitive(PrimitiveValType::String)),
        ("message", ComponentValType::Primitive(PrimitiveValType::String)),
    ]);
    // Exporting the record yields a NEW type index that IS the named export; the func signature must
    // reference THAT (not the anonymous `diagnostic`) so `all_valtypes_named` sees a named type.
    let diagnostic_named = c.export("diagnostic", ComponentExportKind::Type, diagnostic, None);
    let (list_diag, lddef) = c.type_defined();
    lddef.list(ComponentValType::Type(diagnostic_named));
    let (result_ty, rdef) = c.type_defined();
    rdef.result(Some(ComponentValType::Type(list_u8)), Some(ComponentValType::Type(list_diag)));

    // (7) Lift `compile` as `func(list<u8>) -> result<list<u8>, list<diagnostic>>` and export it.
    let (compile_ty, mut enc) = c.type_function();
    enc.params([("ast", ComponentValType::Type(list_u8))])
        .result(Some(ComponentValType::Type(result_ty)));
    let compile_comp = c.lift_func(
        compile_core,
        compile_ty,
        [CanonicalOption::Memory(mem), CanonicalOption::Realloc(realloc)],
    );
    c.export("compile", ComponentExportKind::Func, compile_comp, None);

    let bytes = c.finish();
    validate_component(&bytes, "compile result reference");
    bytes
}

/// Build the KINDED-ARTIFACT reference (ask-41 / Amendment 0.8.0): `compile: func(list<artifact>) ->
/// compile-output`, where
///   `artifact       = record { kind: string, bytes: list<u8> }`
///   `diagnostic     = record { severity: enum { error, warning }, code: string, message: string }`
///   `compile-output = record { artifacts: list<artifact>, diagnostics: list<diagnostic> }`.
/// The symmetric artifacts-in / {artifacts, diagnostics}-out interface: the input is a list of kinded
/// artifacts (the AST is the `ast`/`source` artifact; a cache / imported units are more inputs) and
/// the output pairs the produced artifacts (the component is one, by kind; DWARF/manifest are peers)
/// with the diagnostics (severity distinguishes an error that denies a component from a warning that
/// rides alongside one). Both the input list and the output record are non-flat, so the core `compile`
/// stays `(i32 ptr, i32 len) -> i32 retptr` (the input list lowers to ptr+len; the output record lifts
/// via the retptr). Same HEAD/TAIL split. ⚠ Every non-primitive record referenced by the exported
/// func's signature must itself be an EXPORTED NAMED type (wasmparser `all_valtypes_named`), and each
/// wrapper must reference the index `c.export()` RETURNS, not the anonymous `type_defined` index.
fn build_compile_artifacts_reference(iface: &[HeapFn]) -> Vec<u8> {
    let mut c = ComponentBuilder::default();

    // (1) Heap import instance-type (identical to the other references).
    let mut it = InstanceType::new();
    for (i, f) in iface.iter().enumerate() {
        let params: Vec<(&str, ComponentValType)> = f
            .params
            .iter()
            .enumerate()
            .map(|(j, p)| (param_name(&f.name, j), ComponentValType::Primitive(p.comp())))
            .collect();
        {
            let mut ft = it.ty().function();
            ft.params(params.iter().map(|(n, t)| (*n, *t)));
            ft.result(f.result.map(|r| ComponentValType::Primitive(r.comp())));
        }
        it.export(&f.name, ComponentTypeRef::Func(i as u32));
    }
    c.type_instance(&it);
    c.import(HEAP_INTERFACE, ComponentTypeRef::Instance(0));

    // (2) Alias + canon-lower each heap export → core funcs 0..n.
    for f in iface {
        let comp_fn = c.alias_export(0, &f.name, ComponentExportKind::Func);
        c.lower_func(comp_fn, []);
    }

    // (3) Stub core module: `compile: (i32,i32)->i32 retptr` + `cabi_realloc`.
    let core = build_compile_stub_core_module(iface);
    let module_idx = c.core_module_raw(&core);

    // (4) Instantiate, threading the lowered heap funcs in as instance "heap".
    let heap_args: Vec<(&str, ExportKind, u32)> =
        iface.iter().enumerate().map(|(i, f)| (f.name.as_str(), ExportKind::Func, i as u32)).collect();
    let heap_inst = c.core_instantiate_exports(heap_args);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);

    // (5) Alias `compile`/`memory`/`cabi_realloc`.
    let compile_core = c.core_alias_export(prog_inst, "compile", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);

    // (6) Build the type graph, exporting each record as a named type so the signature validates.
    // ⚠ FIELD ORDER = SORTED BY KEY, matching the seed's runtime record representation (a runtime
    // record is a heap `arr` whose slots are the field VALUES sorted by field name). Declaring the
    // component record fields in that SAME sorted order makes the wrapper's marshal a straight
    // slot-i → canonical-offset-i copy — no per-record permutation between the runtime `arr` slot
    // order and the canonical-ABI record field order. (A record's field order is a free choice in the
    // type; the host decoder finds fields by NAME, so sorting is invisible to a consumer.)
    //   artifact = record { bytes: list<u8>, kind: string }   (sorted: bytes < kind)
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (artifact, adef) = c.type_defined();
    adef.record([
        ("bytes", ComponentValType::Type(list_u8)),
        ("kind", ComponentValType::Primitive(PrimitiveValType::String)),
    ]);
    let artifact_named = c.export("artifact", ComponentExportKind::Type, artifact, None);
    //   diagnostic = record { code: string, message: string, severity: enum }  (sorted: code < message < severity)
    let (severity, sdef) = c.type_defined();
    sdef.enum_type(["error", "warning"]);
    let severity_named = c.export("severity", ComponentExportKind::Type, severity, None);
    let (diagnostic, ddef) = c.type_defined();
    ddef.record([
        ("code", ComponentValType::Primitive(PrimitiveValType::String)),
        ("message", ComponentValType::Primitive(PrimitiveValType::String)),
        ("severity", ComponentValType::Type(severity_named)),
    ]);
    let diagnostic_named = c.export("diagnostic", ComponentExportKind::Type, diagnostic, None);
    //   list<artifact>, list<diagnostic>
    let (list_artifact, ladef) = c.type_defined();
    ladef.list(ComponentValType::Type(artifact_named));
    let (list_diag, lddef) = c.type_defined();
    lddef.list(ComponentValType::Type(diagnostic_named));
    //   compile-output = record { artifacts: list<artifact>, diagnostics: list<diagnostic> }
    //   (sorted: artifacts < diagnostics — already in that order)
    let (compile_output, codef) = c.type_defined();
    codef.record([
        ("artifacts", ComponentValType::Type(list_artifact)),
        ("diagnostics", ComponentValType::Type(list_diag)),
    ]);
    let compile_output_named = c.export("compile-output", ComponentExportKind::Type, compile_output, None);
    //   the input list<artifact> references the SAME exported `artifact`.
    let (in_list_artifact, iladef) = c.type_defined();
    iladef.list(ComponentValType::Type(artifact_named));

    // (7) Lift `compile` as `func(inputs: list<artifact>) -> compile-output` and export it.
    let (compile_ty, mut enc) = c.type_function();
    enc.params([("inputs", ComponentValType::Type(in_list_artifact))])
        .result(Some(ComponentValType::Type(compile_output_named)));
    let compile_comp = c.lift_func(
        compile_core,
        compile_ty,
        [CanonicalOption::Memory(mem), CanonicalOption::Realloc(realloc)],
    );
    c.export("compile", ComponentExportKind::Func, compile_comp, None);

    let bytes = c.finish();
    validate_component(&bytes, "compile artifacts reference");
    bytes
}

/// The stub core module the `compile` reference embeds. Like `build_stub_core_module` but its entry
/// is `compile: (i32 ptr, i32 len) -> i32 retptr` (the canonical-ABI core shape for `list<u8> ->
/// list<u8>`), exported as `compile`. Thrown away — only the HEAD-before / TAIL-after surround is kept.
fn build_compile_stub_core_module(iface: &[HeapFn]) -> Vec<u8> {
    use wasm_encoder::{
        CodeSection, ExportSection, Function, FunctionSection, ImportSection, Instruction,
        MemorySection, MemoryType, Module as CoreModule, TypeSection, ValType,
    };
    let n = iface.len() as u32;
    let mut m = CoreModule::new();

    // Types: one per heap func, then compile `(i32,i32)->i32` and realloc `(i32×4)->i32`.
    let mut types = TypeSection::new();
    for f in iface {
        let params: Vec<ValType> = f.params.iter().map(|p| p.core_valtype()).collect();
        let results: Vec<ValType> = f.result.map(|r| vec![r.core_valtype()]).unwrap_or_default();
        types.ty().function(params, results);
    }
    let ty_compile = n; // (i32,i32)->i32
    types.ty().function([ValType::I32, ValType::I32], [ValType::I32]);
    let ty_realloc = n + 1; // (i32×4)->i32
    types.ty().function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], [ValType::I32]);
    m.section(&types);

    // Imports: heap.<name> : type i.
    let mut imports = ImportSection::new();
    for (i, f) in iface.iter().enumerate() {
        imports.import("heap", &f.name, EntityType::Function(i as u32));
    }
    m.section(&imports);

    // Functions: compile (ty_compile), cabi_realloc (ty_realloc). Defined func indices n, n+1.
    let mut funcs = FunctionSection::new();
    funcs.function(ty_compile);
    funcs.function(ty_realloc);
    m.section(&funcs);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("compile", ExportKind::Func, n);
    exports.export("cabi_realloc", ExportKind::Func, n + 1);
    m.section(&exports);

    let mut code = CodeSection::new();
    let mut compile = Function::new([]);
    compile.instruction(&Instruction::I32Const(0));
    compile.instruction(&Instruction::End);
    code.function(&compile);
    let mut realloc = Function::new([]);
    realloc.instruction(&Instruction::I32Const(0));
    realloc.instruction(&Instruction::End);
    code.function(&realloc);
    m.section(&code);

    m.finish()
}

/// The stub core module the reference embeds. Its ONLY job is to type-check so the surrounding
/// component (instance-type, lowers, instantiation, aliases, lift) is well-formed; its function
/// bodies are throwaway stubs — the REAL program core module the compiler builds at emit time
/// replaces it byte-for-byte between HEAD and TAIL. Imports each heap func at its core signature,
/// then defines `run: ()->i32` (returns 0) and `cabi_realloc: (i32×4)->i32` (returns 0), and exports
/// `memory`/`run`/`cabi_realloc`.
fn build_stub_core_module(iface: &[HeapFn]) -> Vec<u8> {
    use wasm_encoder::{
        CodeSection, ConstExpr, ExportSection, Function, FunctionSection, ImportSection, Instruction,
        MemorySection, MemoryType, Module as CoreModule, TypeSection, ValType,
    };
    let n = iface.len() as u32;
    let mut m = CoreModule::new();

    // Types: one per distinct heap signature (we just emit one per function; dedup is not needed —
    // the reference is thrown away, only its HEAD/TAIL surround is kept). Then run `()->i32` and
    // realloc `(i32×4)->i32`.
    let mut types = TypeSection::new();
    for f in iface {
        let params: Vec<ValType> = f.params.iter().map(|p| p.core_valtype()).collect();
        let results: Vec<ValType> = f.result.map(|r| vec![r.core_valtype()]).unwrap_or_default();
        types.ty().function(params, results);
    }
    let ty_run = n; // ()->i32
    types.ty().function([], [ValType::I32]);
    let ty_realloc = n + 1; // (i32×4)->i32
    types.ty().function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], [ValType::I32]);
    m.section(&types);

    // Imports: heap.<name> : type i, for each function in order.
    let mut imports = ImportSection::new();
    for (i, f) in iface.iter().enumerate() {
        imports.import("heap", &f.name, EntityType::Function(i as u32));
        let _ = i;
    }
    m.section(&imports);

    // Functions: run (type ty_run), cabi_realloc (type ty_realloc). Defined func indices n, n+1.
    let mut funcs = FunctionSection::new();
    funcs.function(ty_run);
    funcs.function(ty_realloc);
    m.section(&funcs);

    // Memory: 1 page.
    let mut mems = MemorySection::new();
    mems.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);

    // Exports: memory, run (func n), cabi_realloc (func n+1).
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("run", ExportKind::Func, n);
    exports.export("cabi_realloc", ExportKind::Func, n + 1);
    m.section(&exports);

    // Code: run → i32.const 0 ; realloc → i32.const 0.
    let mut code = CodeSection::new();
    let mut run = Function::new([]);
    run.instruction(&Instruction::I32Const(0));
    run.instruction(&Instruction::End);
    code.function(&run);
    let mut realloc = Function::new([]);
    realloc.instruction(&Instruction::I32Const(0));
    realloc.instruction(&Instruction::End);
    code.function(&realloc);
    m.section(&code);

    let _ = ConstExpr::i32_const(0);
    m.finish()
}

/// A per-parameter name for the import instance-type. WIT param names are cosmetic in the component
/// (the runtime resolves by function name), so a stable synthetic name suffices and keeps the
/// generator independent of the WIT's parameter spelling.
fn param_name(_fn_name: &str, index: usize) -> &'static str {
    const NAMES: &[&str] = &["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"];
    NAMES[index]
}

// wasm-encoder's core-instantiate arg enum, used above.
use wasm_encoder::ModuleArg;

/// Validate a component's bytes with the component-model features on; panic with context on failure.
/// This is the self-check that replaces the manual `wasm-tools validate` step — a generated envelope
/// that does not validate is a generation bug, caught here, never shipped.
fn validate_component(bytes: &[u8], what: &str) {
    use wasmparser::{Validator, WasmFeatures};
    let mut v = Validator::new_with_features(WasmFeatures::all());
    if let Err(e) = v.validate_all(bytes) {
        panic!("generated {what} failed component validation: {e}");
    }
}

// ─── Split at the embedded core-module boundary ──────────────────────────────────────────────────

/// Split a component into (HEAD, CORE, TAIL) at the embedded core-module section — the Rust port of
/// the `/tmp` splitter. A component embeds a core module as a section `0x01 <uleb len> <core bytes>`;
/// the core module starts with the core magic `00 61 73 6d 01 00 00 00`. HEAD is everything before
/// the section id, CORE is the module bytes, TAIL is everything after. The compiler slots its own
/// core module between HEAD and TAIL, so HEAD/TAIL are exactly the program-independent surround.
fn split_at_core_module(data: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    const CORE_MAGIC: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    let pos = find(data, CORE_MAGIC).expect("no embedded core-module magic in reference component");
    // The bytes just before `pos` are the uleb length, preceded by the section id `0x01`. Scan
    // candidate id positions and take the one whose uleb ends exactly at `pos`.
    for id_pos in (pos.saturating_sub(5)..pos).rev() {
        if data[id_pos] != 0x01 {
            continue;
        }
        let mut j = id_pos + 1;
        let (mut val, mut shift, mut ok) = (0u64, 0u32, true);
        loop {
            if j >= pos {
                ok = false;
                break;
            }
            let b = data[j];
            val |= u64::from(b & 0x7f) << shift;
            j += 1;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        if ok && j == pos {
            let core_end = pos + val as usize;
            return (data[..id_pos].to_vec(), data[pos..core_end].to_vec(), data[core_end..].to_vec());
        }
    }
    panic!("could not locate the core-module section framing");
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The offset within `tail` where the `run` component-type section begins — the split point the
/// scalar envelope path uses (`RT_TAIL[..RT_TAIL_PREFIX_LEN]`). The compound tail declares `run` as
/// `() -> string`; the pattern `07 05 01 40 00 00` (component type section: len 5, 1 type, func,
/// 0 params, 0 results-tag) precedes the `string` result byte, and the scalar path substitutes its
/// own `() -> <scalar>` type there. Derived, never hand-counted.
fn tail_prefix_len(tail: &[u8]) -> usize {
    const RUN_TYPE_SEC: &[u8] = &[0x07, 0x05, 0x01, 0x40, 0x00, 0x00];
    find(tail, RUN_TYPE_SEC).expect("no `run` component-type section in reference tail")
}

// ─── The compiler-built core module's import pieces (computed directly, not reference-derived) ────

/// `RT_IMPORT_CONTENT` — the import section CONTENT the compiler puts inside the core module it
/// builds: a count, then per import `<4>heap<len>name<0x00><type-index>`, in allow-list order with
/// `type-index == import-index` (the compiler appends the import types ahead of the defined-function
/// types, one per import at the same index). Computed directly from the interface — this lives inside
/// the compiler's own core module, not in the reference component, so it is generated, not split out.
fn import_content(iface: &[HeapFn]) -> Vec<u8> {
    let mut out = vec![iface.len() as u8];
    for (i, f) in iface.iter().enumerate() {
        out.push(4);
        out.extend_from_slice(b"heap");
        out.push(f.name.len() as u8);
        out.extend_from_slice(f.name.as_bytes());
        out.push(0x00); // func import
        out.push(i as u8); // type index == import index
    }
    out
}

/// One core functype `0x60 <params> <results>` per heap function, in allow-list order — the compiler's
/// `rt_import_types()`. Matches `Kind::core_valtype` byte-for-byte via `LogTy::core_byte`.
fn import_functype_bytes(f: &HeapFn) -> Vec<u8> {
    let mut out = vec![0x60];
    out.push(f.params.len() as u8);
    for p in &f.params {
        out.push(p.core_byte());
    }
    match f.result {
        Some(r) => {
            out.push(1);
            out.push(r.core_byte());
        }
        None => out.push(0),
    }
    out
}

// ─── The host-import shared-memory core module (fixed; generated for clarity, not derived) ────────

/// The `HOST_MEM_MODULE`: a tiny core module exporting `memory` (1 page) and a no-op
/// `cabi_realloc: (i32×4)->i32` returning 0, used by the host-import component to break the
/// canon-lowering memory circularity. Independent of the runtime interface, but generated here (with
/// wasm-encoder) rather than hand-pasted so its derivation is also a program, not a comment.
fn build_host_mem_module() -> Vec<u8> {
    use wasm_encoder::{
        CodeSection, ExportSection, Function, FunctionSection, Instruction, MemorySection,
        MemoryType, Module as CoreModule, TypeSection, ValType,
    };
    let mut m = CoreModule::new();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], [ValType::I32]);
    m.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    m.section(&funcs);
    let mut mems = MemorySection::new();
    mems.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("cabi_realloc", ExportKind::Func, 0);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut f = Function::new([]);
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::End);
    code.function(&f);
    m.section(&code);
    m.finish()
}

// ─── The all-const resource-with-display envelope (RUNNABLE_ENVELOPE_TAIL) ───────────────────────

/// Build the reference component for the all-const `runnable_component` path — the
/// resource-with-display ABI: `run:()->value` (a resource owning `display()->string`), exported as
/// `cadenza:run/run`. This is NOT interface-driven (independent of the runtime WIT); it is a FIXED
/// envelope, but generated here (wasm-encoder + wasmparser) rather than hand-pasted so its derivation
/// is a checked-in, re-runnable program. The compiler embeds its own core module (which bakes the
/// rendered string and exports `make`/`display`/`memory`/`cabi_realloc`) between HEAD and TAIL; only
/// the TAIL — everything after the core module — is baked as `RUNNABLE_ENVELOPE_TAIL`.
///
/// Shape (decoded from the frozen reference): a `value` resource; `make:()->own<value>` and
/// `display:(borrow<value>)->string` canon-lifted from the core module's exports; a nested inner
/// component that imports the resource/method/make and re-exports them under the wit-bindgen names
/// (`value`, `[method]value.display`, `make`); the inner component instantiated with those; and the
/// resulting instance exported as `cadenza:run/run`.
fn build_runnable_reference() -> Vec<u8> {
    use wasm_encoder::{
        CodeSection, ExportSection, Function, FunctionSection, ImportSection, Instruction,
        MemorySection, MemoryType, Module as CoreModule, TypeSection, ValType,
    };
    let mut c = ComponentBuilder::default();

    // ── The embedded core module STUB (thrown away; only HEAD-before-it and TAIL-after-it are kept;
    //    the compiler substitutes its real core module). Imports `intr.new:(i32)->i32`; exports
    //    memory + cabi_realloc:(i32×4)->i32 + make:()->i32 + display:(i32)->i32. Same export shape the
    //    compiler's real `runnable_component` core module has, so the aliases below resolve. ──
    let core = {
        let mut m = CoreModule::new();
        let mut types = TypeSection::new();
        types.ty().function([ValType::I32], [ValType::I32]); // 0: (i32)->i32  (intr.new, display)
        types.ty().function([ValType::I32; 4], [ValType::I32]); // 1: (i32×4)->i32 (realloc)
        types.ty().function([], [ValType::I32]); // 2: ()->i32 (make)
        m.section(&types);
        let mut imports = ImportSection::new();
        imports.import("intr", "new", EntityType::Function(0));
        m.section(&imports);
        let mut funcs = FunctionSection::new();
        funcs.function(1); // realloc
        funcs.function(2); // make
        funcs.function(0); // display
        m.section(&funcs);
        let mut mems = MemorySection::new();
        mems.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
        m.section(&mems);
        let mut exports = ExportSection::new();
        exports.export("memory", ExportKind::Memory, 0);
        exports.export("cabi_realloc", ExportKind::Func, 1);
        exports.export("make", ExportKind::Func, 2);
        exports.export("display", ExportKind::Func, 3);
        m.section(&exports);
        let mut code = CodeSection::new();
        let mut realloc = Function::new([(1, ValType::I32)]);
        realloc.instruction(&Instruction::I32Const(0)).instruction(&Instruction::End);
        code.function(&realloc);
        let mut make = Function::new([]);
        make.instruction(&Instruction::I32Const(0)).instruction(&Instruction::End);
        code.function(&make);
        let mut display = Function::new([]);
        display.instruction(&Instruction::I32Const(0)).instruction(&Instruction::End);
        code.function(&display);
        m.section(&code);
        m.finish()
    };

    // The embedded core module MUST come FIRST — the compiler splices its own core module at a fixed
    // offset (right after the component preamble) and appends the TAIL, so EVERYTHING else (resource
    // type, resource.new, instances, lifts, inner component) must land in the tail, AFTER the module.
    // Emitting `core_module_raw` before any resource machinery keeps the module the first section.
    let module_idx = c.core_module_raw(&core); // core module 0

    // ── The `value` resource + the resource.new intrinsic threaded into the core module. ──
    let res_ty = c.type_resource(ValType::I32, None); // component type 0: (resource (rep i32))
    let rnew = c.resource_new(res_ty); // core func 0: canon resource.new 0
    let intr_inst = c.core_instantiate_exports([("new", ExportKind::Func, rnew)]); // core instance 0
    let prog_inst = c.core_instantiate(module_idx, [("intr", ModuleArg::Instance(intr_inst))]); // core instance 1

    // ── make:()->own<value>, lifted from core `make`. ──
    let own_ty = {
        let (idx, enc) = c.type_defined();
        enc.own(res_ty);
        idx
    };
    let make_fnty = {
        let (idx, mut enc) = c.type_function();
        enc.params::<[(&str, ComponentValType); 0], _>([]).result(Some(ComponentValType::Type(own_ty)));
        idx
    };
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let make_fn = c.lift_func(make_core, make_fnty, []); // component func 0

    // ── display:(borrow<value>)->string, lifted from core `display` with memory+realloc. ──
    let borrow_ty = {
        let (idx, enc) = c.type_defined();
        enc.borrow(res_ty);
        idx
    };
    let display_fnty = {
        let (idx, mut enc) = c.type_function();
        enc.params([("self", ComponentValType::Type(borrow_ty))])
            .result(Some(ComponentValType::Primitive(PrimitiveValType::String)));
        idx
    };
    let display_core = c.core_alias_export(prog_inst, "display", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);
    let display_fn = c.lift_func(
        display_core,
        display_fnty,
        [CanonicalOption::Memory(mem), CanonicalOption::Realloc(realloc)],
    ); // component func 1

    // ── The nested inner component: imports the resource/method/make, re-exports them under the
    //    wit-bindgen names so the outer instance presents `value` + `[method]value.display` + `make`.
    let inner = build_runnable_inner();
    let inner_idx = c.component(inner); // component 0

    // Instantiate the inner component with our resource type + display + make.
    let run_inst = c.instantiate(
        inner_idx,
        [
            ("import-type-value", ComponentExportKind::Type, res_ty),
            ("import-method-value-display", ComponentExportKind::Func, display_fn),
            ("import-func-make", ComponentExportKind::Func, make_fn),
        ],
    ); // instance 0
    c.export("cadenza:run/run", ComponentExportKind::Instance, run_inst, None);

    let bytes = c.finish();
    validate_component(&bytes, "runnable (resource-with-display) reference");
    bytes
}

/// The nested inner component of the runnable envelope: imports the resource type, its `display`
/// method, and `make`, and re-exports them under the wit-bindgen-canonical names so the outer
/// instance's `cadenza:run/run` presents `value` / `[method]value.display` / `make`.
fn build_runnable_inner() -> ComponentBuilder {
    use wasm_encoder::TypeBounds;
    let mut ic = ComponentBuilder::default();
    // import "import-type-value" (type (sub resource)) → type 0
    let v = ic.import("import-type-value", ComponentTypeRef::Type(TypeBounds::SubResource));
    // display: (borrow v) -> string
    let borrow = {
        let (idx, enc) = ic.type_defined();
        enc.borrow(v);
        idx
    };
    let disp_ty = {
        let (idx, mut enc) = ic.type_function();
        enc.params([("self", ComponentValType::Type(borrow))])
            .result(Some(ComponentValType::Primitive(PrimitiveValType::String)));
        idx
    };
    let disp = ic.import("import-method-value-display", ComponentTypeRef::Func(disp_ty));
    // make: () -> own v
    let own = {
        let (idx, enc) = ic.type_defined();
        enc.own(v);
        idx
    };
    let make_ty = {
        let (idx, mut enc) = ic.type_function();
        enc.params::<[(&str, ComponentValType); 0], _>([]).result(Some(ComponentValType::Type(own)));
        idx
    };
    let mk = ic.import("import-func-make", ComponentTypeRef::Func(make_ty));
    // Re-export the resource under the name `value`; this yields a FRESH exported resource type.
    let ev = ic.export("value", ComponentExportKind::Type, v, None);
    // The method/make exports must be ascribed func types over the EXPORTED resource `ev` (not the
    // imported `v`), matching the wit-bindgen reference (`(func (type 7))` over `borrow $ev`).
    let ev_borrow = {
        let (idx, enc) = ic.type_defined();
        enc.borrow(ev);
        idx
    };
    let disp_ext_ty = {
        let (idx, mut enc) = ic.type_function();
        enc.params([("self", ComponentValType::Type(ev_borrow))])
            .result(Some(ComponentValType::Primitive(PrimitiveValType::String)));
        idx
    };
    ic.export(
        "[method]value.display",
        ComponentExportKind::Func,
        disp,
        Some(ComponentTypeRef::Func(disp_ext_ty)),
    );
    let ev_own = {
        let (idx, enc) = ic.type_defined();
        enc.own(ev);
        idx
    };
    let make_ext_ty = {
        let (idx, mut enc) = ic.type_function();
        enc.params::<[(&str, ComponentValType); 0], _>([]).result(Some(ComponentValType::Type(ev_own)));
        idx
    };
    ic.export("make", ComponentExportKind::Func, mk, Some(ComponentTypeRef::Func(make_ext_ty)));
    ic
}

// ─── Emit the generated Rust sources (write-if-changed) ──────────────────────────────────────────

/// Format a byte slice as a Rust `&[u8]` array literal body (20 per line, 4-space indent), matching
/// the checked-in style so a re-baseline diff is minimal and readable.
fn bytes_literal(bytes: &[u8]) -> String {
    let mut s = String::new();
    for chunk in bytes.chunks(20) {
        s.push_str("    ");
        for b in chunk {
            s.push_str(&b.to_string());
            s.push_str(", ");
        }
        s.push('\n');
    }
    s
}

// `write_if_changed` (the Membrain `core-codegen` no-op-if-unchanged pattern) is shared with the
// opcode generator, so it lives in `main.rs` as `crate::write_if_changed`.
use crate::write_if_changed;

const GEN_HEADER: &str = "\
// @generated by `cargo run -p xtask -- build` from the runtime WIT (crates/cdz-runtime/wit/runtime.wit)
// and xtask/src/wit_envelope.rs::HEAP_ALLOWLIST. DO NOT EDIT — edit the WIT or the allow-list and rebuild.
//
// The value-heap runtime interface has exactly ONE source of truth: the runtime's WIT. This file is
// the compiler's view of it — the component-model envelope byte-chunks, the import indices, the core
// signatures, and the required-runtime pin — all derived, self-validated (wasmparser), and split from
// a wasm-encoder-built reference. See spec/learnings/2026-07-06-the-envelope-blobs-are-generated-from-the-runtime-contract.md.
";

/// Render the compiler's `heap_envelope.rs`: the byte constants, `RT_N_IMPORTS`,
/// `RT_TAIL_PREFIX_LEN`, `REQUIRED_RUNTIME_HASH`, `HOST_MEM_MODULE`, `mod himport`, and
/// `rt_import_types()`.
fn render_heap_envelope(iface: &[HeapFn], runtime_hash: &str, host_mem: &[u8]) -> String {
    let reference = build_heap_reference(iface);
    let (head, _core, tail) = split_at_core_module(&reference);
    let prefix_len = tail_prefix_len(&tail);
    let import_content = import_content(iface);
    let n = iface.len();

    let mut s = String::new();
    s.push_str(GEN_HEADER);
    s.push('\n');

    // The required-runtime pin (was a dead env var; now an in-source constant).
    s.push_str(&format!(
        "/// The content address (SHA-256) of the value-heap runtime this compiler targets. The\n\
         /// compiler↔runtime versioned pair: the runtime is derived first and its hash baked here, so\n\
         /// the pin is a deterministic function of the runtime source, not a build-time env var. A\n\
         /// forward-looking pin — the emitted-component required-runtime record is not yet wired, so\n\
         /// this is `dead_code` until then (component-abi.md §The Emitted Component Records Its\n\
         /// Required Runtime).\n\
         #[allow(dead_code)]\n\
         pub const REQUIRED_RUNTIME_HASH: &str = \"{runtime_hash}\";\n\n"
    ));

    // How many heap functions the envelope imports + lowers.
    s.push_str(&format!(
        "/// How many heap functions the envelope imports + lowers (indices 0..RT_N_IMPORTS).\n\
         pub const RT_N_IMPORTS: u32 = {n};\n\n"
    ));

    // himport index constants (SCREAMING_SNAKE of the kebab name).
    s.push_str(
        "/// Absolute core-func indices of the imported heap operations (stable; never offset), in\n\
         /// the compiler's import order (== RT_IMPORT_CONTENT order; the runtime resolves by name).\n\
         #[allow(dead_code)]\n\
         pub mod himport {\n",
    );
    for (i, f) in iface.iter().enumerate() {
        s.push_str(&format!("    pub const {}: u32 = {i};\n", screaming(&f.name)));
    }
    s.push_str("}\n\n");

    // rt_import_types(): one core functype per import, in order.
    s.push_str(
        "/// The heap-import core functypes, in the compiler's import order — one per imported\n\
         /// function. These precede the defined-function types in the emitted core module's type\n\
         /// section; each import references its type by the same index.\n\
         pub fn rt_import_types() -> Vec<Vec<u8>> {\n    vec![\n",
    );
    for f in iface {
        let bytes = import_functype_bytes(f);
        let lits: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
        s.push_str(&format!("        vec![{}], // {}\n", lits.join(", "), f.name));
    }
    s.push_str("    ]\n}\n\n");

    // The byte constants.
    s.push_str(&const_bytes("RT_HEAD", "component HEAD: magic + heap import instance-type + the canon-lowers", &head));
    s.push_str(&const_bytes("RT_TAIL", "component TAIL: core instance, run/memory/realloc aliases, run:()->string lift+export", &tail));
    s.push_str(&const_bytes(
        "RT_IMPORT_CONTENT",
        "the import section CONTENT the compiler puts inside its core module (count + per-import heap/name/type)",
        &import_content,
    ));
    s.push_str(&format!(
        "/// Byte offset within RT_TAIL where the `run` component-type section begins — the scalar\n\
         /// envelope uses `RT_TAIL[..RT_TAIL_PREFIX_LEN]` then appends its own `()->scalar` lift.\n\
         pub const RT_TAIL_PREFIX_LEN: usize = {prefix_len};\n\n"
    ));
    // The memory + global sections are fixed (1 page; bump ptr = 16).
    s.push_str(&const_bytes("RT_MEM", "memory section content (one memory, min 1 page)", &[1, 0, 1]));
    s.push_str(&const_bytes("RT_GLOBAL", "global section content (bump ptr = 16)", &[1, 127, 1, 65, 16, 11]));

    // The host-import shared-memory core module (fixed; used by the host-import component path).
    s.push_str(&const_bytes(
        "HOST_MEM_MODULE",
        "fixed shared-memory core module: exports `memory` (1 page) + a no-op `cabi_realloc`",
        host_mem,
    ));

    // The all-const resource-with-display envelope TAIL: everything AFTER the embedded core module of
    // the runnable (resource-with-display) reference. The compiler writes the component preamble + its
    // own core module, then appends this. Not interface-driven, but generated (wasm-encoder) for the
    // same reason — its derivation is a checked-in program, not a `/tmp` script.
    let runnable = build_runnable_reference();
    let (_rhead, _rcore, rtail) = split_at_core_module(&runnable);
    s.push_str(&const_bytes(
        "RUNNABLE_ENVELOPE_TAIL",
        "resource-with-display envelope: everything after the core module (resource, make/display lifts, inner component, cadenza:run/run export)",
        &rtail,
    ));

    // The `compile : list<u8> -> list<u8>` envelope (GAP 3l): a `bytes → bytes` entry — a
    // Cadenza-authored compiler — is emitted through this, exported as `cadenza:compiler/compile`,
    // so the host harness drives it over the corpus. HEAD = magic + heap import instance-type +
    // canon-lowers (same as RT_HEAD but for this component's type space); TAIL = the heap
    // core-instance instantiation + compile/memory/realloc aliases + the `compile` list-lift+export.
    // The compiler splices its own core module (which imports the heap funcs and exports
    // `compile`/`memory`/`cabi_realloc`) between HEAD and TAIL.
    let compile_ref = build_compile_reference(iface);
    let (chead, _ccore, ctail) = split_at_core_module(&compile_ref);
    s.push_str(&const_bytes(
        "COMPILE_HEAD",
        "compile-envelope HEAD: magic + heap import instance-type + the canon-lowers",
        &chead,
    ));
    s.push_str(&const_bytes(
        "COMPILE_TAIL",
        "compile-envelope TAIL: heap core-instance, compile/memory/realloc aliases, compile:(list<u8>)->list<u8> lift+export",
        &ctail,
    ));

    // The DIAGNOSTICS-ABI envelope (ask-30 sub-gap 2): a `compile` entry lifted as
    // `func(list<u8>) -> result<list<u8>, list<diagnostic>>`, so a Cadenza-authored compiler can
    // return `Ok(component-bytes)` for a well-typed program or `Err(list<diagnostic>)` — a coded
    // rejection — for an ill-typed one, instead of trapping. Same HEAD/TAIL split; the compiler
    // splices its core module (whose `compile` writes the result's `[discriminant, payload…]` retptr)
    // between them.
    let compile_result_ref = build_compile_result_reference(iface);
    let (crhead, _crcore, crtail) = split_at_core_module(&compile_result_ref);
    s.push_str(&const_bytes(
        "COMPILE_RESULT_HEAD",
        "diagnostics-ABI compile-envelope HEAD: magic + heap import instance-type + the canon-lowers",
        &crhead,
    ));
    s.push_str(&const_bytes(
        "COMPILE_RESULT_TAIL",
        "diagnostics-ABI compile-envelope TAIL: heap core-instance, aliases, compile:(list<u8>)->result<list<u8>,list<diagnostic>> lift+export",
        &crtail,
    ));

    // The KINDED-ARTIFACT envelope (ask-41 / Amendment 0.8.0): `compile: func(list<artifact>) ->
    // compile-output{artifacts,diagnostics}` — the symmetric artifacts-in/out interface. The compiler
    // splices its core module (whose `compile` unmarshals the input artifact list and writes the
    // output record's retptr) between HEAD and TAIL.
    let compile_artifacts_ref = build_compile_artifacts_reference(iface);
    let (cahead, _cacore, catail) = split_at_core_module(&compile_artifacts_ref);
    s.push_str(&const_bytes(
        "COMPILE_ARTIFACTS_HEAD",
        "artifact-ABI compile-envelope HEAD: magic + heap import instance-type + the canon-lowers",
        &cahead,
    ));
    s.push_str(&const_bytes(
        "COMPILE_ARTIFACTS_TAIL",
        "artifact-ABI compile-envelope TAIL: heap core-instance, aliases, compile:(list<artifact>)->compile-output lift+export",
        &catail,
    ));

    s
}

/// Render `runtime_funcs.rs` for the host: the names it forwards into each emitted program's imports.
fn render_runtime_funcs(iface: &[HeapFn]) -> String {
    let mut s = String::new();
    s.push_str(GEN_HEADER);
    s.push_str(
        "\n/// The heap functions the runtime interface exports; the host forwards each into the\n\
         /// program's import at composition. MUST cover every function RT_IMPORT_CONTENT names.\n\
         pub const RUNTIME_FUNCS: &[&str] = &[\n",
    );
    for f in iface {
        s.push_str(&format!("    {:?},\n", f.name));
    }
    s.push_str("];\n");
    s
}

fn const_bytes(name: &str, doc: &str, bytes: &[u8]) -> String {
    format!(
        "/// {doc}.\npub const {name}: &[u8] = &[\n{}];\n\n",
        bytes_literal(bytes)
    )
}

/// kebab-case → SCREAMING_SNAKE_CASE (`bytes-concat` → `BYTES_CONCAT`).
fn screaming(name: &str) -> String {
    name.to_uppercase().replace('-', "_")
}

/// Generate both files from the runtime WIT + the baked runtime hash. `seed` is the seed root
/// (`<repo>/implementation/seed`). Returns whether anything changed (for a caching-friendly log).
pub fn generate(seed: &Path, runtime_hash: &str) -> Result<bool, String> {
    let wit = seed.join("crates/cdz-runtime/wit/runtime.wit");
    let iface = resolve_iface(&wit)?;

    // Self-check: also build + validate the host-mem module so a break there is caught here.
    let host_mem = build_host_mem_module();
    validate_host_mem(&host_mem);

    let envelope = render_heap_envelope(&iface, runtime_hash, &host_mem);
    let funcs = render_runtime_funcs(&iface);

    let env_path = seed.join("crates/cdz-compiler/src/heap_envelope.rs");
    let funcs_path = seed.join("crates/cadenza-seed/src/runtime_funcs.rs");
    let a = write_if_changed(&env_path, &envelope).map_err(|e| format!("write {}: {e}", env_path.display()))?;
    let b = write_if_changed(&funcs_path, &funcs).map_err(|e| format!("write {}: {e}", funcs_path.display()))?;
    Ok(a || b)
}

/// A core module cannot be validated as a component; validate it as a bare module.
fn validate_host_mem(bytes: &[u8]) {
    use wasmparser::{Validator, WasmFeatures};
    let mut v = Validator::new_with_features(WasmFeatures::all());
    if let Err(e) = v.validate_all(bytes) {
        panic!("generated host-mem core module failed validation: {e}");
    }
}
