//! `envelope` — the N-export component-model envelope wrapping an embedded core module.
//!
//! A component exporting N functions is fully mechanical. There are TWO shapes, chosen by whether the
//! program imports the value-heap runtime:
//!
//!  * NO runtime import (`imports` empty) — the bare shape: one embedded core module, a fixed
//!    core-instance (`instantiate module 0`), then four count-prefixed vectors (component types, core
//!    aliases, canon lifts, component exports), one entry per export. Byte-identical to what the
//!    program emitted before value heap (`component-abi.md` v3 migration: a program importing nothing
//!    crosses as under v2).
//!  * WITH a runtime import (`imports` non-empty) — the import shape: an import-instance-type declaring
//!    the used ops, a component import of `cadenza:runtime/heap@…+<hash>` as an instance of that type,
//!    an alias+canon-lower of each op to a core func, the embedded core module, TWO core instances (the
//!    lowered ops exported as `"heap"`, then the program instantiated threading `"heap"` in), and the
//!    boundary alias/lift/export sequence off the PROGRAM instance. This is the 7-step shape the old
//!    `wit_envelope.rs::build_heap_envelope` used.
//!
//! The per-item byte grammar of BOTH shapes is byte-identical to the authoritative component-model
//! encoder (`wasm-encoder`'s `ComponentBuilder`; the oracle tests pin each shape), which is what
//! licenses hand-emitting the envelope with no external encoder in the compile path
//! (`reference-compiler.md` §Emission Is Validated Byte-Identical To An Independent Encoder). Section
//! ORDER is load-bearing: the bare shape puts component TYPE (7) before core ALIAS (6); the import
//! shape follows `ComponentBuilder`'s call order (instance-type → import → alias-ops → lower-ops →
//! core-module → core-instances → core-alias → boundary-type → lift → export).
//!
//! What comes from where: the single-byte ABI values — the component MAGIC header, the section ids,
//! the component functype form tag — are read from the GENERATED `wasm_abi` table (extracted from
//! `wasm-encoder`), so none is hand-typed. The per-item GRAMMARS below (`INSTANCE_BODY`, the alias /
//! canon-lift / lower / export items, the result-list form) still lay their bytes by hand: they
//! encode the component-model "sort" tags (`0x00` core, `0x01` func, …) which `wasm-encoder` does
//! NOT expose as public constants. Those are pinned instead by the byte-identity oracle tests — a
//! whole-item diff against the authoritative encoder, the stronger check for a structural encoding.

use crate::backend::wasm::encode::{section, uleb_bytes, uleb128, wasm_vec};
use crate::backend::wasm::runtime_abi::RtOp;
use crate::backend::wasm::wasm_abi;

/// The component-model preamble (`\0asm` + component-layer version) — from the generated `wasm_abi`
/// table (`Component::HEADER` as `wasm-encoder` writes it), not a hand-typed byte string.
const COMPONENT_MAGIC: &[u8] = wasm_abi::COMPONENT_MAGIC;

/// Component section ids used by the envelope (component-model numbering, distinct from core wasm) —
/// each re-named from the generated `wasm_abi` table (extracted from `wasm-encoder`'s
/// `ComponentSectionId`), so no section id is hand-typed here.
mod sec {
    use crate::backend::wasm::wasm_abi;
    pub const CORE_MODULE: u8 = wasm_abi::COMP_SEC_CORE_MODULE;
    pub const CORE_INSTANCE: u8 = wasm_abi::COMP_SEC_CORE_INSTANCE;
    pub const ALIAS: u8 = wasm_abi::COMP_SEC_ALIAS;
    pub const COMPONENT_TYPE: u8 = wasm_abi::COMP_SEC_TYPE;
    pub const CANON: u8 = wasm_abi::COMP_SEC_CANONICAL;
    pub const COMPONENT_IMPORT: u8 = wasm_abi::COMP_SEC_IMPORT;
    pub const COMPONENT_EXPORT: u8 = wasm_abi::COMP_SEC_EXPORT;
    pub const COMPONENT: u8 = wasm_abi::COMP_SEC_COMPONENT;
    pub const COMPONENT_INSTANCE: u8 = wasm_abi::COMP_SEC_INSTANCE;
}

/// The core wasm module name the program's core module imports the runtime funcs from, and the name
/// the threaded core-instance of lowered ops is bound under (they must match).
const HEAP_MODULE: &str = "heap";
/// The core module name the RUNTIME resource dtor imports `drop` from — a SEPARATE small instance of the
/// lowered `drop` op (distinct from `heap`), so the dtor can instantiate before the resource type
/// without depending on the resource intrinsics (R2, [[rcdzc-r1-resource-encode-linking-findings]]).
const HEAP_DTOR_MODULE: &str = "heap-dtor";
/// The runtime op the dtor calls to release the compound's rc handle — its name in the used-op set.
const RUNTIME_DROP: &str = "drop";

/// The well-known component instance the resource escape path publishes — the `cadenza:run/run`
/// interface `cdz-run` reaches into for the `make`/`encode` exports (the resource-aware host, R3).
const RUN_INTERFACE: &str = "cadenza:run/run";
/// The core-module import name for the `resource.new` intrinsic the escape shape threads in (module
/// [`HEAP_MODULE`], export `resource-new`) — the resource type's constructor lowered to a core func.
const RESOURCE_NEW: &str = "resource-new";
/// The core-module import name for the `resource.rep` intrinsic (module [`HEAP_MODULE`], export
/// `resource-rep`) — recovers the heap rep from a resource handle. Threaded into the RUNTIME resource
/// shape (R2) so `t-encode` can turn its `own<t>` handle param back into the walkable heap rep.
const RESOURCE_REP: &str = "resource-rep";
/// The core-module export names the escape shape aliases off the program instance: the resource
/// constructor, the `encode` method body, and the canonical-ABI memory + realloc the list-lift reads.
const MAKE_CORE_EXPORT: &str = "make";
const ENCODE_CORE_EXPORT: &str = "t-encode";
const MEMORY_EXPORT: &str = "memory";
const REALLOC_EXPORT: &str = "cabi_realloc";
/// The dtor core module's single export — the resource destructor the component invokes on host-drop
/// (its own module so it instantiates first, dissolving the resource↔dtor↔`resource.new` cycle without
/// a shim; [[rcdzc-r1-resource-encode-linking-findings]]).
const DTOR_CORE_EXPORT: &str = "t-dtor";
/// The boundary export names the resource + its two methods cross under (inside [`RUN_INTERFACE`]).
const RESOURCE_TYPE_NAME: &str = "t";
const MAKE_BOUNDARY_NAME: &str = "make";
const ENCODE_BOUNDARY_NAME: &str = "encode";
/// The interface a CLOSURE-resource export publishes under, and its method name — distinct from the
/// value-escape's `cadenza:run/run` + `encode` because the host contract is a callable method, not a
/// serializer (`DESIGN-closure-host-resource-rcdzc.md`). The core module exports `make`/`call` (a closure
/// resource needs no `memory`/`cabi_realloc` for scalar args), so `CALL_CORE_EXPORT == CALL_BOUNDARY_NAME`.
const CLOSURE_INTERFACE: &str = "cadenza:closure/exports";
const CALL_CORE_EXPORT: &str = "call";
const CALL_BOUNDARY_NAME: &str = "call";

/// An export's RESULT as the boundary crosses it. A scalar crosses as a component primitive byte; a
/// compound's canonical binary value form crosses as `list<u8>` (the R0 escape-path ABI — the resource
/// method's `encode()` return); a unit-returning export has no result. Distinguished as an enum (not a
/// bare `Option<u8>`) because a `list<u8>` result reshapes the envelope: it needs the core module's
/// `memory` + `cabi_realloc` aliased in and the canon-lift to carry Memory/Realloc options, so the
/// distinction must survive to the assembler.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BoundaryResult {
    /// No result — a unit-returning export.
    None,
    /// A component primitive valtype byte (`s64`/`u32`/`bool`/…) — the faithful boundary form of a
    /// scalar. Crosses by value with no memory involvement.
    Primitive(u8),
    /// A `list<u8>` — the canonical binary value form of a compound, crossing through linear memory by
    /// the canonical ABI (a `(ptr, len)` return area). Requires the exporting core module to export
    /// `memory` + `cabi_realloc` (`contracts/deterministic-value-form.md`; `value-interchange.md`).
    Bytes,
}

/// One export as the envelope assembler needs it: its verbatim boundary name, its parameter component
/// valtype bytes (in order; empty for a nullary export), and its result form (see [`BoundaryResult`]).
pub struct BoundaryExport {
    pub name: String,
    pub params: Vec<u8>,
    pub result: BoundaryResult,
}

/// Assemble the whole component around an embedded `core` module. `exports` are the boundary exports
/// (in declaration/emission order; export `j` lifts core func `import_count + j`). `imports` is the
/// program's per-program set of runtime ops (ordered, same order the core module imported them);
/// `import_name` is the versioned interface name (`cadenza:runtime/heap@0.0.0+<hash>`) to import the
/// runtime instance under. An empty `imports` emits the BARE shape (byte-identical to a runtime-free
/// program); `import_name` is then unused.
pub fn assemble(
    core: &[u8],
    exports: &[BoundaryExport],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    if imports.is_empty() {
        assemble_bare(core, exports)
    } else {
        assemble_with_imports(core, exports, imports, import_name)
    }
}

/// The BARE shape (no runtime import). Two sub-shapes, chosen by whether any export returns a
/// `list<u8>` (the canonical binary value form — the escape path). An all-scalar/unit program takes the
/// original TYPE(7)-before-ALIAS(6) path, byte-identical to a pre-value-heap program (the scalar
/// byte-neutrality guard). A program with a `list<u8>` result takes the [`assemble_bare_bytes`] path,
/// which aliases the core module's `memory` + `cabi_realloc` and lifts through them — matching the
/// `ComponentBuilder` oracle (ALIAS(6)-before-TYPE(7), its call order).
fn assemble_bare(core: &[u8], exports: &[BoundaryExport]) -> Vec<u8> {
    if exports.iter().any(|e| e.result == BoundaryResult::Bytes) {
        return assemble_bare_bytes(core, exports);
    }
    let n = exports.len();

    // sec 7: one component functype per export (nullary → its result form). No `list<u8>` result here
    // (the caller routed those to `assemble_bare_bytes`), so the list-type index is unused.
    let mut type_items = Vec::new();
    for e in exports {
        type_items.extend_from_slice(&comp_functype(e, 0));
    }
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(n, &type_items));

    // sec 6: one core-func alias per export (alias core-instance 0's export by name).
    let mut alias_items = Vec::new();
    for e in exports {
        alias_items.extend_from_slice(&core_alias_item(0, &e.name));
    }
    let alias_sec = section(sec::ALIAS, &wasm_vec(n, &alias_items));

    // sec 8: one canon-lift per export (lift core func i using component type i).
    let mut canon_items = Vec::new();
    for i in 0..n {
        canon_items.extend_from_slice(&canon_lift_item(i as u32, i as u32));
    }
    let canon_sec = section(sec::CANON, &wasm_vec(n, &canon_items));

    // sec 11: one component export per export (export component func i under its verbatim name).
    let mut export_items = Vec::new();
    for (i, e) in exports.iter().enumerate() {
        export_items.extend_from_slice(&comp_export_item(&e.name, i as u32));
    }
    let export_sec = section(sec::COMPONENT_EXPORT, &wasm_vec(n, &export_items));

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 1: embedded core module.
    out.extend_from_slice(&core_module_section(core));
    // sec 2: core instance (instantiate module 0, no args).
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &[0x01, 0x00, 0x00, 0x00]));
    // Component TYPE (7) BEFORE core ALIAS (6) — load-bearing for byte identity.
    out.extend_from_slice(&type_sec);
    out.extend_from_slice(&alias_sec);
    out.extend_from_slice(&canon_sec);
    out.extend_from_slice(&export_sec);
    out
}

/// The BARE shape with a `list<u8>` result — the escape-path ABI (a compound's canonical binary value
/// form crosses as `list<u8>`, the resource `encode()` return). No runtime import, but the lift reads
/// the `(ptr, len)` return area out of the core module's linear memory, so it aliases the core's
/// `memory` + `cabi_realloc` and carries them as canon-lift options. Follows the `ComponentBuilder`
/// oracle's call order — ALIAS(6) before TYPE(7) — so the bytes match. Index spaces (with
/// `m = exports.len()`):
///   * export core-funcs → core funcs `0..m`; `cabi_realloc` → core func `m`; `memory` → memory 0.
///   * `list u8` defined type → component type 0; boundary functypes → component types `1..=m`.
///   * lifts → component funcs `0..m` (no lowered ops precede them in the bare shape).
///
/// A scalar export mixed in (a program exporting both a scalar and a compound) lifts with no options; a
/// `list<u8>` export lifts with Memory+Realloc. The core module MUST export `memory` and
/// `cabi_realloc` (the R0 serializer emits both whenever any export returns `list<u8>`).
fn assemble_bare_bytes(core: &[u8], exports: &[BoundaryExport]) -> Vec<u8> {
    let m = exports.len();
    let list_type_idx: u32 = 0; // the shared `list u8` type is component type 0
    let mem_idx: u32 = 0; // the sole aliased memory
    let realloc_func: u32 = m as u32; // core func after the m export funcs

    // sec 6: alias each export's core func (core funcs 0..m), then `memory`, then `cabi_realloc`.
    let mut alias_items = Vec::new();
    for e in exports {
        alias_items.extend_from_slice(&core_alias_item(0, &e.name));
    }
    alias_items.extend_from_slice(&memory_alias_item(0, "memory"));
    alias_items.extend_from_slice(&core_alias_item(0, "cabi_realloc"));
    let alias_sec = section(sec::ALIAS, &wasm_vec(m + 2, &alias_items));

    // sec 7: the shared `list u8` defined type (type 0), then one functype per export (types 1..=m).
    let mut type_items = list_u8_defined_type();
    for e in exports {
        type_items.extend_from_slice(&comp_functype(e, list_type_idx));
    }
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1 + m, &type_items));

    // sec 8: lift export j (core func j) using functype type `1+j`; a `list<u8>` result carries the
    // Memory+Realloc options, a scalar result none.
    let mut canon_items = Vec::new();
    for (j, e) in exports.iter().enumerate() {
        let core_func = j as u32;
        let type_idx = (1 + j) as u32;
        match e.result {
            BoundaryResult::Bytes => canon_items.extend_from_slice(&canon_lift_list_item(
                core_func,
                mem_idx,
                realloc_func,
                type_idx,
            )),
            _ => canon_items.extend_from_slice(&canon_lift_item(core_func, type_idx)),
        }
    }
    let canon_sec = section(sec::CANON, &wasm_vec(m, &canon_items));

    // sec 11: export each lifted component func (0..m) under its verbatim name.
    let mut export_items = Vec::new();
    for (j, e) in exports.iter().enumerate() {
        export_items.extend_from_slice(&comp_export_item(&e.name, j as u32));
    }
    let export_sec = section(sec::COMPONENT_EXPORT, &wasm_vec(m, &export_items));

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&core_module_section(core)); // sec 1
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &[0x01, 0x00, 0x00, 0x00])); // sec 2
    // ALIAS(6) before TYPE(7) here — the oracle's call order for the memory/realloc-lift shape.
    out.extend_from_slice(&alias_sec);
    out.extend_from_slice(&type_sec);
    out.extend_from_slice(&canon_sec);
    out.extend_from_slice(&export_sec);
    out
}

/// The IMPORT shape (the program imports `k = imports.len()` runtime ops). Follows `ComponentBuilder`'s
/// call order so the bytes match the oracle. Index spaces (with `m = exports.len()`):
///   * lowered ops → core funcs `0..k`; boundary core-aliases → core funcs `k..k+m`.
///   * import instance-type → component type 0; boundary functypes → component types `1..=m`.
///   * op aliases → component funcs `0..k`; lifts → component funcs `k..k+m`.
///   * heap-exports core-instance → core instance 0; program → core instance 1 (its exports are what
///     the boundary aliases read).
fn assemble_with_imports(
    core: &[u8],
    exports: &[BoundaryExport],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    let k = imports.len();
    let m = exports.len();

    // sec 7: the import instance-type — component type 0. `0x42` then a vec of 2k declarations,
    // INTERLEAVED per op: a `ty` decl (the op's component functype) then an `export` decl naming the
    // op and referencing that func type by index.
    let instance_type = {
        let mut decls = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            // ty decl: `01` <component-functype>.
            decls.push(0x01);
            decls.extend_from_slice(&op_comp_functype(op));
            // export decl: `04` <export-name> <ComponentTypeRef::Func(i)>.
            decls.push(0x04);
            decls.extend_from_slice(&extern_name(op.name));
            decls.push(0x01); // sort: component func
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42]; // instance type form
        it.extend_from_slice(&wasm_vec(2 * k, &decls));
        it
    };
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type));

    // sec 10: import the runtime interface as an instance of component type 0.
    let import_sec = {
        let mut item = extern_name(import_name);
        item.push(0x05); // ComponentTypeRef::Instance sort
        uleb128(0, &mut item); // type index 0
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };

    // sec 6 (first): alias each op out of the imported instance (component instance 0) → component
    // funcs `0..k`.
    let op_alias_sec = {
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    };

    // sec 8 (first): canon-lower each aliased op (component func `i`) → core funcs `0..k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    };

    // sec 2: TWO core instances — (0) the lowered ops exported under their names, forming the `"heap"`
    // instance; (1) the program module instantiated with `"heap"` bound to instance 0.
    let core_instance_sec = {
        let mut items = Vec::new();
        // instance 0: export-items form (`0x01`) of the k lowered core funcs (indices 0..k).
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00); // ExportKind::Func
            uleb128(i as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        // instance 1: instantiate module 0 with one arg `"heap" = instance 0`.
        let mut prog = vec![0x00]; // instantiate form
        uleb128(0, &mut prog); // module index 0
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12); // ModuleArg::Instance sort (CORE_INSTANCE_SORT)
        uleb128(0, &mut args); // core instance 0
        prog.extend_from_slice(&wasm_vec(1, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(2, &items))
    };

    // sec 6 (second): alias each boundary func out of the PROGRAM instance (core instance 1) → core
    // funcs `k..k+m`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(1, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };

    // sec 7 (second): one component functype per boundary export → component types `1..=m`. This
    // multi-export-with-imports assembler never carries a `list<u8>` result: a compound that escapes
    // AND uses runtime ops takes the dedicated `assemble_runtime_resource` path instead (the resource
    // whose `encode()` walks the live handle), so no `list u8` defined type is laid here and
    // `comp_functype`'s list-type index is unused. The `debug_assert` pins that a `Bytes` result never
    // reaches this assembler.
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            debug_assert!(
                e.result != BoundaryResult::Bytes,
                "a list<u8> boundary result takes the assemble_runtime_resource path, not this one"
            );
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };

    // sec 8 (second): lift each boundary core func (`k+j`) using its component type (`1+j`) → component
    // funcs `k..k+m`.
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((k + j) as u32, (1 + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };

    // sec 11: export each lifted component func (`k+j`) under its verbatim boundary name.
    let export_sec = {
        let mut items = Vec::new();
        for (j, e) in exports.iter().enumerate() {
            items.extend_from_slice(&comp_export_item(&e.name, (k + j) as u32));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: import instance-type
    out.extend_from_slice(&import_sec); // 10: component import
    out.extend_from_slice(&op_alias_sec); // 6: alias ops out of the import
    out.extend_from_slice(&lower_sec); // 8: lower ops → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: embedded program
    out.extend_from_slice(&core_instance_sec); // 2: heap-instance + program-instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&boundary_type_sec); // 7: boundary functypes
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&export_sec); // 11: export
    out
}

/// One host-import function the [`assemble_host`] shape imports: the operation NAME (the func the effect
/// interface exports) and its component functype BYTES (a `0x40 <params> <result>` item — the caller
/// builds it from the op's scalar signature). The declaring effect (the interface name) is a separate
/// argument since this increment delegates a SINGLE effect.
pub struct HostFn {
    pub op: String,
    /// The op's component functype item bytes (`0x40 …`) — declared in the effect's instance-type AND
    /// (re)used for the core import functype indirectly via the lowered form.
    pub comp_functype: Vec<u8>,
    /// The op's CORE functype item bytes (`0x60 <params> <results>`) — the type the program's core module
    /// imports the lowered op under. Built by the caller from the op's core valtypes.
    pub core_functype: Vec<u8>,
}

/// The HOST-IMPORT shape (E2h-2): a program that DELEGATES a single effect `iface` to the host, importing
/// `h = host_fns.len()` operations of it as a component INTERFACE (an instance-type declaring each op as a
/// func), aliasing + lowering each to a core func the program binds under module `"host"`. Structurally
/// the runtime-import shape ([`assemble_with_imports`]) with the imported instance named by the EFFECT
/// (a dotted `E.op` is never a top-level extern — the component model forbids the dot, so the boundary is
/// `interface iface { func op }`). SCOPE: host-only (no value-heap runtime import) and scalar ops — a
/// program mixing host + runtime, or a string/compound op, declines upstream.
///
/// Index spaces (with `m = exports.len()`):
///   * lowered ops → core funcs `0..h`; boundary core-aliases → core funcs `h..h+m`.
///   * effect instance-type → component type 0; boundary functypes → component types `1..=m`.
///   * op aliases → component funcs `0..h`; lifts → component funcs `h..h+m`.
///   * imported effect instance → component instance 0; program → core instance 1.
pub fn assemble_host(
    core: &[u8],
    exports: &[BoundaryExport],
    iface: &str,
    host_fns: &[HostFn],
) -> Vec<u8> {
    let h = host_fns.len();
    let m = exports.len();

    // sec 7: the effect's instance-type — component type 0. A vec of 2h declarations, INTERLEAVED per op:
    // a `ty` decl (the op's component functype) then an `export` decl naming the op + referencing that
    // func type by index. Identical shape to the runtime import instance-type, but the exported ops are
    // the effect's operations.
    let instance_type = {
        let mut decls = Vec::new();
        for (i, f) in host_fns.iter().enumerate() {
            decls.push(0x01); // ty decl
            decls.extend_from_slice(&f.comp_functype);
            decls.push(0x04); // export decl
            decls.extend_from_slice(&extern_name(&f.op));
            decls.push(0x01); // sort: component func
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42]; // instance type form
        it.extend_from_slice(&wasm_vec(2 * h, &decls));
        it
    };
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type));

    // sec 10: import the effect interface as an instance of component type 0, under the effect's name.
    let import_sec = {
        let mut item = extern_name(iface);
        item.push(0x05); // ComponentTypeRef::Instance sort
        uleb128(0, &mut item); // type index 0
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };

    // sec 6 (first): alias each op out of the imported effect instance (component instance 0) → component
    // funcs `0..h`.
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(0, &f.op));
        }
        section(sec::ALIAS, &wasm_vec(h, &items))
    };

    // sec 8 (first): canon-lower each aliased op (component func `i`) → core funcs `0..h`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..h {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h, &items))
    };

    // sec 2: TWO core instances — (0) the lowered ops exported under their names, forming the `"host"`
    // instance; (1) the program module instantiated with `"host"` bound to instance 0.
    let core_instance_sec = {
        let mut items = Vec::new();
        // instance 0: export-items form of the h lowered core funcs (indices 0..h), under the op names.
        let mut host = vec![0x01];
        let mut host_exports = Vec::new();
        for (i, f) in host_fns.iter().enumerate() {
            host_exports.extend_from_slice(&uleb_bytes(f.op.len() as u64));
            host_exports.extend_from_slice(f.op.as_bytes());
            host_exports.push(0x00); // ExportKind::Func
            uleb128(i as u64, &mut host_exports);
        }
        host.extend_from_slice(&wasm_vec(h, &host_exports));
        items.extend_from_slice(&host);
        // instance 1: instantiate module 0 with one arg `"host" = instance 0`.
        let mut prog = vec![0x00]; // instantiate form
        uleb128(0, &mut prog); // module index 0
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HOST_MODULE.len() as u64));
        args.extend_from_slice(HOST_MODULE.as_bytes());
        args.push(0x12); // ModuleArg::Instance sort
        uleb128(0, &mut args); // core instance 0
        prog.extend_from_slice(&wasm_vec(1, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(2, &items))
    };

    // sec 6 (second): alias each boundary func out of the PROGRAM instance (core instance 1) → core funcs
    // `h..h+m`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(1, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };

    // sec 7 (second): one component functype per boundary export → component types `1..=m`.
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            debug_assert!(
                e.result != BoundaryResult::Bytes,
                "a list<u8> boundary result takes the resource path, not the host shape"
            );
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };

    // sec 8 (second): lift each boundary core func (`h+j`) using its component type (`1+j`) → component
    // funcs `h..h+m`.
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((h + j) as u32, (1 + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };

    // sec 11: export each lifted component func (`h+j`) under its verbatim boundary name.
    let export_sec = {
        let mut items = Vec::new();
        for (j, e) in exports.iter().enumerate() {
            items.extend_from_slice(&comp_export_item(&e.name, (h + j) as u32));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: effect instance-type
    out.extend_from_slice(&import_sec); // 10: component import of the effect interface
    out.extend_from_slice(&op_alias_sec); // 6: alias ops out of the import
    out.extend_from_slice(&lower_sec); // 8: lower ops → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: embedded program
    out.extend_from_slice(&core_instance_sec); // 2: host-instance + program-instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&boundary_type_sec); // 7: boundary functypes
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&export_sec); // 11: export
    out
}

/// The core-module IMPORT module name the host-shape's lowered ops are threaded under (the twin of
/// `"heap"` for the runtime shape). The program's core module imports each host op from `"host"`.
const HOST_MODULE: &str = "host";

/// The SHARED-MEMORY core module the string-arg host shape threads: a one-page memory EXPORTED as `mem`,
/// nothing else. The program core module imports this memory (from module `"mem"`), and each string op's
/// canon-LOWER binds it so the `(ptr,len)` a `string` lowers to is read out of the SAME memory the
/// program's data segment wrote the string into. A separate module (not the program's own memory) breaks
/// the lower↔instance circularity: the memory instance exists BEFORE the lower that references it, which
/// exists before the program instance that imports it. Bytes: core magic + memory section (1 memory, min
/// 1) + export section (`mem` = memory 0).
fn shared_mem_module() -> Vec<u8> {
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01])); // limits {min:1}
    let export_sec = {
        let mut item = uleb_bytes("mem".len() as u64);
        item.extend_from_slice(b"mem");
        item.push(wasm_abi::EXPORT_KIND_MEMORY);
        uleb128(0, &mut item); // memory index 0
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(1, &item))
    };
    let mut out = Vec::new();
    out.extend_from_slice(wasm_abi::CORE_MAGIC);
    out.extend_from_slice(&mem_sec);
    out.extend_from_slice(&export_sec);
    out
}

/// The HOST-IMPORT + MEMORY shape (E2h-string): a program that delegates a single effect `iface` whose
/// operations take a `string` parameter. Adds, over [`assemble_host`], a shared-memory core module + its
/// instance + a memory alias, a Memory canon-option on each op's lower, and the program instance
/// instantiated with BOTH `"host"` (the lowered ops) and `"mem"` (the shared memory). Follows the
/// `ComponentBuilder` oracle's section order (verified byte-shape). SCOPE: host-only, single effect,
/// scalar/unit result, `string` or scalar params.
///
/// Index spaces (`m = exports.len()`): core memory `0` (the mem alias); lowered ops → core funcs `0..h`;
/// boundary core-aliases → core funcs `h..h+m`. Component: effect instance-type → type 0; imported effect
/// instance → comp instance 0; op aliases → comp funcs `0..h`; boundary functypes → types `1..=m`; lifts →
/// comp funcs `h..h+m`. Core instances: mem `0`, host-ops `1`, program `2`.
pub fn assemble_host_mem(
    core: &[u8],
    exports: &[BoundaryExport],
    iface: &str,
    host_fns: &[HostFn],
) -> Vec<u8> {
    let h = host_fns.len();
    let m = exports.len();

    // sec 7: the effect's instance-type — component type 0 (same as the scalar shape).
    let instance_type = {
        let mut decls = Vec::new();
        for (i, f) in host_fns.iter().enumerate() {
            decls.push(0x01);
            decls.extend_from_slice(&f.comp_functype);
            decls.push(0x04);
            decls.extend_from_slice(&extern_name(&f.op));
            decls.push(0x01);
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42];
        it.extend_from_slice(&wasm_vec(2 * h, &decls));
        it
    };
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type));

    // sec 10: import the effect interface as an instance of component type 0.
    let import_sec = {
        let mut item = extern_name(iface);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };

    // sec 6 (first): alias each op out of the imported effect instance (comp instance 0) → comp funcs.
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(0, &f.op));
        }
        section(sec::ALIAS, &wasm_vec(h, &items))
    };

    // sec 1 (first): the SHARED-MEMORY core module (module 0).
    let mem_module_sec = core_module_section(&shared_mem_module());
    // sec 2 (first): instantiate the mem module (no args) → core instance 0.
    let mem_instance_sec = section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[])),
    );
    // sec 6 (memory alias): alias `mem`.`mem` out of core instance 0 → core memory 0.
    let mem_alias_sec = section(sec::ALIAS, &wasm_vec(1, &memory_alias_item(0, "mem")));

    // sec 8 (first): canon-lower each aliased op with the MEMORY option (core memory 0) → core funcs.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..h {
            items.extend_from_slice(&canon_lower_item_mem(i as u32, 0));
        }
        section(sec::CANON, &wasm_vec(h, &items))
    };

    // sec 1 (second): the embedded program core module (module 1).
    let prog_module_sec = core_module_section(core);

    // sec 2 (second): TWO core instances — (1) the lowered ops as `"host"`, (2) the program instantiated
    // with `"host" = instance 1` AND `"mem" = instance 0`.
    let prog_instance_sec = {
        let mut items = Vec::new();
        // instance 1: the lowered ops exported under their names.
        let mut host = vec![0x01];
        let mut host_exports = Vec::new();
        for (i, f) in host_fns.iter().enumerate() {
            host_exports.extend_from_slice(&uleb_bytes(f.op.len() as u64));
            host_exports.extend_from_slice(f.op.as_bytes());
            host_exports.push(0x00);
            uleb128(i as u64, &mut host_exports);
        }
        host.extend_from_slice(&wasm_vec(h, &host_exports));
        items.extend_from_slice(&host);
        // instance 2: instantiate module 1 with `"host" = instance 1`, `"mem" = instance 0`.
        let mut prog = vec![0x00];
        uleb128(1, &mut prog); // module index 1 (the program)
        let mut args = Vec::new();
        // arg "host" = core instance 1
        args.extend_from_slice(&uleb_bytes(HOST_MODULE.len() as u64));
        args.extend_from_slice(HOST_MODULE.as_bytes());
        args.push(0x12);
        uleb128(1, &mut args);
        // arg "mem" = core instance 0
        args.extend_from_slice(&uleb_bytes("mem".len() as u64));
        args.extend_from_slice(b"mem");
        args.push(0x12);
        uleb128(0, &mut args);
        prog.extend_from_slice(&wasm_vec(2, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(2, &items))
    };

    // sec 6 (boundary alias): alias each boundary func out of the PROGRAM instance (core instance 2).
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(2, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };

    // sec 7 (second): one component functype per boundary export → comp types `1..=m`.
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };

    // sec 8 (second): lift each boundary core func (`h+j`) using its component type (`1+j`).
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((h + j) as u32, (1 + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };

    // sec 11: export each lifted component func under its boundary name.
    let export_sec = {
        let mut items = Vec::new();
        for (j, e) in exports.iter().enumerate() {
            items.extend_from_slice(&comp_export_item(&e.name, (h + j) as u32));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: effect instance-type
    out.extend_from_slice(&import_sec); // 10: component import of the effect interface
    out.extend_from_slice(&op_alias_sec); // 6: alias ops out of the import
    out.extend_from_slice(&mem_module_sec); // 1: shared-memory module (module 0)
    out.extend_from_slice(&mem_instance_sec); // 2: instantiate mem → core instance 0
    out.extend_from_slice(&mem_alias_sec); // 6: alias mem.mem → core memory 0
    out.extend_from_slice(&lower_sec); // 8: lower ops (with Memory option) → core funcs
    out.extend_from_slice(&prog_module_sec); // 1: embedded program (module 1)
    out.extend_from_slice(&prog_instance_sec); // 2: host-ops instance + program instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&boundary_type_sec); // 7: boundary functypes
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&export_sec); // 11: export
    out
}

/// The RESOURCE-ESCAPE shape — a compound leaves the component as a monomorphized component-model
/// RESOURCE `t` (rep i32, with a dtor) whose `encode() -> list<u8>` returns the canonical binary value
/// form, published inside the `cadenza:run/run` instance alongside `make : () -> own<t>`. This is the
/// escape path (`DESIGN-value-heap-rcdzc.md` §3a): the host holds a strongly-typed live handle and calls
/// `encode()` to render it (R3), rather than a raw `u32` crossing the boundary.
///
/// `main_core` is the program core module (imports `heap.resource-new`; exports `memory`,
/// `cabi_realloc`, `make`, `t-encode`); `dtor_core` is the STANDALONE dtor module (exports `t-dtor`,
/// imports nothing). Two core modules dissolve the resource↔dtor↔`resource.new` circular dependency
/// WITHOUT wit-bindgen's shim/fixup: the dtor module instantiates FIRST, so the resource type has a real
/// dtor core-func before `resource.new` (and hence `main_core`) needs the resource type
/// ([[rcdzc-r1-resource-encode-linking-findings]]). Byte-identical to the `ComponentBuilder`
/// `oracle_resource_component` (the R1 reference), hand-emitted section-by-section per that dump.
///
/// Index spaces: core funcs — dtor `0`, lowered `resource.new` `1`, `make` `2`, `t-encode` `3`,
/// `cabi_realloc` `4`; memory `0`; core instances — dtor `0`, heap `1`, program `2`. Outer component
/// types — resource `0`, `own<t>` `1`, make-ft `2`, `list u8` `3`, encode-ft `4`. Outer component funcs
/// — `make` (lift) `0`, `encode` (lift) `1`. The inner re-export component (its own index spaces) is a
/// raw nested blob ([`resource_inner_component`]) instantiated as component instance `0`, then exported
/// as the `cadenza:run/run` instance.
pub fn assemble_resource(main_core: &[u8], dtor_core: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 1: the standalone dtor core module (module 0) — imports nothing, so it instantiates first.
    out.extend_from_slice(&core_module_section(dtor_core));
    // sec 2: instantiate the dtor module (no args) → core instance 0.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[])),
    ));
    // sec 6: alias `t-dtor` out of core instance 0 → core func 0.
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(0, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func 0) → component type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(0)),
    ));
    // sec 8: canon `resource.new` for type 0 → core func 1 (the constructor `make` calls).
    out.extend_from_slice(&section(sec::CANON, &wasm_vec(1, &resource_new_item(0))));
    // sec 2: the `heap` core instance (export-items form) exporting `resource-new` = core func 1 → core
    // instance 1 (the instance `main_core` binds its `heap` import to).
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RESOURCE_NEW, 1)])),
    ));
    // sec 1: the program core module (module 1).
    out.extend_from_slice(&core_module_section(main_core));
    // sec 2: instantiate the program module (module 1) threading `heap` = core instance 1 → core
    // instance 2.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 1)])),
    ));
    // sec 6: alias the boundary exports off the program instance (core instance 2) — `make` = core func
    // 2, `t-encode` = core func 3, `memory` = memory 0, `cabi_realloc` = core func 4.
    let boundary_aliases = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(2, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(2, ENCODE_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(2, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(2, REALLOC_EXPORT));
        section(sec::ALIAS, &wasm_vec(4, &items))
    };
    out.extend_from_slice(&boundary_aliases);
    // sec 7: `own<t>` (type 1) then the `make` functype `() -> own<t>` (type 2).
    let make_types = {
        let mut items = own_item(0);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_types);
    // sec 8: lift `make` (core func 2) against functype type 2 → component func 0.
    out.extend_from_slice(&section(sec::CANON, &wasm_vec(1, &canon_lift_item(2, 2))));
    // sec 7: the shared `list u8` type (type 3) then the `encode` functype `(self: own<t>) -> list<u8>`
    // (type 4).
    let encode_types = {
        let mut items = list_u8_defined_type();
        items.extend_from_slice(&self_own_to_list_functype(1, 3));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&encode_types);
    // sec 8: lift `encode` (core func 3) against functype type 4, carrying Memory 0 + Realloc (core func
    // 4) → component func 1.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_list_item(3, 0, 4, 4)),
    ));
    // sec 4: the nested re-export component (its own header + sections) — the mechanism that converts
    // the internal rep-carrying resource identity into an exported abstract one.
    out.extend_from_slice(&component_section(&resource_inner_component()));
    // sec 5: instantiate the inner component (component 0) with the internal resource type (comp type 0)
    // + the two lifted funcs (comp funcs 0, 1) → component instance 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(1, &component_instantiate_item(0, 0, 1)),
    ));
    // sec 11: export the instantiated inner component as the `cadenza:run/run` instance.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(RUN_INTERFACE, 0)),
    ));
    out
}

/// The COMBINED runtime-import + resource escape shape (R2) — a RUNTIME compound (built on the value
/// heap, not a compile-time constant) leaves the component as a resource whose `encode()` WALKS the live
/// handle. It fuses the import shape's prologue (import the runtime `heap` interface, alias + lower the
/// used ops) with the resource shape (dtor module, resource type + `resource.new`/`resource.rep`, the
/// `heap` core-instance threading BOTH the lowered runtime ops AND the two resource intrinsics, the
/// program core, lift make/encode, inner re-export). `imports` is the program's sorted used-op set
/// (`k = imports.len()`), `import_name` the versioned runtime import. Byte-identical to the
/// `ComponentBuilder` combined oracle (`r2_runtime_resource::oracle_runtime_resource_component`); the
/// section sequence + index math were dumped from it and mirrored (the H1b method,
/// [[rcdzc-r1-resource-encode-linking-findings]]).
///
/// Index spaces (with `k = imports.len()`): core funcs — lowered ops `0..k`, `t-dtor` `k`,
/// `resource.new` `k+1`, `resource.rep` `k+2`, then the program's `make` `k+3`, `t-encode` `k+4`,
/// `cabi_realloc` `k+5` (the program core module imports the `k` runtime ops + `resource-new` +
/// `resource-rep` = `k+2` funcs before its own three). Component types — import-instance-type `0`,
/// resource `1`, `own<t>` `2`, make-ft `3`, `list u8` `4`, encode-ft `5`. Component funcs — aliased ops
/// `0..k`, make-lift `k`, encode-lift `k+1`. Core instances — dtor `0`, heap `1`, program `2`. The
/// program core module (`serialize::runtime_resource_core_module`) must import the `k` ops + the two
/// intrinsics in THIS order and export `memory`/`make`/`t-encode`/`cabi_realloc`.
pub fn assemble_runtime_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: the import instance-type declaring the k used runtime ops (component type 0). Identical to
    // the import shape's instance-type: 2k interleaved (ty, export) decls.
    let instance_type = {
        let mut decls = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            decls.push(0x01);
            decls.extend_from_slice(&op_comp_functype(op));
            decls.push(0x04);
            decls.extend_from_slice(&extern_name(op.name));
            decls.push(0x01);
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42];
        it.extend_from_slice(&wasm_vec(2 * k, &decls));
        it
    };
    out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type)));

    // sec 10: import the runtime interface as an instance of component type 0.
    let import_sec = {
        let mut item = extern_name(import_name);
        item.push(0x05); // ComponentTypeRef::Instance
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };
    out.extend_from_slice(&import_sec);

    // sec 6: alias each op out of the imported instance (component instance 0) → component funcs 0..k.
    let op_alias_sec = {
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    };
    out.extend_from_slice(&op_alias_sec);

    // sec 8: canon-lower each aliased op (component func i) → core funcs 0..k.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    };
    out.extend_from_slice(&lower_sec);

    // sec 2: the `heap-dtor` core instance (export-items form) exporting the lowered `drop` op (the core
    // func at `drop`'s sorted position in the used-set) as `drop` → core instance 0. The dtor module
    // imports `heap-dtor.drop` to release the resource's rep; sourcing `drop` from THIS small instance
    // (a plain lowered op, no resource intrinsic) — not the full `heap` instance — is what keeps the dtor
    // instantiable before the resource type, dissolving the resource↔dtor↔`resource.new` cycle.
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| i as u32)
        .expect("the runtime-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    // sec 1: the dtor core module (module 0) — imports `heap-dtor.drop`, calls it in `t-dtor`.
    out.extend_from_slice(&core_module_section(dtor_core));
    // sec 2: instantiate the dtor module threading `heap-dtor` = core instance 0 → core instance 1.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 0)])),
    ));
    // sec 6: alias `t-dtor` out of core instance 1 → core func k.
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(1, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func k) → component type 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(k as u32)),
    ));
    // sec 8: canon `resource.new` (→ core func k+1) AND `resource.rep` (→ core func k+2) for the resource
    // type — BOTH in one canon section (count 2), as the oracle emits them.
    let resource_canons = {
        let mut items = resource_new_item(1);
        items.extend_from_slice(&resource_rep_item(1));
        section(sec::CANON, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&resource_canons);
    // sec 2: the `heap` core instance (export-items form) exporting the k lowered ops (funcs 0..k) + the
    // two resource intrinsics (`resource-new` = core func k+1, `resource-rep` = core func k+2) → core
    // instance 2 (what `main_core` binds its `heap` import to).
    let heap_exports = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, i as u32))
            .collect();
        ex.push((RESOURCE_NEW, (k + 1) as u32));
        ex.push((RESOURCE_REP, (k + 2) as u32));
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    // sec 1: the program core module (module 1).
    out.extend_from_slice(&core_module_section(main_core));
    // sec 2: instantiate the program module (module 1) threading `heap` = core instance 2 → core
    // instance 3.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 2)])),
    ));
    // sec 6: alias the boundary exports off the program instance (core instance 3). Program core funcs
    // are shifted by the `k+2` imports (k ops + resource-new + resource-rep): `make` = core func k+3,
    // `t-encode` = k+4, `memory` = memory 0, `cabi_realloc` = k+5.
    let boundary_aliases = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(3, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(3, ENCODE_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(3, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(3, REALLOC_EXPORT));
        section(sec::ALIAS, &wasm_vec(4, &items))
    };
    out.extend_from_slice(&boundary_aliases);
    // sec 7: `own<t>` (type 2) then the `make` functype `() -> own<t>` (type 3). The resource is
    // component type 1 here (the import-instance-type is type 0), so `own` references type 1.
    let make_types = {
        let mut items = own_item(1);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(2)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_types);
    // sec 8: lift `make` (core func k+3) against functype type 3 → component func k.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((k + 3) as u32, 3)),
    ));
    // sec 7: the shared `list u8` type (type 4) then the `encode` functype `(self: own<t>) -> list<u8>`
    // (type 5). ⚠ `encode` takes `own<t>` (CONSUMES self) — this is why a runtime compound's handle
    // LEAKS: encode swallows the handle and the guest never drops it, so the dtor never fires. The
    // correct design is `borrow<t>` (host keeps ownership, drops afterward → dtor fires), but that
    // regresses the composed walk under wasmtime 37 with an un-root-caused host-side trap in encode
    // (resource.rep / borrow-lend interaction). Tracked as the R2-dtor follow-up in
    // [[rcdzc-r1-resource-encode-linking-findings]]; kept as `own` here to preserve a GREEN gate.
    let encode_types = {
        let mut items = list_u8_defined_type();
        items.extend_from_slice(&self_own_to_list_functype(2, 4));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&encode_types);
    // sec 8: lift `encode` (core func k+4) against functype type 5, carrying Memory 0 + Realloc (core
    // func k+5) → component func k+1.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((k + 4) as u32, 0, (k + 5) as u32, 5),
        ),
    ));
    // sec 4: the nested re-export component (its own local resource/func indices) — same blob as the
    // constant path's inner component.
    out.extend_from_slice(&component_section(&resource_inner_component()));
    // sec 5: instantiate the inner component (component 0) with the resource (comp type 1) + the two
    // lifted funcs (comp funcs k, k+1) → component instance 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(1, &component_instantiate_item(1, k as u32, (k + 1) as u32)),
    ));
    // sec 11: export the instantiated inner component as the `cadenza:run/run` instance. The runtime
    // IMPORT is component instance 0, so the inner re-export instantiation is component instance 1 (in
    // the constant shape, with no import, it is instance 0).
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(RUN_INTERFACE, 1)),
    ));
    out
}

/// Assemble a CLOSURE-RESOURCE component (C-HOST-1): a closure crossing the boundary as a resource whose
/// `call` method invokes it. The same runtime-import + resource shape as [`assemble_runtime_resource`],
/// but the second lifted method is `call : (self: own<t>, args…) -> R` (no `encode`, so no `list<u8>`
/// type, no Memory/Realloc canon options — a scalar-arg `call` needs no linear memory). Published as
/// `cadenza:closure/exports`. `main_core` is [`serialize::closure_resource_core_module`]'s output (which
/// exports `make` + `call`); `arg_bytes`/`result_byte` are the closure's boundary valtypes
/// (`AbiValType::comp_byte`). BYTE-IDENTICAL to the C-HOST-1 oracle
/// (`closure_host_resource::oracle_closure_component`).
///
/// Outer index spaces (k = imports.len()): lowered ops → core funcs 0..k; `t-dtor` → core func k;
/// `resource.new` → k+1, `resource.rep` → k+2; aliased `make` → k+3, `call` → k+4. The resource type is
/// component type 1 (the import-instance-type is type 0). Component funcs: aliased ops 0..k, then the
/// lifted `make` → comp func k, `call` → comp func k+1.
pub fn assemble_closure_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: the import instance-type declaring the k used runtime ops (component type 0).
    let instance_type = {
        let mut decls = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            decls.push(0x01);
            decls.extend_from_slice(&op_comp_functype(op));
            decls.push(0x04);
            decls.extend_from_slice(&extern_name(op.name));
            decls.push(0x01);
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42];
        it.extend_from_slice(&wasm_vec(2 * k, &decls));
        it
    };
    out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type)));

    // sec 10: import the runtime interface as an instance of component type 0.
    out.extend_from_slice(&{
        let mut item = extern_name(import_name);
        item.push(0x05); // ComponentTypeRef::Instance
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    });

    // sec 6: alias each op out of the imported instance → component funcs 0..k.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    });
    // sec 8: canon-lower each aliased op → core funcs 0..k.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    });

    // sec 2: `heap-dtor` core instance exporting the lowered `drop` (→ core instance 0); sec 1: dtor
    // module; sec 2: instantiate it (→ core instance 1); sec 6: alias `t-dtor` (→ core func k).
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| i as u32)
        .expect("the closure-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    out.extend_from_slice(&core_module_section(dtor_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 0)])),
    ));
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(1, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func k) → component type 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(k as u32)),
    ));
    // sec 8: canon `resource.new` (→ core func k+1) AND `resource.rep` (→ core func k+2).
    out.extend_from_slice(&{
        let mut items = resource_new_item(1);
        items.extend_from_slice(&resource_rep_item(1));
        section(sec::CANON, &wasm_vec(2, &items))
    });
    // sec 2: the `heap` core instance exporting the k lowered ops + resource-new (k+1) + resource-rep
    // (k+2) → core instance 2 (what `main_core` binds its `heap` import to).
    let heap_exports = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, i as u32))
            .collect();
        ex.push((RESOURCE_NEW, (k + 1) as u32));
        ex.push((RESOURCE_REP, (k + 2) as u32));
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    // sec 1: the program core module (module 1). sec 2: instantiate threading `heap` = core instance 2 →
    // core instance 3.
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 2)])),
    ));
    // sec 6: alias `make` + `call` off the program instance (core instance 3) → core funcs k+3, k+4. No
    // memory/realloc (a scalar-arg `call` needs no linear memory).
    out.extend_from_slice(&{
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(3, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(3, CALL_CORE_EXPORT));
        section(sec::ALIAS, &wasm_vec(2, &items))
    });
    // sec 7: `own<t>` (type 2) then the `make` functype `(export-params…) -> own<t>` (type 3). Resource
    // is comp type 1. A PARAMETERIZED export gives `make` those params (C-HOST-2); nullary gives `()`.
    out.extend_from_slice(&{
        let mut items = own_item(1);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(2)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 8: lift `make` (core func k+3) against functype type 3 → component func k.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((k + 3) as u32, 3)),
    ));
    // sec 7: `own<t>` (type 4) then the `call` functype `(self: own<t>, args…) -> R` (type 5). ⚠ `own<t>`
    // CONSUMES self per call (single-use per handle) — the `borrow<t>` migration for repeated calls is
    // C-HOST-5 (shared with the value-escape's `encode`).
    out.extend_from_slice(&{
        let mut items = own_item(1);
        items.extend_from_slice(&closure_call_functype(4, arg_bytes, result_byte));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 8: lift `call` (core func k+4) against functype type 5 → component func k+1. No canon options
    // (scalar args/result — no memory/realloc needed).
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((k + 4) as u32, 5)),
    ));
    // sec 4: the nested re-export component. sec 5: instantiate it (comp type 1 + comp funcs k, k+1) →
    // component instance 1 (the runtime import is component instance 0). sec 11: export as the closure
    // interface.
    out.extend_from_slice(&component_section(&resource_inner_component_closure(
        make_param_bytes,
        arg_bytes,
        result_byte,
    )));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(1, &component_instantiate_call_item(1, k as u32, (k + 1) as u32)),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(CLOSURE_INTERFACE, 1)),
    ));
    out
}

/// One closure export's boundary `make`, for the multi-export envelope: the name the host reaches it by
/// (`make-<def-name>`) + the core-module export name the program core exposes it under (the SAME string —
/// the serializer names the core export identically) + the export's parameter component bytes (`make`
/// forwards them). Parallel to [`serialize::ClosureMake`] but carrying the component-boundary bytes.
pub struct ClosureMakeAbi {
    pub name: String,
    pub make_param_bytes: Vec<u8>,
}

/// Assemble a MULTI-EXPORT closure-resource component: N `make-<name>` functions sharing ONE `call`,
/// published together under `cadenza:closure/exports`. The single-export [`assemble_closure_resource`] is
/// the N=1 case; this generalizes the make-related sections (alias, lift, functype, inner-component
/// import/export) to a loop over `makes`, keeping the ONE shared `call` (all exports share the closure
/// signature). Same runtime-import + resource shape as the single-export path. `main_core` is
/// [`serialize::multi_closure_resource_core_module`]'s output (exporting each `make-<name>` + `call`).
///
/// Outer index spaces (k = imports.len(), N = makes.len()): lowered ops → core funcs 0..k; `t-dtor` → k;
/// `resource.new` → k+1, `resource.rep` → k+2; aliased make[i] → k+3+i, `call` → k+3+N. Component funcs:
/// aliased ops 0..k, then lifted make[i] → comp func k+i, `call` → comp func k+N. Component types: 0 =
/// import instance-type, 1 = resource; then per make: `own<t>` + make-functype (types 2+2i, 3+2i); then
/// `own<t>` + call-functype (types 2+2N, 3+2N).
pub fn assemble_multi_closure_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let k = imports.len();
    let nmk = makes.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: the import instance-type declaring the k used runtime ops (component type 0).
    let instance_type = {
        let mut decls = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            decls.push(0x01);
            decls.extend_from_slice(&op_comp_functype(op));
            decls.push(0x04);
            decls.extend_from_slice(&extern_name(op.name));
            decls.push(0x01);
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42];
        it.extend_from_slice(&wasm_vec(2 * k, &decls));
        it
    };
    out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type)));

    // sec 10: import the runtime interface as an instance of component type 0.
    out.extend_from_slice(&{
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    });

    // sec 6: alias each op out of the imported instance → component funcs 0..k.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    });
    // sec 8: canon-lower each aliased op → core funcs 0..k.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    });

    // sec 2/1/2/6: dtor instance, module, instantiate, alias `t-dtor` → core func k.
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| i as u32)
        .expect("the closure-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    out.extend_from_slice(&core_module_section(dtor_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 0)])),
    ));
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(1, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func k) → component type 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(k as u32)),
    ));
    // sec 8: canon `resource.new` (→ core func k+1) AND `resource.rep` (→ core func k+2).
    out.extend_from_slice(&{
        let mut items = resource_new_item(1);
        items.extend_from_slice(&resource_rep_item(1));
        section(sec::CANON, &wasm_vec(2, &items))
    });
    // sec 2: the `heap` core instance exporting the k lowered ops + resource-new + resource-rep → core
    // instance 2 (what `main_core` binds its `heap` import to).
    let heap_exports = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, i as u32))
            .collect();
        ex.push((RESOURCE_NEW, (k + 1) as u32));
        ex.push((RESOURCE_REP, (k + 2) as u32));
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    // sec 1/2: the program core module (module 1); instantiate threading `heap` = core instance 2 → core
    // instance 3.
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 2)])),
    ));
    // sec 6: alias each `make-<name>` + `call` off the program instance (core instance 3) → core funcs
    // k+3..k+3+N (makes) then k+3+N (call).
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for mk in makes {
            items.extend_from_slice(&core_alias_item(3, &mk.name));
        }
        items.extend_from_slice(&core_alias_item(3, CALL_CORE_EXPORT));
        section(sec::ALIAS, &wasm_vec(nmk + 1, &items))
    });
    // sec 7: per make, `own<t>` + `make` functype `(export-params…) -> own<t>`; then `own<t>` + `call`
    // functype `(self: own<t>, args…) -> R`. Resource is comp type 1.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for (i, mk) in makes.iter().enumerate() {
            items.extend_from_slice(&own_item(1));
            let own_ty = (2 + 2 * i) as u32;
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(own_ty),
            ));
        }
        // the shared call's own<t> + functype.
        items.extend_from_slice(&own_item(1));
        let call_own_ty = (2 + 2 * nmk) as u32;
        items.extend_from_slice(&closure_call_functype(call_own_ty, arg_bytes, result_byte));
        section(sec::COMPONENT_TYPE, &wasm_vec(2 * (nmk + 1), &items))
    });
    // sec 8: lift each make (core func k+3+i) against its functype (type 3+2i) → comp func k+i; then lift
    // `call` (core func k+3+N) against the call functype (type 3+2N) → comp func k+N.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..nmk {
            let core_fn = (k + 3 + i) as u32;
            let functype = (3 + 2 * i) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        let call_core_fn = (k + 3 + nmk) as u32;
        let call_functype = (3 + 2 * nmk) as u32;
        items.extend_from_slice(&canon_lift_item(call_core_fn, call_functype));
        section(sec::CANON, &wasm_vec(nmk + 1, &items))
    });
    // sec 4/5/11: nested re-export component; instantiate it (resource type 1 + comp funcs k..k+N makes,
    // k+N call) → component instance 1; export as the closure interface.
    out.extend_from_slice(&component_section(&resource_inner_component_multi_closure(
        makes,
        arg_bytes,
        result_byte,
    )));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_multi_call_item(1, k as u32, nmk, makes),
        ),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(CLOSURE_INTERFACE, 1)),
    ));
    out
}

/// The nested RE-EXPORT component (a self-contained component blob, its own magic + sections). It
/// IMPORTS an abstract resource (`SubResource` bound) + the two funcs typed against it, then RE-EXPORTS
/// the resource DIRECTLY (no `SubResource` ascription — that would mint a fresh identity distinct from
/// the funcs' resource → "resource types are not the same") + the funcs re-typed against the exported
/// resource. This is the only way to export a resource-with-methods; the outer component instantiates it
/// with the real (rep-carrying) resource + lifted funcs. Inner index spaces: imported resource → type 0;
/// `own<0>` → type 1; make-ft → type 2; `list u8` → type 3; encode-ft → type 4; imported `make` → func
/// 0; imported `encode` → func 1; the RE-EXPORTED resource → type 5; `own<5>` → type 6; make-exp-ft →
/// type 7; `own<5>`,`list u8`,encode-exp-ft → types 8,9,10.
fn resource_inner_component() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource `import-type-t` (Type, SubResource bound) → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: `own<0>` (type 1) then the imported `make` functype `() -> own<0>` (type 2).
    let make_import_types = {
        let mut items = own_item(0);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_import_types);
    // sec 10: import `import-func-make` as a func of type 2 → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: `list u8` (type 3) then the imported `encode` functype `(self: own<0>) -> list<u8>` (type
    // 4).
    let encode_import_types = {
        let mut items = list_u8_defined_type();
        items.extend_from_slice(&self_own_to_list_functype(1, 3));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&encode_import_types);
    // sec 10: import `import-func-encode` as a func of type 4 → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-encode", 4)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY under the name `t` (no ascription — a
    // `SubResource` ascription would mint a fresh identity) → exported type 5.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: `own<5>` (type 6) then the `make` functype re-typed against the exported resource (type 7).
    let make_export_types = {
        let mut items = own_item(5);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(6)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_export_types);
    // sec 11: export `make` (func 0) ascribed to the exported functype 7.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, 7)),
    ));
    // sec 7: `own<5>` (type 8), `list u8` (type 9), then the `encode` functype re-typed against the
    // exported resource (type 10).
    let encode_export_types = {
        let mut items = own_item(5);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_own_to_list_functype(8, 9));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_export_types);
    // sec 11: export `encode` (func 1) ascribed to the exported functype 10.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(ENCODE_BOUNDARY_NAME, 1, 10)),
    ));
    out
}

/// The nested RE-EXPORT component for a CLOSURE resource — like [`resource_inner_component`] but the
/// second method is `call : (self: own<t>, args…) -> R` (invoke the closure) instead of `encode : (self:
/// own<t>) -> list<u8>`. Imports the abstract resource + `make`/`call` typed against it, re-exports the
/// resource DIRECTLY + both funcs ascribed against the exported identity — the only way to export a
/// resource-with-methods (the outer component instantiates it with the real rep-carrying resource + the
/// lifted funcs). `arg_bytes`/`result_byte` are the closure's boundary valtypes (`AbiValType::comp_byte`).
/// Inner index spaces: imported resource → type 0; `own<0>` → 1; make-ft `()->own<0>` → 2; `own<0>` → 3;
/// call-ft `(self:own<3>, args…)->R` → 4; imported `make` → func 0; imported `call` → func 1; RE-EXPORTED
/// resource → type 5; `own<5>` → 6; make-exp-ft → 7; `own<5>` → 8; call-exp-ft `(self:own<8>, args…)->R` →
/// 9. (No `list u8` type as the encode variant has — a `call`'s result is a scalar valtype inline.)
fn resource_inner_component_closure(
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: `own<0>` (type 1) then the imported `make` functype `(export-params…) -> own<0>` (type 2).
    let make_import_types = {
        let mut items = own_item(0);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_import_types);
    // sec 10: import `import-func-make` as a func of type 2 → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: `own<0>` (type 3) then the imported `call` functype `(self: own<3>, args…) -> R` (type 4).
    let call_import_types = {
        let mut items = own_item(0);
        items.extend_from_slice(&closure_call_functype(3, arg_bytes, result_byte));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&call_import_types);
    // sec 10: import `import-func-call` as a func of type 4 → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-call", 4)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY as `t` → exported type 5.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: `own<5>` (type 6) then the `make` functype re-typed against the exported resource (type 7).
    let make_export_types = {
        let mut items = own_item(5);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(6)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_export_types);
    // sec 11: export `make` (func 0) ascribed to functype 7.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, 7)),
    ));
    // sec 7: `own<5>` (type 8) then the `call` functype re-typed against the exported resource (type 9).
    let call_export_types = {
        let mut items = own_item(5);
        items.extend_from_slice(&closure_call_functype(8, arg_bytes, result_byte));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&call_export_types);
    // sec 11: export `call` (func 1) ascribed to functype 9.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(CALL_BOUNDARY_NAME, 1, 9)),
    ));
    out
}

/// The MULTI-EXPORT inner re-export component: imports the abstract resource + N `import-func-make-<i>`
/// (each `(export-params…) -> own<t>`) + one shared `import-func-call`, then re-exports the resource type
/// directly + each make under its boundary name (`makes[i].name`) + the shared `call`, all ascribed
/// against the exported resource identity. The N=1 case is byte-identical to
/// [`resource_inner_component_closure`]. Type-index layout (N = makes.len()): imported resource → type 0;
/// per make i: `own<0>` (1+2i), make functype (2+2i), imported func i; then `own<0>` (1+2N), call functype
/// (2+2N), imported func N. Exported resource → type R = 2N+3; per make i: `own<R>` (R+1+2i), make functype
/// (R+2+2i), exported func i; then `own<R>` (R+1+2N), call functype (R+2+2N), exported func N.
fn resource_inner_component_multi_closure(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let n = makes.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // Per make i: `own<0>` (type 1+2i) + make functype (type 2+2i); then import the func → func i.
    for (i, mk) in makes.iter().enumerate() {
        let own_ty = (1 + 2 * i) as u32;
        let ft_ty = (2 + 2 * i) as u32;
        out.extend_from_slice(&{
            let mut items = own_item(0);
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(own_ty),
            ));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_func_item(&format!("import-func-{}", mk.name), ft_ty)),
        ));
    }
    // Shared call: `own<0>` (type 1+2N) + call functype (type 2+2N); import the func → func N.
    let call_own_ty = (1 + 2 * n) as u32;
    let call_ft_ty = (2 + 2 * n) as u32;
    out.extend_from_slice(&{
        let mut items = own_item(0);
        items.extend_from_slice(&closure_call_functype(call_own_ty, arg_bytes, result_byte));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-call", call_ft_ty)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY as `t` → exported type R = 2N+3.
    let r = (2 * n + 3) as u32;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // Per make i: `own<R>` (R+1+2i) + make functype re-typed (R+2+2i); export func i under its name.
    for (i, mk) in makes.iter().enumerate() {
        let own_ty = r + (1 + 2 * i) as u32;
        let ft_ty = r + (2 + 2 * i) as u32;
        out.extend_from_slice(&{
            let mut items = own_item(r);
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(own_ty),
            ));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_func_ascribed_item(&mk.name, i as u32, ft_ty)),
        ));
    }
    // Shared call: `own<R>` (R+1+2N) + call functype re-typed (R+2+2N); export `call` (func N).
    let call_exp_own = r + (1 + 2 * n) as u32;
    let call_exp_ft = r + (2 + 2 * n) as u32;
    out.extend_from_slice(&{
        let mut items = own_item(r);
        items.extend_from_slice(&closure_call_functype(call_exp_own, arg_bytes, result_byte));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(CALL_BOUNDARY_NAME, n as u32, call_exp_ft),
        ),
    ));
    out
}

/// The nested RE-EXPORT component for the RUNTIME escape (R2) — like [`resource_inner_component`] but
/// `encode` takes `self: borrow<t>` (reads without consuming) instead of `own<t>`. The extra `borrow`
/// defined type shifts every type index after it by one vs the own variant. Inner index spaces:
/// imported resource → type 0; `own<0>` → 1; make-ft → 2; `borrow<0>` → 3; `list u8` → 4; encode-ft
/// `(self:borrow<0>)->list u8` → 5; imported `make` → func 0; imported `encode` → func 1; RE-EXPORTED
/// resource → type 6; `own<6>` → 7; make-exp-ft → 8; `borrow<6>` → 9, `list u8` → 10, encode-exp-ft → 11.
///
/// ⚠ NOT yet wired in: the `borrow<t>` encode is the correct R2-dtor fix (so the host keeps ownership
/// and drops → dtor fires), but it currently regresses the composed walk under wasmtime 37 with an
/// un-root-caused host-side trap in encode. Kept here (byte-layout worked out, byte-identity verified
/// against the ComponentBuilder borrow oracle) as scaffolding for the follow-up; the live path still
/// uses `own` ([[rcdzc-r1-resource-encode-linking-findings]]).
#[allow(dead_code)]
fn resource_inner_component_borrow() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource `import-type-t` (Type, SubResource bound) → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: `own<0>` (type 1) then the imported `make` functype `() -> own<0>` (type 2).
    let make_import_types = {
        let mut items = own_item(0);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_import_types);
    // sec 10: import `import-func-make` as a func of type 2 → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: `borrow<0>` (type 3), `list u8` (type 4), then the imported `encode` functype
    // `(self: borrow<0>) -> list<u8>` (type 5).
    let encode_import_types = {
        let mut items = borrow_item(0);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(3, 4));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_import_types);
    // sec 10: import `import-func-encode` as a func of type 5 → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-encode", 5)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY under the name `t` → exported type 6.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: `own<6>` (type 7) then the `make` functype re-typed against the exported resource (type 8).
    let make_export_types = {
        let mut items = own_item(6);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(7)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_export_types);
    // sec 11: export `make` (func 0) ascribed to the exported functype 8.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, 8)),
    ));
    // sec 7: `borrow<6>` (type 9), `list u8` (type 10), then the `encode` functype re-typed against the
    // exported resource (type 11).
    let encode_export_types = {
        let mut items = borrow_item(6);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(9, 10));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_export_types);
    // sec 11: export `encode` (func 1) ascribed to the exported functype 11.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(ENCODE_BOUNDARY_NAME, 1, 11)),
    ));
    out
}

/// The sec-4 nested-component bytes: `<id> <byte-length> <component>` — like [`core_module_section`] but
/// for a whole embedded component (its own magic + sections travel as a raw blob).
fn component_section(component: &[u8]) -> Vec<u8> {
    let mut out = vec![sec::COMPONENT];
    out.extend_from_slice(&uleb_bytes(component.len() as u64));
    out.extend_from_slice(component);
    out
}

/// A core-instance INSTANTIATE item: `00 <module-idx> <args-vec>`, each arg `<name> 0x12 <core-instance>`
/// (`0x12` = the ModuleArg::Instance / core-instance sort). Instantiate-arg names are BARE (no `0x00`
/// extern-name prefix), unlike component imports/exports.
fn core_instantiate_item(module_idx: u32, args: &[(&str, u32)]) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(module_idx as u64, &mut item);
    let mut arg_items = Vec::new();
    for (name, core_instance) in args {
        arg_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        arg_items.extend_from_slice(name.as_bytes());
        arg_items.push(0x12); // ModuleArg::Instance (core-instance sort)
        uleb128(*core_instance as u64, &mut arg_items);
    }
    item.extend_from_slice(&wasm_vec(args.len(), &arg_items));
    item
}

/// A core-instance EXPORT-ITEMS item: `01 <exports-vec>`, each export `<name> 0x00 <core-func>` (`0x00`
/// = ExportKind::Func). Forms an inline core instance from already-defined core funcs (here the lowered
/// `resource.new`, bound as the `heap` instance the program module imports).
fn core_export_instance_item(exports: &[(&str, u32)]) -> Vec<u8> {
    let mut item = vec![0x01]; // export-items form
    let mut export_items = Vec::new();
    for (name, core_func) in exports {
        export_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        export_items.extend_from_slice(name.as_bytes());
        export_items.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128(*core_func as u64, &mut export_items);
    }
    item.extend_from_slice(&wasm_vec(exports.len(), &export_items));
    item
}

/// A sec-7 RESOURCE-type item: `3f 7f 01 <dtor-core-func>` — resource-def tag `0x3f`, rep `i32`
/// (`CORE_I32`), has-dtor flag `0x01`, then the dtor core-func index. The `0x3f`/`0x01` are
/// component-model structural bytes `wasm-encoder` does not expose as constants; pinned by the R1
/// byte-identity oracle.
fn resource_type_item(dtor_core_func: u32) -> Vec<u8> {
    let mut item = vec![0x3f, wasm_abi::CORE_I32, 0x01];
    uleb128(dtor_core_func as u64, &mut item);
    item
}

/// A sec-8 canon `resource.new` item: `02 <resource-type>` — canon tag `0x02` (resource.new) then the
/// resource type index; lowers the constructor intrinsic to a core func (the guest calls it in `make` to
/// register a rep → an export-table handle).
fn resource_new_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x02];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A sec-8 canon `resource.rep` item: `04 <resource-type>` — canon tag `0x04` (resource.rep) then the
/// resource type index; lowers the rep-recovery intrinsic to a core func. `encode` calls it to turn the
/// resource-table HANDLE the canonical ABI hands it (an `own<t>` param crosses as a table index, NOT the
/// heap rep) back into the i32 heap rep the guest registered via `resource.new` — then walks that rep
/// ([[rcdzc-r1-resource-encode-linking-findings]] R2: without `resource.rep`, `arr-get` traps on the
/// small handle index, which `is_immediate` misreads as an inline value).
fn resource_rep_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x04];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A sec-7 `own<resource>` defined-type item: `69 <resource-type>` — the own-handle tag `0x69` then the
/// resource type index. A functype references a resource ONLY through an `own`/`borrow` handle, never the
/// resource type directly.
fn own_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x69];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A component VALTYPE referencing a defined type (an `own<…>` handle here) by index — encoded as the
/// bare type-index uleb (distinct from a primitive, which is its own negative-space byte).
fn owned_valtype(type_idx: u32) -> Vec<u8> {
    uleb_bytes(type_idx as u64)
}

/// A component functype with NO params and ONE result: `40 00 00 <result-valtype>` — functype form,
/// empty param vec, result-form `0x00` (one result), then the result valtype bytes. Used for `make : ()
/// -> own<t>`.
fn nullary_result_functype(result_valtype: &[u8]) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x00, 0x00];
    item.extend_from_slice(result_valtype);
    item
}

/// A component functype `(p0: <vt>, …) -> <result>` — form, a param vec of the given scalar valtype bytes
/// (named `p0`, `p1`, …), result-form `0x00` (one result), then the result valtype bytes. Used for a
/// PARAMETERIZED closure export's `make(export-params…) -> own<t>` (C-HOST-2); an empty `param_bytes`
/// reduces to the nullary shape. The result may be a DEFINED type (an `own<t>` handle) referenced by index
/// — pass its `owned_valtype(idx)` bytes.
fn params_result_functype(param_bytes: &[u8], result_valtype: &[u8]) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut params = Vec::new();
    for (i, &vt) in param_bytes.iter().enumerate() {
        let pname = format!("p{i}");
        params.extend_from_slice(&uleb_bytes(pname.len() as u64));
        params.extend_from_slice(pname.as_bytes());
        params.push(vt);
    }
    item.extend_from_slice(&wasm_vec(param_bytes.len(), &params));
    item.push(0x00); // one result
    item.extend_from_slice(result_valtype);
    item
}

/// A component functype `(self: own<t>) -> list<u8>` — form, one param named `self` of type
/// `own<own_type_idx>`, one `list<u8>` result. Used by the CONSTANT escape (R1), whose resource carries
/// no live heap handle, so consuming it in `encode` leaks nothing.
fn self_own_to_list_functype(own_type_idx: u32, list_type_idx: u32) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x01];
    item.extend_from_slice(&uleb_bytes("self".len() as u64));
    item.extend_from_slice(b"self");
    item.extend_from_slice(&owned_valtype(own_type_idx));
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A sec-7 `borrow<resource>` defined-type item: `68 <resource-type>` — the borrow-handle tag `0x68`
/// then the resource type index. `encode` takes a BORROW (reads self without consuming), so the caller
/// keeps ownership and drops the handle afterward — which fires the dtor. (An `own` self would move the
/// handle into `encode`, which then leaks it.)
#[allow(dead_code)]
fn borrow_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x68];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A component functype `(self: borrow<t>) -> list<u8>`: `40 01 <"self"> <borrow-valtype> 00 <list-type>`
/// — form, one param named `self` of type `borrow<borrow_type_idx>` (a DEFINED type by index),
/// result-form `0x00` (one result), the list defined-type index. Used for `encode`. `borrow_type_idx` is
/// the component-type index of the `borrow<t>` defined type (laid just before the functype).
#[allow(dead_code)]
fn self_borrow_to_list_functype(borrow_type_idx: u32, list_type_idx: u32) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x01];
    item.extend_from_slice(&uleb_bytes("self".len() as u64));
    item.extend_from_slice(b"self");
    item.extend_from_slice(&owned_valtype(borrow_type_idx));
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A component functype for a CLOSURE-RESOURCE `call` method: `(self: <handle<t>>, p0: <vt>, …) -> <vt>`
/// — form `0x40`, then the param vec `[self : own/borrow<self_type_idx>, p0.., …]`, then the result form.
/// `self` is the receiver (an `own`/`borrow` handle to the closure resource — a DEFINED type by index);
/// the remaining params are the closure's argument valtypes and the result is its return valtype, both
/// scalar `AbiValType::comp_byte`s (the aliased boundary widths). This is the closure analog of
/// `self_borrow_to_list_functype` — a method whose body does `resource.rep(self)` then a `call_indirect`
/// on the recovered cell (C-HOST-1). `arg_bytes`/`result_byte` are `AbiValType::comp_byte()` values.
/// (C-HOST-0: emitted + oracle-checked; not yet wired into an assembled component.)
#[allow(dead_code)]
fn closure_call_functype(self_handle_type_idx: u32, arg_bytes: &[u8], result_byte: u8) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    // `self` — the receiver handle (own/borrow<t>), a defined type referenced by index.
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    // The closure's arguments, named `p0`, `p1`, … (positional at a boundary call; the names are cosmetic).
    for (i, &vt) in arg_bytes.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(1 + arg_bytes.len(), &param_items));
    // One result — the closure's return valtype (a scalar boundary byte).
    item.extend_from_slice(&[0x00, result_byte]);
    item
}

/// A sec-10 component-import item for an abstract RESOURCE: `<extern-name> 03 01` — `0x03` =
/// ComponentTypeRef::Type, `0x01` = TypeBounds::SubResource (mint a fresh abstract resource the importer
/// binds).
fn import_subresource_item(name: &str) -> Vec<u8> {
    let mut item = extern_name(name);
    item.push(0x03); // ComponentTypeRef::Type
    item.push(0x01); // TypeBounds::SubResource
    item
}

/// A sec-10 component-import item for a FUNC: `<extern-name> 01 <type-idx>` — `0x01` =
/// ComponentTypeRef::Func, then the functype index.
fn import_func_item(name: &str, type_idx: u32) -> Vec<u8> {
    let mut item = extern_name(name);
    item.push(0x01); // ComponentTypeRef::Func
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-11 export item RE-EXPORTING a TYPE directly: `00 <name> 03 <type-idx> 00` — extern-name, sort
/// type `0x03`, the type index, no outer-type ascription (`0x00`). A direct re-export publishes the
/// imported resource's identity unchanged; an ascription would mint a fresh, incompatible identity.
fn export_type_direct_item(name: &str, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x03); // sort: type
    uleb128(type_idx as u64, &mut item);
    item.push(0x00); // no outer-type ascription
    item
}

/// A sec-11 export item for a FUNC WITH an outer-type ascription: `00 <name> 01 <func-idx> 01 01
/// <type-idx>` — extern-name, sort func `0x01`, the func index, ascription-present `0x01`, then a
/// ComponentTypeRef::Func (`0x01`) + the functype index. The ascription re-types the imported func
/// against the EXPORTED resource identity.
fn export_func_ascribed_item(name: &str, func_idx: u32, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x01); // sort: component func
    uleb128(func_idx as u64, &mut item);
    item.push(0x01); // outer-type ascription present
    item.push(0x01); // ComponentTypeRef::Func
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-5 component-INSTANTIATE item wiring the resource re-export component's imports: `00 <component>
/// <args-vec>` with the three args — `import-type-t` = the internal resource (comp type `res_ty`),
/// `import-func-make` = the lifted `make` (comp func `make_fn`), `import-func-encode` = the lifted
/// `encode` (comp func `encode_fn`). The constant escape wires `(0, 0, 1)` (no ops precede); the runtime
/// escape wires `(1, k, k+1)` (the import-instance-type is comp type 0 + the `k` aliased ops precede the
/// lifts). Instantiate-arg names are BARE (no `0x00` prefix); the sort byte is `0x03` (type) / `0x01`
/// (func).
fn component_instantiate_item(res_ty: u32, make_fn: u32, encode_fn: u32) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (always component 0 in both shapes)
    let args: [(&str, u8, u32); 3] = [
        ("import-type-t", 0x03, res_ty), // Type → internal resource comp type
        ("import-func-make", 0x01, make_fn), // Func → lifted make comp func
        ("import-func-encode", 0x01, encode_fn), // Func → lifted encode comp func
    ];
    let mut arg_items = Vec::new();
    for (name, sort, idx) in args {
        arg_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        arg_items.extend_from_slice(name.as_bytes());
        arg_items.push(sort);
        uleb128(idx as u64, &mut arg_items);
    }
    item.extend_from_slice(&wasm_vec(args.len(), &arg_items));
    item
}

/// Like [`component_instantiate_item`] but for the CLOSURE inner component: the second imported func is
/// `import-func-call` (the `call` method), not `import-func-encode`.
fn component_instantiate_call_item(res_ty: u32, make_fn: u32, call_fn: u32) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (component 0)
    let args: [(&str, u8, u32); 3] = [
        ("import-type-t", 0x03, res_ty),
        ("import-func-make", 0x01, make_fn),
        ("import-func-call", 0x01, call_fn),
    ];
    let mut arg_items = Vec::new();
    for (name, sort, idx) in args {
        arg_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        arg_items.extend_from_slice(name.as_bytes());
        arg_items.push(sort);
        uleb128(idx as u64, &mut arg_items);
    }
    item.extend_from_slice(&wasm_vec(args.len(), &arg_items));
    item
}

/// The MULTI-EXPORT instantiate item: supply the resource type + N make funcs (`import-func-make-<i>` →
/// comp func `first_make_fn + i`) + the shared `call` (`import-func-call` → comp func `first_make_fn +
/// N`). The inner component ([`resource_inner_component_multi_closure`]) imports under these same names.
fn component_instantiate_multi_call_item(
    res_ty: u32,
    first_make_fn: u32,
    nmk: usize,
    makes: &[ClosureMakeAbi],
) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (component 0)
    let mut arg_items = Vec::new();
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    push("import-type-t", 0x03, res_ty, &mut arg_items);
    for (i, mk) in makes.iter().enumerate() {
        push(
            &format!("import-func-{}", mk.name),
            0x01,
            first_make_fn + i as u32,
            &mut arg_items,
        );
    }
    push(
        "import-func-call",
        0x01,
        first_make_fn + nmk as u32,
        &mut arg_items,
    );
    item.extend_from_slice(&wasm_vec(1 + nmk + 1, &arg_items));
    item
}

/// A sec-11 export item for an INSTANCE: `00 <name> 05 <instance-idx> 00` — extern-name, sort
/// component-instance `0x05`, the instance index, no type ascription. Publishes the instantiated
/// re-export component as the well-known `cadenza:run/run` interface.
fn export_instance_item(name: &str, instance_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x05); // sort: component instance
    uleb128(instance_idx as u64, &mut item);
    item.push(0x00); // no type ascription
    item
}

/// The sec-1 embedded-core-module bytes: `<id> <byte-length> <core>` (the module is a raw blob, not a
/// wasm_vec of items).
fn core_module_section(core: &[u8]) -> Vec<u8> {
    let mut out = vec![sec::CORE_MODULE];
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(core);
    out
}

/// A component-model extern name (import name / instance-type export name): a `0x00` prefix, then the
/// length-prefixed UTF-8 bytes.
fn extern_name(name: &str) -> Vec<u8> {
    let mut out = vec![0x00];
    out.extend_from_slice(&uleb_bytes(name.len() as u64));
    out.extend_from_slice(name.as_bytes());
    out
}

/// A component functype `0x40 <params-vec> <result-form>` for a runtime OP, using its COMPONENT
/// valtype bytes (`AbiValType::comp_byte` — a `u32` handle is `0x79`, distinct from its core i32).
/// Params are NAMED at the boundary (a positional call ignores the name → synthesized `p0`, `p1`, …).
fn op_comp_functype(op: &RtOp) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, ty) in op.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(ty.comp_byte());
    }
    item.extend_from_slice(&wasm_vec(op.params.len(), &param_items));
    match op.result {
        Some(ty) => item.extend_from_slice(&[0x00, ty.comp_byte()]),
        None => item.extend_from_slice(&[0x01, 0x00]),
    }
    item
}

/// A sec-7 component functype item for a BOUNDARY export: `<func:0x40> <params-vec> <result-form>`. The
/// params vec is `<count> (<name> <valtype>)*` — each parameter NAMED (synthesized `p0`, `p1`, …). The
/// result form is `00 <valtype>` for one result (a primitive's own byte, or a DEFINED type by index for
/// a `list<u8>`), `01 00` for none. `list_type_idx` is the component-type index of the shared `list u8`
/// defined type, referenced when the result is [`BoundaryResult::Bytes`].
fn comp_functype(e: &BoundaryExport, list_type_idx: u32) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM]; // function type form
    let mut param_items = Vec::new();
    for (i, &vt) in e.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(e.params.len(), &param_items));
    match e.result {
        // A primitive result is its own valtype byte, inline; a `list<u8>` result references the shared
        // defined type by index (both under result-form `0x00` = "one result").
        BoundaryResult::Primitive(vt) => item.extend_from_slice(&[0x00, vt]),
        BoundaryResult::Bytes => {
            item.push(0x00);
            uleb128(list_type_idx as u64, &mut item);
        }
        BoundaryResult::None => item.extend_from_slice(&[0x01, 0x00]),
    }
    item
}

/// A sec-6 CORE-func alias item (alias a core-instance export): `00 00 01 <instance> <namelen> <name>`
/// — core-sort `0x00`, core-func-kind `0x00`, alias-target core-instance-export `0x01`, the instance
/// index, then the export name.
fn core_alias_item(instance: u32, name: &str) -> Vec<u8> {
    let mut item = vec![0x00, 0x00, 0x01];
    uleb128(instance as u64, &mut item);
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item
}

/// A sec-6 COMPONENT-func alias item (alias a component-instance export): `01 00 <instance> <namelen>
/// <name>` — component-func sort `0x01`, alias-target component-instance-export `0x00`, the instance
/// index, then the export name.
fn comp_alias_item(instance: u32, name: &str) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(instance as u64, &mut item);
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item
}

/// A sec-8 canon-lower item: `01 00 <comp-func> 00` — `01 00` canon lower a component func, the
/// component func index, then `00` empty canon-options.
fn canon_lower_item(comp_func: u32) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(comp_func as u64, &mut item);
    item.push(0x00); // canon options: none
    item
}

/// A sec-8 canon-lower item WITH a MEMORY option: `01 00 <comp-func> 01 03 <mem-idx>` — lower a component
/// func, then a one-option canon-options vec carrying `Memory(mem-idx)` (tag `0x03`). A host op with a
/// STRING parameter needs the memory the `(ptr,len)` lowering reads the string from, so its lower binds
/// the shared memory (the `0x03` Memory tag is the same component-model canon-opt encoding
/// `canon_lift_list_item` uses, pinned by the byte-identity oracle).
fn canon_lower_item_mem(comp_func: u32, mem_idx: u32) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(comp_func as u64, &mut item);
    item.push(0x01); // canon options: count 1
    item.push(0x03); // CanonicalOption::Memory
    uleb128(mem_idx as u64, &mut item);
    item
}

/// A sec-8 canon-lift item: `00 00 <core-func> 00 <type>` — `00 00` canon lift core func, `00` empty
/// canon-options, then the component type index.
fn canon_lift_item(core_func: u32, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00, 0x00];
    uleb128(core_func as u64, &mut item);
    item.push(0x00); // canon options: none
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-8 canon-lift item for a `list<u8>`-returning boundary func: `00 00 <core-func> <opts> <type>`,
/// where the canon-options vec carries the MEMORY and REALLOC the canonical ABI needs to read the
/// `(ptr, len)` return area out of the core module's linear memory (`00 00 <core-func> 02 03 <mem-idx>
/// 04 <realloc-func-idx> <type>`). Option tags: `0x03 <mem-idx>` = Memory, `0x04 <core-func-idx>` =
/// Realloc — the exact byte shape the `ComponentBuilder` oracle emits for `CanonicalOption::Memory` +
/// `::Realloc` (pinned by the R0 byte-identity test; the tags are component-model canon-opt encodings
/// `wasm-encoder` does not expose as public constants). Options are ordered Memory-then-Realloc.
fn canon_lift_list_item(core_func: u32, mem_idx: u32, realloc_func: u32, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00, 0x00];
    uleb128(core_func as u64, &mut item);
    // canon options vec: count 2, then Memory then Realloc.
    item.push(0x02);
    item.push(0x03); // CanonicalOption::Memory
    uleb128(mem_idx as u64, &mut item);
    item.push(0x04); // CanonicalOption::Realloc
    uleb128(realloc_func as u64, &mut item);
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-6 MEMORY alias item (alias a core-instance's exported memory): `00 02 01 <instance> <namelen>
/// <name>` — core-sort `0x00`, core-MEMORY-kind `0x02`, alias-target core-instance-export `0x01`, the
/// instance index, then the export name. The `0x02` (memory) kind is the only difference from
/// [`core_alias_item`] (a func alias, kind `0x00`); pinned by the R0 byte-identity oracle.
fn memory_alias_item(instance: u32, name: &str) -> Vec<u8> {
    let mut item = vec![0x00, 0x02, 0x01];
    uleb128(instance as u64, &mut item);
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item
}

/// The sec-7 defined-type item for `list<u8>`: `70 7d` — the component-model `list` defined-type tag
/// `0x70` followed by the element valtype (`u8` = `wasm_abi::COMP_U8`). It is the canonical binary value
/// form's boundary type (the resource `encode()` return); shared by every `list<u8>`-returning export,
/// laid at a fixed component-type index. The `0x70` list tag is a component-model structural encoding
/// `wasm-encoder` does not expose as a constant, pinned by the R0 byte-identity oracle.
fn list_u8_defined_type() -> Vec<u8> {
    vec![0x70, wasm_abi::COMP_U8]
}

/// A sec-11 component-export item: `00 <namelen><name> 01 <func-idx> 00` — name, sort component
/// func:0x01, the func index, no declared type ascription.
fn comp_export_item(name: &str, func_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x01); // sort: component func
    uleb128(func_idx as u64, &mut item);
    item.push(0x00); // no declared type ascription
    item
}

#[cfg(test)]
mod closure_resource_tests {
    use super::*;

    /// C-HOST-0 (byte-neutral probe): the `call`-method functype encodes to the exact component-model
    /// bytes — form `0x40`, a param vec `[self : own<t>, p0 : s64]`, result `00 <s64>`. `self` references
    /// the resource handle (a DEFINED type by index) as a bare type-index uleb; the arg + result are the
    /// scalar boundary bytes (`AbiValType::comp_byte`: `s64` = `0x78`). Deep byte-identity against a full
    /// `ComponentBuilder` component arrives in C-HOST-1 (when the `own<t>` index is real); this pins the
    /// item shape now so the wiring increment builds on a checked primitive.
    #[test]
    fn closure_call_functype_encodes_the_call_method_shape() {
        use crate::backend::wasm::runtime_abi::AbiValType;
        let s64 = AbiValType::S64.comp_byte(); // 0x78
        // A `(-> Int64 Int64)` closure whose resource handle is defined type index 5: one arg + result.
        let got = closure_call_functype(5, &[s64], s64);
        let want: Vec<u8> = vec![
            wasm_abi::COMP_FUNCTYPE_FORM, // 0x40 functype form
            0x02, // param count = 2 (self + p0)
            0x04, b's', b'e', b'l', b'f', // "self"
            0x05, // own<t> defined type, index 5 (bare uleb)
            0x02, b'p', b'0', // "p0"
            s64,  // arg valtype s64
            0x00, // result form: one result
            s64,  // result valtype s64
        ];
        assert_eq!(got, want, "call-method functype byte shape");

        // A two-arg closure `(-> Int64 (-> Int64 Int64))` flattens to two `call` params after `self`.
        let got2 = closure_call_functype(7, &[s64, s64], s64);
        let want2: Vec<u8> = vec![
            wasm_abi::COMP_FUNCTYPE_FORM,
            0x03, // self + p0 + p1
            0x04, b's', b'e', b'l', b'f',
            0x07, // own<t> index 7
            0x02, b'p', b'0', s64,
            0x02, b'p', b'1', s64,
            0x00, s64,
        ];
        assert_eq!(got2, want2, "two-arg call-method functype byte shape");
    }
}
