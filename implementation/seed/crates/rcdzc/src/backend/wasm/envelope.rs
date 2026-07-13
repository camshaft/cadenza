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
/// The core-module export name for the scalar `len` method body (VM-1) — a `bytes-len`/`vec-len` over the
/// borrow rep, aliased off the program instance alongside `make`/`t-encode`.
const LEN_CORE_EXPORT: &str = "t-len";
/// The `len` method's boundary name (inside [`RUN_INTERFACE`]).
const LEN_BOUNDARY_NAME: &str = "len";
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
            decls.push(0x04); // export decl — the op's COMPONENT extern name (kebab-normalized).
            decls.extend_from_slice(&extern_name(&super::kebab_extern_name(&f.op)));
            decls.push(0x01); // sort: component func
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42]; // instance type form
        it.extend_from_slice(&wasm_vec(2 * h, &decls));
        it
    };
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type));

    // sec 10: import the effect interface as an instance of component type 0, under the effect's name —
    // KEBAB-normalized (a non-kebab effect name like `Log` is not a valid component import extern name).
    let import_sec = {
        let mut item = extern_name(&super::kebab_extern_name(iface));
        item.push(0x05); // ComponentTypeRef::Instance sort
        uleb128(0, &mut item); // type index 0
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };

    // sec 6 (first): alias each op out of the imported effect instance (component instance 0) → component
    // funcs `0..h`. The alias reads the op by the COMPONENT extern name the instance-type exports it under
    // (the kebab-normalized op name), so it must match the export decl above.
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(0, &super::kebab_extern_name(&f.op)));
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
            decls.extend_from_slice(&extern_name(&super::kebab_extern_name(&f.op)));
            decls.push(0x01);
            uleb128(i as u64, &mut decls);
        }
        let mut it = vec![0x42];
        it.extend_from_slice(&wasm_vec(2 * h, &decls));
        it
    };
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type));

    // sec 10: import the effect interface as an instance of component type 0 (kebab-normalized name).
    let import_sec = {
        let mut item = extern_name(&super::kebab_extern_name(iface));
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };

    // sec 6 (first): alias each op out of the imported effect instance (comp instance 0) → comp funcs. The
    // alias name is the kebab-normalized op name the instance-type export decl uses (they must match).
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(0, &super::kebab_extern_name(&f.op)));
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
/// resource `1`, `own<t>` `2`, make-ft `3`, then `borrow<t>` `4`, `list u8` `5`, encode-ft `6`. Component
/// funcs — aliased ops `0..k`, make-lift `k`, encode-lift `k+1`. Core instances — dtor `0`, heap `1`,
/// program `2`. The
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
    // sec 7: `borrow<t>` (type 4), the shared `list u8` type (type 5), then the `encode` functype
    // `(self: borrow<t>) -> list<u8>` (type 6). `encode` BORROWS self — the host keeps ownership and
    // drops the handle afterward (firing the dtor, which reclaims the heap rep). The core `t-encode`
    // receives the borrow's REP DIRECTLY as its param (wasmtime's `lift_borrow` passes the rep, not a
    // table index), so it walks the heap without `resource.rep` and does NOT drop — the value survives
    // the call, making the method repeatable ([[rcdzc-r1-resource-encode-linking-findings]], the
    // 2026-07-13 borrow correction; proven by `a_borrow_self_encode_walks_and_crosses`).
    let encode_types = {
        let mut items = borrow_item(1); // borrow<resource> — resource is component type 1
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(4, 5));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_types);
    // sec 8: lift `encode` (core func k+4) against functype type 6, carrying Memory 0 + Realloc (core
    // func k+5) → component func k+1.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((k + 4) as u32, 0, (k + 5) as u32, 6),
        ),
    ));
    // sec 4: the nested re-export component — the BORROW variant (re-types `encode` against
    // `borrow<t>`), matching the borrow lift above.
    out.extend_from_slice(&component_section(&resource_inner_component_borrow()));
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

/// Assemble a runtime-resource component (VM-1) that carries `make` + `encode` PLUS a scalar
/// `len : borrow<t> -> u32` method — the value-resource-with-methods shape for a String/Bytes/List whose
/// length the host can query without decoding the whole value form. Identical to
/// [`assemble_runtime_resource`] through the boundary aliases, then it lifts a THIRD method: the program
/// core module exports `t-len` (a `bytes-len`/`vec-len` over the borrow rep, a scalar-result borrow method
/// with NO Memory/Realloc canon options), aliased at core func `k+6` (after make `k+3`, t-encode `k+4`,
/// cabi_realloc `k+5`), lifted against a fresh `borrow<t>` + `self_borrow_to_scalar_functype`. The inner
/// re-export component is [`resource_inner_component_borrow_len`], which re-exports make/encode/len.
/// BYTE-IDENTICAL to `tests::r2_runtime_resource::oracle_tuple_methods` (the proven ComponentBuilder
/// reference). Index spaces (k = imports.len()): as `assemble_runtime_resource` plus — core func `t-len` =
/// `k+6`; component types after encode-ft `6`: `borrow<t>` `7`, len-ft `8`; component funcs: make-lift `k`,
/// encode-lift `k+1`, len-lift `k+2`.
pub fn assemble_runtime_resource_with_len(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    // `len` is the single scalar method `len : borrow<t> -> u32`; the generic path is the one hand-emit.
    assemble_runtime_resource_with_scalar_methods(
        main_core,
        dtor_core,
        imports,
        import_name,
        &[ScalarMethod {
            boundary_name: LEN_BOUNDARY_NAME,
            core_export: LEN_CORE_EXPORT,
            result: MethodResult::Scalar(wasm_abi::COMP_U32),
        }],
    )
}

/// The RESULT shape of a value-resource method — what the boundary form + canon-lift options are.
#[derive(Clone, Copy)]
pub enum MethodResult {
    /// A primitive scalar (e.g. `u32` for `len`) — crosses by value, NO Memory/Realloc canon options.
    Scalar(u8),
    /// A `list<u8>` (e.g. `to-bytes`) — crosses through linear memory by the canonical ABI, so its lift
    /// carries Memory 0 + Realloc, exactly like `encode`. Reuses encode's `borrow<t>` + `list<u8>` defined
    /// types (both identity-free), so it lays only its own functype.
    ListU8,
}

/// A value-resource METHOD beyond make+encode: its boundary name (e.g. `"len"`/`"to-bytes"`), the core
/// module's export name for its body (e.g. `"t-len"`/`"t-to-bytes"`), and its result shape. All methods
/// are `borrow<t>` (repeatable). A `Scalar` result needs no Memory/Realloc; a `ListU8` result does (like
/// encode). (Kept named `ScalarMethod` historically — now carries a `MethodResult` for both kinds.)
#[derive(Clone, Copy)]
pub struct ScalarMethod {
    pub boundary_name: &'static str,
    pub core_export: &'static str,
    pub result: MethodResult,
}

/// Assemble a runtime-resource component carrying make + encode + N extra SCALAR borrow methods (VM-1/VM-2).
/// The generalization of [`assemble_runtime_resource`] (N=0) and [`assemble_runtime_resource_with_len`]
/// (N=1, `len`). Each scalar method appends: a boundary alias of its `t-<name>` core export (at core func
/// `k+6+i`, in method order, AFTER make k+3/t-encode k+4/cabi_realloc k+5), a functype `(self: borrow<t>)
/// -> <prim>` REUSING the `borrow<t>` defined type 4 laid for encode (identity-free — the ComponentBuilder
/// oracle reuses it), a scalar `canon lift` (no Memory/Realloc), and a re-export in the inner component.
/// BYTE-IDENTICAL to the ComponentBuilder oracle (the N=1 case is gated by
/// `len_method_envelope_matches_component_builder_oracle`; N=0 by `combined_envelope_matches_...`). The
/// program core module must export `make`/`t-encode`/`cabi_realloc` + each method's `t-<name>` (a scalar
/// borrow body, its param IS the rep). Component type/func indices are documented inline.
pub fn assemble_runtime_resource_with_scalar_methods(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    methods: &[ScalarMethod],
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // Sections through the boundary aliases are IDENTICAL to `assemble_runtime_resource` (the shared
    // prologue: import instance-type, runtime import, op aliases + lowers, dtor instance/module/instance,
    // t-dtor alias, resource type, resource.new/rep canons, heap instance, program module/instance).
    // Re-emitted here rather than factored to keep each hand-emit a single auditable byte stream.
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
    let import_sec = {
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };
    out.extend_from_slice(&import_sec);
    let op_alias_sec = {
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    };
    out.extend_from_slice(&op_alias_sec);
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    };
    out.extend_from_slice(&lower_sec);
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| i as u32)
        .expect("the runtime-resource escape imports `drop` for the dtor");
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
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(k as u32)),
    ));
    let resource_canons = {
        let mut items = resource_new_item(1);
        items.extend_from_slice(&resource_rep_item(1));
        section(sec::CANON, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&resource_canons);
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
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 2)])),
    ));
    // sec 6: alias the boundary exports off the program instance (core instance 3): make k+3, t-encode
    // k+4, memory, cabi_realloc k+5, THEN each scalar method's `t-<name>` (core func k+6+i, in method
    // order). Alias order fixes the core-func indices the lifts reference.
    let boundary_aliases = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(3, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(3, ENCODE_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(3, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(3, REALLOC_EXPORT));
        for m in methods {
            items.extend_from_slice(&core_alias_item(3, m.core_export));
        }
        section(sec::ALIAS, &wasm_vec(4 + methods.len(), &items))
    };
    out.extend_from_slice(&boundary_aliases);
    // sec 7 + 8: make (types 2,3 → comp func k) and encode (types 4,5,6 → comp func k+1) — IDENTICAL to
    // `assemble_runtime_resource`.
    let make_types = {
        let mut items = own_item(1);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(2)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_types);
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((k + 3) as u32, 3)),
    ));
    let encode_types = {
        let mut items = borrow_item(1);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(4, 5));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_types);
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((k + 4) as u32, 0, (k + 5) as u32, 6),
        ),
    ));
    // Per method `i`: a functype `(self: borrow<t>) -> <result>` (component type 7+i) REUSING the
    // `borrow<t>` defined type 4 (+ the `list<u8>` type 5 for a list result) laid for encode
    // (identity-free — the oracle reuses them), then a `canon lift` of its core func (k+6+i) against that
    // functype → component func k+2+i. A SCALAR result lifts plainly (no options); a `list<u8>` result
    // carries Memory 0 + Realloc (core func k+5, the shared `cabi_realloc`), exactly like encode.
    for (i, m) in methods.iter().enumerate() {
        let ty_idx = 7 + i as u32;
        let functype = match m.result {
            MethodResult::Scalar(prim) => self_borrow_to_scalar_functype(4, prim),
            MethodResult::ListU8 => self_borrow_to_list_functype(4, 5),
        };
        out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(1, &functype)));
        let lift = match m.result {
            MethodResult::Scalar(_) => canon_lift_item((k + 6 + i) as u32, ty_idx),
            MethodResult::ListU8 => {
                canon_lift_list_item((k + 6 + i) as u32, 0, (k + 5) as u32, ty_idx)
            }
        };
        out.extend_from_slice(&section(sec::CANON, &wasm_vec(1, &lift)));
    }
    // sec 4: the nested re-export component with make/encode + each scalar method.
    out.extend_from_slice(&component_section(
        &resource_inner_component_scalar_methods(methods),
    ));
    // sec 5: instantiate the inner component (component 0) with the resource (comp type 1) + the lifted
    // funcs (make = comp func k, encode = k+1, method i = k+2+i) → component instance 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_scalar_methods_item(1, k as u32, methods),
        ),
    ));
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
        &wasm_vec(
            1,
            &component_instantiate_call_item(1, k as u32, (k + 1) as u32),
        ),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(CLOSURE_INTERFACE, 1)),
    ));
    out
}

/// Assemble a closure-resource component whose `call` returns a COMPOUND (`Bytes` → `list<u8>`), not a
/// scalar. A fork of [`assemble_closure_resource`] that (1) ALSO aliases the program core's `memory` +
/// `cabi_realloc` (the compound result crosses through linear memory by the canonical ABI), and (2) lifts
/// `call` with Memory/Realloc canon options against a `(self: own<t>, args…) -> list<u8>` functype. Pairs
/// with [`serialize::closure_bytes_resource_core_module`] (whose `call` writes the payload + `(ptr,len)`
/// return area). BYTE target: `tests::closure_host_resource::oracle_closure_list_component`.
///
/// Core-func indices (k = imports.len()): lowered ops 0..k; `t-dtor` k; `resource.new` k+1, `resource.rep`
/// k+2; aliased `make` k+3, `call` k+4, `cabi_realloc` k+5 (memory is a memory index, not a func). Component
/// types: 0 = import instance-type, 1 = resource; make `own<t>` 2 + make-ft 3; call `own<t>` 4 + `list<u8>`
/// 5 + call-ft 6. Component funcs: aliased ops 0..k; `make` lift → k, `call` lift → k+1.
pub fn assemble_closure_bytes_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: import instance-type (component type 0). — identical prologue to `assemble_closure_resource`.
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
    out.extend_from_slice(&{
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    });
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
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(k as u32)),
    ));
    out.extend_from_slice(&{
        let mut items = resource_new_item(1);
        items.extend_from_slice(&resource_rep_item(1));
        section(sec::CANON, &wasm_vec(2, &items))
    });
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
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 2)])),
    ));
    // sec 6: alias `make` (k+3), `call` (k+4), `memory`, `cabi_realloc` (k+5) off the program instance
    // (core instance 3). UNLIKE the scalar path, the compound `call` needs the memory + realloc.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(3, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(3, CALL_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(3, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(3, REALLOC_EXPORT));
        section(sec::ALIAS, &wasm_vec(4, &items))
    });
    // sec 7: `own<t>` (type 2) + `make` functype `(export-params…) -> own<t>` (type 3).
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
    // sec 7: `own<t>` (type 4), `list<u8>` (type 5), then the `call` functype `(self: own<t>, args…) ->
    // list<u8>` (type 6).
    out.extend_from_slice(&{
        let mut items = own_item(1);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&closure_call_list_functype(4, arg_bytes, 5));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    });
    // sec 8: lift `call` (core func k+4) against functype type 6 WITH Memory 0 + Realloc (core func k+5) →
    // component func k+1. The compound result crosses through linear memory by the canonical ABI.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((k + 4) as u32, 0, (k + 5) as u32, 6),
        ),
    ));
    // sec 4/5/11: nested re-export component; instantiate (comp type 1 + comp funcs k, k+1); export.
    out.extend_from_slice(&component_section(&resource_inner_component_closure_bytes(
        make_param_bytes,
        arg_bytes,
    )));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_call_item(1, k as u32, (k + 1) as u32),
        ),
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

/// One PLAIN (non-closure) export riding alongside the closure exports (the "closure ALONGSIDE a
/// non-closure export" shape). Its body lives in the SAME program core module under `core_name`; the
/// envelope aliases it off the program instance, lifts it as an ORDINARY top-level component func, and
/// exports it under `name` (kebab-normalized). First cut: a SCALAR (component-primitive) result — a
/// compound `list<u8>` result would need the memory/realloc lift shape, a later widening.
pub struct PlainExportAbi {
    /// The public component export name (kebab-normalized at emit).
    pub name: String,
    /// The core-module export name the program instance exposes the body under (the source name).
    pub core_name: String,
    /// The export's parameter component-valtype bytes, in order (empty for a nullary export).
    pub param_bytes: Vec<u8>,
    /// The export's result component-primitive byte (scalar only this increment).
    pub result_byte: u8,
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
    assemble_mixed_closure_resource(
        main_core,
        dtor_core,
        imports,
        import_name,
        makes,
        arg_bytes,
        result_byte,
        &[],
    )
}

/// The MIXED shape: N same-signature closure exports (`makes` + one shared `call`) PLUS P PLAIN
/// (non-closure) exports (`plain`) in ONE component. Generalizes [`assemble_multi_closure_resource`]
/// (P=0). The closure `make`/`call` publish under `cadenza:closure/exports`; each plain export is aliased
/// off the SAME program instance, lifted as an ORDINARY top-level component func, and exported directly —
/// the `oracle_mixed_component` byte anchor proved the resource-instance + top-level-func coexistence.
///
/// Index deltas over the P=0 case: the plain bodies are aliased AFTER `call` (core funcs k+3+nmk+1..),
/// lifted AFTER the `call` lift (comp funcs k+nmk+1..), their functypes laid AFTER the call functype
/// (comp types 2+2*nmk+2 ..); each is exported at the TOP level (not inside the closure interface).
#[allow(clippy::too_many_arguments)]
pub fn assemble_mixed_closure_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
    plain: &[PlainExportAbi],
) -> Vec<u8> {
    let k = imports.len();
    let nmk = makes.len();
    let np = plain.len();
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
    // k+3..k+3+N (makes) then k+3+N (call), then each PLAIN export's body → core funcs k+3+N+1..
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for mk in makes {
            items.extend_from_slice(&core_alias_item(3, &mk.name));
        }
        items.extend_from_slice(&core_alias_item(3, CALL_CORE_EXPORT));
        for p in plain {
            items.extend_from_slice(&core_alias_item(3, &p.core_name));
        }
        section(sec::ALIAS, &wasm_vec(nmk + 1 + np, &items))
    });
    // sec 7: per make, `own<t>` + `make` functype `(export-params…) -> own<t>`; then `own<t>` + `call`
    // functype `(self: own<t>, args…) -> R`; then one PLAIN functype per plain export (a scalar
    // `(params…) -> R`, NO own<t> wrapper — a plain export carries no resource handle). Resource is comp
    // type 1; make own/functype at 2+2i / 3+2i; call own/functype at 2+2*nmk / 3+2*nmk; plain functype j at
    // 4+2*nmk+j.
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
        // each plain export's functype (scalar result, inline primitive byte).
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(2 * (nmk + 1) + np, &items))
    });
    // sec 8: lift each make (core func k+3+i) against its functype (type 3+2i) → comp func k+i; then lift
    // `call` (core func k+3+N) against the call functype (type 3+2N) → comp func k+N; then lift each PLAIN
    // export (core func k+3+N+1+j) against its functype (type 4+2N+j) → comp func k+N+1+j.
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
        for j in 0..np {
            let core_fn = (k + 3 + nmk + 1 + j) as u32;
            let functype = (4 + 2 * nmk + j) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        section(sec::CANON, &wasm_vec(nmk + 1 + np, &items))
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
    // sec 11: export the closure interface instance, then each PLAIN export as an ORDINARY top-level
    // component func (comp func k+nmk+1+j), under its kebab-normalized name.
    out.extend_from_slice(&{
        let mut items = export_instance_item(CLOSURE_INTERFACE, 1);
        for (j, p) in plain.iter().enumerate() {
            let comp_fn = (k + nmk + 1 + j) as u32;
            items.extend_from_slice(&comp_export_item(&p.name, comp_fn));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(1 + np, &items))
    });
    out
}

/// Assemble a MULTI-EXPORT BYTE-ROPE-result closure component: N `make-<name>` functions sharing ONE `call`
/// that returns `list<u8>` (a `Bytes`/`String` closure result). Combines [`assemble_multi_closure_resource`]
/// (N makes + shared call) with [`assemble_closure_bytes_resource`] (memory + cabi_realloc + the
/// Memory/Realloc-lifted list `call`). Pairs with [`serialize::multi_closure_bytes_resource_core_module`].
///
/// Core-func indices (k = imports.len(), N = makes): lowered ops 0..k; `t-dtor` k; `resource.new` k+1,
/// `resource.rep` k+2; aliased make[i] k+3+i, `call` k+3+N, `cabi_realloc` k+4+N (memory is a memory index).
/// Component types: 0 = import instance-type, 1 = resource; per make own<t> (2+2i) + functype (3+2i); call
/// own<t> (2+2N) + `list<u8>` (3+2N) + call-ft (4+2N). Component funcs: aliased ops 0..k; make[i] lift → k+i,
/// `call` lift → k+N.
pub fn assemble_multi_closure_bytes_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
) -> Vec<u8> {
    let k = imports.len();
    let nmk = makes.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // Shared prologue with the scalar multi envelope: import instance-type, runtime import, op alias/lower,
    // dtor, resource type, resource.new/rep, heap instance, program module/instance.
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
    out.extend_from_slice(&{
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    });
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
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(k as u32)),
    ));
    out.extend_from_slice(&{
        let mut items = resource_new_item(1);
        items.extend_from_slice(&resource_rep_item(1));
        section(sec::CANON, &wasm_vec(2, &items))
    });
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
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 2)])),
    ));
    // sec 6: alias make[i] (k+3+i), `call` (k+3+N), `memory`, `cabi_realloc` (k+4+N) off the program
    // instance (core instance 3). UNLIKE the scalar multi path, the compound `call` needs memory + realloc.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for mk in makes {
            items.extend_from_slice(&core_alias_item(3, &mk.name));
        }
        items.extend_from_slice(&core_alias_item(3, CALL_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(3, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(3, REALLOC_EXPORT));
        // N makes + `call` + `memory` + `cabi_realloc`.
        section(sec::ALIAS, &wasm_vec(nmk + 3, &items))
    });
    // sec 7: per make `own<t>` (2+2i) + make functype (3+2i); then call `own<t>` (2+2N) + `list<u8>` (3+2N) +
    // call functype `(self: own<t>, args…) -> list<u8>` (4+2N).
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
        items.extend_from_slice(&own_item(1));
        let call_own_ty = (2 + 2 * nmk) as u32;
        let list_ty = (3 + 2 * nmk) as u32;
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&closure_call_list_functype(call_own_ty, arg_bytes, list_ty));
        section(sec::COMPONENT_TYPE, &wasm_vec(2 * nmk + 3, &items))
    });
    // sec 8: lift make[i] (core func k+3+i) against functype 3+2i → comp func k+i; lift `call` (core func
    // k+3+N) against functype 4+2N WITH Memory 0 + Realloc (core func k+4+N) → comp func k+N.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..nmk {
            let core_fn = (k + 3 + i) as u32;
            let functype = (3 + 2 * i) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        let call_core_fn = (k + 3 + nmk) as u32;
        let call_functype = (4 + 2 * nmk) as u32;
        let realloc_fn = (k + 4 + nmk) as u32;
        items.extend_from_slice(&canon_lift_list_item(
            call_core_fn,
            0,
            realloc_fn,
            call_functype,
        ));
        section(sec::CANON, &wasm_vec(nmk + 1, &items))
    });
    // sec 4/5/11: nested re-export component (list-result `call`); instantiate; export the closure interface.
    out.extend_from_slice(&component_section(
        &resource_inner_component_multi_closure_bytes(makes, arg_bytes),
    ));
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

/// The MULTI-EXPORT inner re-export component for a BYTE-ROPE-result closure: like
/// [`resource_inner_component_multi_closure`] but the shared `call`'s result is `list<u8>` (each side mints
/// its own `list<u8>` defined type, shifting the export-side type base by 2 vs the scalar version — a make
/// contributes own<t>+ft = 2 types, the call contributes own<t>+list<u8>+ft = 3). Uses a running type
/// counter for clarity. Imported funcs: make[i] → func i, `call` → func N.
fn resource_inner_component_multi_closure_bytes(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
) -> Vec<u8> {
    let n = makes.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    let mut ty = 1u32; // next defined-type index (0 = imported resource)
    // Per make i: own<0> (ty) + make functype (ty+1); import func i.
    for (i, mk) in makes.iter().enumerate() {
        let own_ty = ty;
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
            &wasm_vec(1, &import_func_item(&import_wire_name(i), own_ty + 1)),
        ));
        ty += 2;
    }
    // Shared call: own<0> (ty) + list<u8> (ty+1) + call functype (ty+2); import func N.
    {
        let own_ty = ty;
        let list_ty = ty + 1;
        let ft_ty = ty + 2;
        out.extend_from_slice(&{
            let mut items = own_item(0);
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_functype(own_ty, arg_bytes, list_ty));
            section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_func_item(&import_wire_name(n), ft_ty)),
        ));
        ty += 3;
    }
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY as `t` → exported type `ty` (call it R).
    let r = ty;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    ty += 1;
    // Per make i: own<R> (ty) + make functype re-typed (ty+1); export func i under its name.
    for (i, mk) in makes.iter().enumerate() {
        let own_ty = ty;
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
            &wasm_vec(
                1,
                &export_func_ascribed_item(&mk.name, i as u32, own_ty + 1),
            ),
        ));
        ty += 2;
    }
    // Shared call: own<R> (ty) + list<u8> (ty+1) + call functype re-typed (ty+2); export `call` (func N).
    {
        let own_ty = ty;
        let list_ty = ty + 1;
        let ft_ty = ty + 2;
        out.extend_from_slice(&{
            let mut items = own_item(r);
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_functype(own_ty, arg_bytes, list_ty));
            section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(
                1,
                &export_func_ascribed_item(CALL_BOUNDARY_NAME, n as u32, ft_ty),
            ),
        ));
    }
    out
}

/// One SIGNATURE GROUP's boundary shape for the distinct-signature envelope: its per-export `make`s + its
/// `call` arg/result bytes. Each group becomes ONE resource type with its own make/call published under
/// `make-<name>`/`call-<g>` (g = the group index).
pub struct SigGroupAbi {
    pub makes: Vec<ClosureMakeAbi>,
    pub arg_bytes: Vec<u8>,
    pub result_byte: u8,
}

/// One SIGNATURE GROUP's boundary shape for the distinct-signature ROUND-TRIP envelope: its producers
/// (`makes`, each `(params…) -> own<t_g>`) + its CONSUMERS (each a named func taking a closure of the
/// group's signature back as `own<t_g>` plus scalars, in source order). Each group is ONE resource type;
/// unlike [`SigGroupAbi`] there is NO shared `call-g` — the consumers ARE the applying functions.
pub struct RtSigGroupAbi {
    pub makes: Vec<ClosureMakeAbi>,
    pub consumers: Vec<ClosureConsumeAbi>,
}

/// Assemble a DISTINCT-SIGNATURE closure-resource component: closures of G DIFFERENT signatures cross as G
/// resource types, published together under `cadenza:closure/exports`. The multi-export envelope
/// generalized from ONE resource type to G — G dtors, G resource types, G `resource-new`/`resource-rep`
/// canon pairs (each bound to its resource), and an inner component importing/re-exporting all G resources
/// with each fn ascribed to its own. `main_core` is `serialize::distinct_sig_resource_core_module`'s output
/// (exporting each group's `make-<name>` + `call-<g>`). The `distinct_signature_…` oracle is the byte
/// reference this hand-emits.
///
/// Core-func index layout (k = imports.len()): lowered ops → 0..k; then per group g (in order): `t<g>-dtor`
/// alias, then `resource.new-g`/`resource.rep-g`. Component types: 0 = import instance-type; then per group
/// its resource type; then per fn its own<t> + functype.
pub fn assemble_distinct_sig_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    groups: &[SigGroupAbi],
) -> Vec<u8> {
    assemble_distinct_sig_resource_mixed(main_core, dtor_core, imports, import_name, groups, &[])
}

/// The distinct-signature envelope with P PLAIN (non-closure) exports riding alongside the G resource
/// types. Generalizes [`assemble_distinct_sig_resource`] (P=0): each plain body is aliased off the SAME
/// program instance (after all the closure fns), lifted as an ORDINARY top-level component func, and
/// exported directly under its kebab name — the same plain-export composition the same-signature mixed
/// envelope uses ([`assemble_mixed_closure_resource`]), applied to the G-resource shape.
///
/// Index deltas over P=0: plain bodies are aliased AFTER the `total_fns` closure fns (core funcs
/// `k+3g+total_fns+j`), their functypes laid AFTER the fn functypes (comp types `1+g+2*total_fns+j`),
/// lifted AFTER the closure lifts (comp funcs `k+total_fns+j`), and exported at the TOP level.
#[allow(clippy::too_many_arguments)]
pub fn assemble_distinct_sig_resource_mixed(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    groups: &[SigGroupAbi],
    plain: &[PlainExportAbi],
) -> Vec<u8> {
    let k = imports.len();
    let g = groups.len();
    let np = plain.len();
    // Flat function count across all groups: each group contributes (its makes) + 1 call.
    let total_fns: usize = groups.iter().map(|gr| gr.makes.len() + 1).sum();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: import instance-type (component type 0).
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
    // sec 10: import the runtime interface (component instance 0).
    out.extend_from_slice(&{
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    });
    // sec 6: alias each op → comp funcs 0..k.
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
    // The `drop` op's core-func index (each group's dtor calls it).
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| i as u32)
        .expect("the closure-resource escape imports `drop` for the dtor");
    // Per group: a dtor instance + module + instantiate + alias, a resource type, and resource.new/rep. The
    // dtor MODULE is shared (dtor_core), instantiated once per group (each resource needs its own dtor
    // core-func). Track each resource type's component-type index + its new/rep core-func indices.
    // Core funcs so far: 0..k lowered ops. Each group appends: t<g>-dtor alias (1) + resource.new (1) +
    // resource.rep (1) = 3 core funcs. So group g's dtor = k + 3g, new = k + 3g + 1, rep = k + 3g + 2.
    let mut res_type_idx: Vec<u32> = Vec::new(); // component-type index per group
    let mut rnew_core: Vec<u32> = Vec::new();
    let mut rrep_core: Vec<u32> = Vec::new();
    // Component types: 0 = import instance-type. Each group's resource type is minted next: type 1, 2, ….
    for gi in 0..g {
        // sec 2: dtor instance exporting the lowered `drop` → a fresh core instance.
        out.extend_from_slice(&section(
            sec::CORE_INSTANCE,
            &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
        ));
        // sec 1: the (shared) dtor module.
        out.extend_from_slice(&core_module_section(dtor_core));
        // sec 2: instantiate the dtor module threading heap-dtor = the instance just made. The dtor
        // instance for group g is core-instance `2*gi` (each group adds 2 core instances: the export
        // instance + the module instantiation); the dtor MODULE for group g is module `gi` (each group
        // adds one module here; the program core module comes after all groups).
        out.extend_from_slice(&section(
            sec::CORE_INSTANCE,
            &wasm_vec(
                1,
                &core_instantiate_item(gi as u32, &[(HEAP_DTOR_MODULE, (2 * gi) as u32)]),
            ),
        ));
        // sec 6: alias `t-dtor` from the instantiation instance (core instance `2*gi + 1`) → a core func.
        out.extend_from_slice(&section(
            sec::ALIAS,
            &wasm_vec(1, &core_alias_item((2 * gi + 1) as u32, DTOR_CORE_EXPORT)),
        ));
        let dtor_fn = (k + 3 * gi) as u32;
        // sec 7: the resource type (rep i32, dtor = the aliased core func) → component type 1+gi.
        out.extend_from_slice(&section(
            sec::COMPONENT_TYPE,
            &wasm_vec(1, &resource_type_item(dtor_fn)),
        ));
        let rty = (1 + gi) as u32;
        res_type_idx.push(rty);
        // sec 8: canon resource.new + resource.rep for THIS resource type.
        out.extend_from_slice(&{
            let mut items = resource_new_item(rty);
            items.extend_from_slice(&resource_rep_item(rty));
            section(sec::CANON, &wasm_vec(2, &items))
        });
        rnew_core.push(dtor_fn + 1);
        rrep_core.push(dtor_fn + 2);
    }
    // sec 2: the `heap` core instance exporting the k ops + per group `resource-new-<g>`/`resource-rep-<g>`
    // → the core instance the program core binds its `heap` import to.
    let rnew_names: Vec<String> = (0..g).map(|gi| format!("resource-new-{gi}")).collect();
    let rrep_names: Vec<String> = (0..g).map(|gi| format!("resource-rep-{gi}")).collect();
    let heap_exports: Vec<(&str, u32)> = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, i as u32))
            .collect();
        for gi in 0..g {
            ex.push((rnew_names[gi].as_str(), rnew_core[gi]));
            ex.push((rrep_names[gi].as_str(), rrep_core[gi]));
        }
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    let heap_inst = (2 * g) as u32; // core instances 0..2g are the g dtor pairs; the heap instance is next
    // sec 1/2: the program core module (module g); instantiate threading `heap` = the heap instance.
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(
            1,
            &core_instantiate_item(g as u32, &[(HEAP_MODULE, heap_inst)]),
        ),
    ));
    let prog_inst = heap_inst + 1;
    // sec 6: alias each group's `make-<name>` + `call-<g>` off the program instance → core funcs (after the
    // lowered ops + 3g resource funcs). Record each fn's core-func index in flat order.
    let mut fn_core: Vec<u32> = Vec::new();
    let mut plain_core: Vec<u32> = Vec::new();
    let mut next_fn = (k + 3 * g) as u32;
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for (gi, gr) in groups.iter().enumerate() {
            for mk in &gr.makes {
                items.extend_from_slice(&core_alias_item(prog_inst, &mk.name));
                fn_core.push(next_fn);
                next_fn += 1;
            }
            items.extend_from_slice(&core_alias_item(prog_inst, &format!("call-g{gi}")));
            fn_core.push(next_fn);
            next_fn += 1;
        }
        // each PLAIN export's body, aliased AFTER all the closure fns → core funcs k+3g+total_fns+j.
        for p in plain {
            items.extend_from_slice(&core_alias_item(prog_inst, &p.core_name));
            plain_core.push(next_fn);
            next_fn += 1;
        }
        section(sec::ALIAS, &wasm_vec(total_fns + np, &items))
    });
    // sec 7: per fn, its `own<t>` + functype. Component types after the import-instance-type (0) + G
    // resource types (1..1+g): the next defined type index is `1 + g`. Each fn adds own<t> (1) + functype
    // (1). Record each fn's functype component-type index.
    let mut fn_functype: Vec<u32> = Vec::new();
    let mut plain_functype: Vec<u32> = Vec::new();
    out.extend_from_slice(&{
        let mut items = Vec::new();
        let mut ti = (1 + g) as u32;
        for (gi, gr) in groups.iter().enumerate() {
            let rty = res_type_idx[gi];
            for mk in &gr.makes {
                items.extend_from_slice(&own_item(rty));
                let own_ty = ti;
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(own_ty),
                ));
                fn_functype.push(ti + 1);
                ti += 2;
            }
            // call-<g>: own<t_g> + call functype.
            items.extend_from_slice(&own_item(rty));
            items.extend_from_slice(&closure_call_functype(ti, &gr.arg_bytes, gr.result_byte));
            fn_functype.push(ti + 1);
            ti += 2;
        }
        // each PLAIN export's functype (scalar result, inline primitive byte — NO own<t> wrapper).
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
            plain_functype.push(ti);
            ti += 1;
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(2 * total_fns + np, &items))
    });
    // sec 8: lift each fn (its core func) against its functype → comp funcs k..k+total_fns; then lift each
    // PLAIN export (core func k+3g+total_fns+j) against its functype → comp func k+total_fns+j.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..total_fns {
            items.extend_from_slice(&canon_lift_item(fn_core[i], fn_functype[i]));
        }
        for j in 0..np {
            items.extend_from_slice(&canon_lift_item(plain_core[j], plain_functype[j]));
        }
        section(sec::CANON, &wasm_vec(total_fns + np, &items))
    });
    // sec 4/5/11: nested re-export component; instantiate (G resources + total_fns comp funcs); export.
    out.extend_from_slice(&component_section(&resource_inner_component_distinct_sig(
        groups,
    )));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_distinct_sig_item(&res_type_idx, k as u32, groups),
        ),
    ));
    // sec 11: export the closure interface instance, then each PLAIN export as an ORDINARY top-level
    // component func (comp func k+total_fns+j), under its kebab-normalized name.
    out.extend_from_slice(&{
        let mut items = export_instance_item(CLOSURE_INTERFACE, 1);
        for (j, p) in plain.iter().enumerate() {
            let comp_fn = (k + total_fns + j) as u32;
            items.extend_from_slice(&comp_export_item(&p.name, comp_fn));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(1 + np, &items))
    });
    out
}

/// Assemble a DISTINCT-SIGNATURE ROUND-TRIP component: closures of G different signatures cross as G
/// resource types, and each group publishes its PRODUCERS (`make-<name>`) AND CONSUMERS (named funcs
/// taking a closure of that signature back). The distinct-sig envelope generalized so each group's
/// functions are `makes ++ consumers` (a consumer functype is `(own<t_g>, args…)->R` in SOURCE param
/// order) instead of `makes + [call-g]`. `main_core` is `serialize::distinct_sig_roundtrip_core_module`'s
/// output. Same G-resource core-func layout as `assemble_distinct_sig_resource`.
pub fn assemble_distinct_sig_roundtrip_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    groups: &[RtSigGroupAbi],
) -> Vec<u8> {
    assemble_distinct_sig_roundtrip_resource_mixed(
        main_core,
        dtor_core,
        imports,
        import_name,
        groups,
        &[],
    )
}

/// The distinct-signature round-trip envelope with P PLAIN (non-closure) exports riding alongside the G
/// resource groups. Generalizes [`assemble_distinct_sig_roundtrip_resource`] (P=0): each plain body is
/// aliased off the SAME program instance (after the `total_fns` closure funcs), lifted as an ORDINARY
/// top-level component func, and exported directly under its kebab name. Index deltas over P=0: plain
/// bodies aliased AFTER the closure funcs (core `k+3g+total_fns+j`), functypes AFTER the fn functypes
/// (comp types `1+g+2*total_fns+j`), lifted AFTER the closure lifts (comp funcs `k+total_fns+j`), exported
/// at the TOP level.
#[allow(clippy::too_many_arguments)]
pub fn assemble_distinct_sig_roundtrip_resource_mixed(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    groups: &[RtSigGroupAbi],
    plain: &[PlainExportAbi],
) -> Vec<u8> {
    let k = imports.len();
    let g = groups.len();
    let np = plain.len();
    let total_fns: usize = groups
        .iter()
        .map(|gr| gr.makes.len() + gr.consumers.len())
        .sum();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7/10/6/8: import instance-type, import runtime, alias+lower the ops (identical to distinct-sig).
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
    out.extend_from_slice(&{
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    });
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| i as u32)
        .expect("the closure-resource escape imports `drop` for the dtor");
    // Per group: dtor pair + resource type + resource.new/rep (identical layout to distinct-sig).
    let mut res_type_idx: Vec<u32> = Vec::new();
    let mut rnew_core: Vec<u32> = Vec::new();
    let mut rrep_core: Vec<u32> = Vec::new();
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::CORE_INSTANCE,
            &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
        ));
        out.extend_from_slice(&core_module_section(dtor_core));
        out.extend_from_slice(&section(
            sec::CORE_INSTANCE,
            &wasm_vec(
                1,
                &core_instantiate_item(gi as u32, &[(HEAP_DTOR_MODULE, (2 * gi) as u32)]),
            ),
        ));
        out.extend_from_slice(&section(
            sec::ALIAS,
            &wasm_vec(1, &core_alias_item((2 * gi + 1) as u32, DTOR_CORE_EXPORT)),
        ));
        let dtor_fn = (k + 3 * gi) as u32;
        out.extend_from_slice(&section(
            sec::COMPONENT_TYPE,
            &wasm_vec(1, &resource_type_item(dtor_fn)),
        ));
        res_type_idx.push((1 + gi) as u32);
        out.extend_from_slice(&{
            let mut items = resource_new_item((1 + gi) as u32);
            items.extend_from_slice(&resource_rep_item((1 + gi) as u32));
            section(sec::CANON, &wasm_vec(2, &items))
        });
        rnew_core.push(dtor_fn + 1);
        rrep_core.push(dtor_fn + 2);
    }
    let rnew_names: Vec<String> = (0..g).map(|gi| format!("resource-new-{gi}")).collect();
    let rrep_names: Vec<String> = (0..g).map(|gi| format!("resource-rep-{gi}")).collect();
    let heap_exports: Vec<(&str, u32)> = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, i as u32))
            .collect();
        for gi in 0..g {
            ex.push((rnew_names[gi].as_str(), rnew_core[gi]));
            ex.push((rrep_names[gi].as_str(), rrep_core[gi]));
        }
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    let heap_inst = (2 * g) as u32;
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(
            1,
            &core_instantiate_item(g as u32, &[(HEAP_MODULE, heap_inst)]),
        ),
    ));
    let prog_inst = heap_inst + 1;
    // sec 6: alias each group's makes then consumers off the program instance (core order matches the
    // core module's export order: per group, makes then consumers).
    let mut fn_core: Vec<u32> = Vec::new();
    let mut plain_core: Vec<u32> = Vec::new();
    let mut next_fn = (k + 3 * g) as u32;
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for gr in groups.iter() {
            for mk in &gr.makes {
                items.extend_from_slice(&core_alias_item(prog_inst, &mk.name));
                fn_core.push(next_fn);
                next_fn += 1;
            }
            for c in &gr.consumers {
                items.extend_from_slice(&core_alias_item(prog_inst, &c.name));
                fn_core.push(next_fn);
                next_fn += 1;
            }
        }
        // each PLAIN export's body, aliased AFTER all the closure fns → core funcs k+3g+total_fns+j.
        for p in plain {
            items.extend_from_slice(&core_alias_item(prog_inst, &p.core_name));
            plain_core.push(next_fn);
            next_fn += 1;
        }
        section(sec::ALIAS, &wasm_vec(total_fns + np, &items))
    });
    // sec 7: per fn its `own<t_g>` + functype (make: `(params…)->own<t>`; consumer: source-ordered params);
    // then one PLAIN functype per plain export (scalar `(params…)->R`, NO own<t> wrapper).
    let mut fn_functype: Vec<u32> = Vec::new();
    let mut plain_functype: Vec<u32> = Vec::new();
    out.extend_from_slice(&{
        let mut items = Vec::new();
        let mut ti = (1 + g) as u32;
        for (gi, gr) in groups.iter().enumerate() {
            let rty = res_type_idx[gi];
            for mk in &gr.makes {
                items.extend_from_slice(&own_item(rty));
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(ti),
                ));
                fn_functype.push(ti + 1);
                ti += 2;
            }
            for c in &gr.consumers {
                items.extend_from_slice(&own_item(rty));
                items.extend_from_slice(&consumer_functype(ti, &c.params, c.result_byte));
                fn_functype.push(ti + 1);
                ti += 2;
            }
        }
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
            plain_functype.push(ti);
            ti += 1;
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(2 * total_fns + np, &items))
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..total_fns {
            items.extend_from_slice(&canon_lift_item(fn_core[i], fn_functype[i]));
        }
        for j in 0..np {
            items.extend_from_slice(&canon_lift_item(plain_core[j], plain_functype[j]));
        }
        section(sec::CANON, &wasm_vec(total_fns + np, &items))
    });
    out.extend_from_slice(&component_section(
        &resource_inner_component_distinct_sig_rt(groups),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_distinct_sig_rt_item(&res_type_idx, k as u32, groups),
        ),
    ));
    // sec 11: export the closure interface, then each PLAIN export as an ordinary top-level comp func
    // (comp func k+total_fns+j).
    out.extend_from_slice(&{
        let mut items = export_instance_item(CLOSURE_INTERFACE, 1);
        for (j, p) in plain.iter().enumerate() {
            let comp_fn = (k + total_fns + j) as u32;
            items.extend_from_slice(&comp_export_item(&p.name, comp_fn));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(1 + np, &items))
    });
    out
}

/// One boundary parameter of a round-trip CONSUMER, in SOURCE ORDER: a closure the host hands back
/// (crosses as `own<t>`) or a plain scalar (its component primitive byte). The consumer's component
/// functype is its params in this exact order — a closure param need NOT be first, and a consumer may take
/// several closures (all the same signature → the same resource type `t`).
#[derive(Clone, Copy)]
pub enum ConsumeParamAbi {
    /// A closure the host hands back — crosses as `own<t>` (the resource handle).
    Closure,
    /// A scalar param — its component primitive byte (`comp_valtype_of`).
    Scalar(u8),
}

/// One CONSUMER export's boundary shape for the round-trip envelope: its name + its params in SOURCE ORDER
/// (each a closure `own<t>` or a scalar byte) + result byte. Exported as a plain func under its own name;
/// the host threads a produced handle in for each `Closure` param. Generalizes the earlier "closure first,
/// then the closure's args" shape — a consumer's boundary params are its OWN params, so a closure can sit
/// anywhere in the list and there may be more than one.
pub struct ClosureConsumeAbi {
    pub name: String,
    pub params: Vec<ConsumeParamAbi>,
    pub result_byte: u8,
}

/// Assemble a ROUND-TRIP closure-resource component (C-HOST-4): N producer `make-<name>` functions PLUS M
/// CONSUMER functions, published together under `cadenza:closure/exports`. A producer mints a closure
/// handle (`() / (params…) -> own<t>`); a consumer takes a handle back (`(g: own<t>, args…) -> R`) and
/// applies it. Structurally the multi-export envelope with the shared `call` generalized to M named
/// consumers (each a `call`-shaped functype). `main_core` is
/// [`serialize::roundtrip_resource_core_module`]'s output (exporting each `make-<name>` + each consumer).
///
/// Outer index spaces (k = imports.len(), N = makes, M = consumers): lowered ops → core funcs 0..k;
/// `t-dtor` → k; `resource.new` → k+1, `resource.rep` → k+2; aliased make[i] → k+3+i, consumer[j] →
/// k+3+N+j. Component funcs: aliased ops 0..k, lifted make[i] → comp func k+i, consumer[j] → k+N+j.
/// Component types: 0 = import instance-type, 1 = resource; then per make: `own<t>` + make-functype; then
/// per consumer: `own<t>` + consume-functype.
pub fn assemble_roundtrip_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    consumers: &[ClosureConsumeAbi],
    _spans: Option<()>,
) -> Vec<u8> {
    assemble_roundtrip_resource_mixed(
        main_core,
        dtor_core,
        imports,
        import_name,
        makes,
        consumers,
        &[],
    )
}

/// The round-trip envelope with P PLAIN (non-closure) exports riding alongside the producers + consumers.
/// Generalizes [`assemble_roundtrip_resource`] (P=0): each plain body is aliased off the SAME program
/// instance (after the nfns closure funcs), lifted as an ORDINARY top-level component func, and exported
/// directly under its kebab name — the same plain-export composition the multi-export mixed envelope uses.
/// Without this, a plain export in a round-trip program was SILENTLY DROPPED (a valid component missing the
/// name), a miscompile.
///
/// Index deltas over P=0: plain bodies aliased AFTER the nfns closure funcs (core funcs `k+3+nfns+j`),
/// their functypes laid AFTER the fn functypes (comp types `2+2*nfns+j`), lifted AFTER the closure lifts
/// (comp funcs `k+nfns+j`), and exported at the TOP level.
#[allow(clippy::too_many_arguments)]
pub fn assemble_roundtrip_resource_mixed(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    consumers: &[ClosureConsumeAbi],
    plain: &[PlainExportAbi],
) -> Vec<u8> {
    let k = imports.len();
    let nmk = makes.len();
    let ncons = consumers.len();
    let nfns = nmk + ncons;
    let np = plain.len();
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

    // sec 10: import the runtime interface.
    out.extend_from_slice(&{
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    });
    // sec 6: alias each op → comp funcs 0..k.
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
    // sec 7: resource type `t` → component type 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item(k as u32)),
    ));
    // sec 8: canon resource.new (k+1) + resource.rep (k+2).
    out.extend_from_slice(&{
        let mut items = resource_new_item(1);
        items.extend_from_slice(&resource_rep_item(1));
        section(sec::CANON, &wasm_vec(2, &items))
    });
    // sec 2: the `heap` core instance (k ops + resource-new + resource-rep) → core instance 2.
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
    // sec 1/2: program core module (module 1); instantiate → core instance 3.
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(1, &[(HEAP_MODULE, 2)])),
    ));
    // sec 6: alias each make + each consumer off the program instance → core funcs k+3..k+3+nfns; then each
    // PLAIN export's body → core funcs k+3+nfns..
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for mk in makes {
            items.extend_from_slice(&core_alias_item(3, &mk.name));
        }
        for c in consumers {
            items.extend_from_slice(&core_alias_item(3, &c.name));
        }
        for p in plain {
            items.extend_from_slice(&core_alias_item(3, &p.core_name));
        }
        section(sec::ALIAS, &wasm_vec(nfns + np, &items))
    });
    // sec 7: per make, `own<t>` + make functype; per consumer, `own<t>` + consume functype (`(g: own<t>,
    // args…) -> R`, the `call` shape). Resource is comp type 1.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        let mut ti = 2u32; // next defined-type index (type 0 = import inst, 1 = resource)
        for mk in makes {
            items.extend_from_slice(&own_item(1));
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(ti),
            ));
            ti += 2;
        }
        for c in consumers {
            items.extend_from_slice(&own_item(1));
            items.extend_from_slice(&consumer_functype(ti, &c.params, c.result_byte));
            ti += 2;
        }
        // each PLAIN export's functype (scalar result, inline primitive byte — NO own<t> wrapper).
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(2 * nfns + np, &items))
    });
    // sec 8: lift each make + each consumer against its functype → comp funcs k..k+nfns; then lift each
    // PLAIN export (core func k+3+nfns+j) against its functype (comp type 2+2*nfns+j) → comp func k+nfns+j.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..nfns {
            let core_fn = (k + 3 + i) as u32;
            let functype = (3 + 2 * i) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        for j in 0..np {
            let core_fn = (k + 3 + nfns + j) as u32;
            let functype = (2 + 2 * nfns + j) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        section(sec::CANON, &wasm_vec(nfns + np, &items))
    });
    // sec 4/5/11: nested re-export component; instantiate (resource + comp funcs k..k+nfns); export the
    // closure interface, then each PLAIN export as an ordinary top-level comp func (comp func k+nfns+j).
    out.extend_from_slice(&component_section(&resource_inner_component_roundtrip(
        makes, consumers,
    )));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_roundtrip_item(1, k as u32, makes, consumers),
        ),
    ));
    out.extend_from_slice(&{
        let mut items = export_instance_item(CLOSURE_INTERFACE, 1);
        for (j, p) in plain.iter().enumerate() {
            let comp_fn = (k + nfns + j) as u32;
            items.extend_from_slice(&comp_export_item(&p.name, comp_fn));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(1 + np, &items))
    });
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

/// The inner re-export component for a COMPOUND-RESULT (`Bytes`→`list<u8>`) closure: like
/// [`resource_inner_component_closure`] but `call`'s result is a `list<u8>` defined type instead of a scalar
/// byte. Each `list<u8>` type is minted on both the import and export side (independent type spaces). Type
/// indices (import side): resource 0; make `own<0>` 1, make-ft 2; call `own<0>` 3, `list<u8>` 4, call-ft 5.
/// Export side: re-exported resource 6; make `own<6>` 7, make-ft 8; call `own<6>` 9, `list<u8>` 10, call-ft
/// 11. Imported funcs: make 0, call 1.
fn resource_inner_component_closure_bytes(make_param_bytes: &[u8], arg_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: `own<0>` (type 1) + imported `make` functype `(export-params…) -> own<0>` (type 2).
    out.extend_from_slice(&{
        let mut items = own_item(0);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: `own<0>` (type 3), `list<u8>` (type 4), imported `call` functype `(self: own<3>, args…) ->
    // list<u8>` (type 5).
    out.extend_from_slice(&{
        let mut items = own_item(0);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&closure_call_list_functype(3, arg_bytes, 4));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-call", 5)),
    ));
    // sec 11: RE-EXPORT the resource type 0 DIRECTLY as `t` → exported type 6.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: `own<6>` (type 7) + `make` functype re-typed against the exported resource (type 8).
    out.extend_from_slice(&{
        let mut items = own_item(6);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(7)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, 8)),
    ));
    // sec 7: `own<6>` (type 9), `list<u8>` (type 10), `call` functype re-typed (type 11).
    out.extend_from_slice(&{
        let mut items = own_item(6);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&closure_call_list_functype(9, arg_bytes, 10));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(CALL_BOUNDARY_NAME, 1, 11)),
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
            // PRIVATE wiring name — indexed, not the user name (a user name may be non-kebab, e.g. `mkA`,
            // which wasmtime rejects as an extern name). The instantiate item pairs by this same `f<i>`.
            &wasm_vec(1, &import_func_item(&import_wire_name(i), ft_ty)),
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
        &wasm_vec(1, &import_func_item(&import_wire_name(n), call_ft_ty)),
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

/// The DISTINCT-SIGNATURE inner re-export component: imports G abstract resources (`import-type-t<g>`) +
/// each group's `make-<name>` (→ `own<t_g>`) and `call-<g>` (`(self: own<t_g>, args…) -> R`), then
/// re-exports all G resources (`t0`,`t1`,…) + every fn ascribed against its group's exported resource.
/// The only way to export G resources-with-methods together. Import-phase type layout: resources → types
/// 0..g; then per fn (flat, group order — each group's makes then its call): `own<t_g>` at `g + 2f` +
/// functype at `g + 2f + 1`, and the func imported → func f. Export phase: re-export G resources → exported
/// types `E..E+g` (E = g + 2*total_fns); then per fn: `own<exp_t_g>` + re-ascribed functype.
fn resource_inner_component_distinct_sig(groups: &[SigGroupAbi]) -> Vec<u8> {
    let g = groups.len();
    let total_fns: usize = groups.iter().map(|gr| gr.makes.len() + 1).sum();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import G abstract resources → types 0..g.
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_subresource_item(&format!("import-type-t{gi}"))),
        ));
    }
    // IMPORT each fn (flat, group order): own<t_g> (type g+2f) + functype (type g+2f+1); import → func f.
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        for mk in &gr.makes {
            let own_ty = (g + 2 * f) as u32;
            let ft_ty = (g + 2 * f + 1) as u32;
            out.extend_from_slice(&{
                let mut items = own_item(gi as u32);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(own_ty),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            f += 1;
        }
        // call-<gi> : (self: own<t_gi>, args…) -> R
        let own_ty = (g + 2 * f) as u32;
        let ft_ty = (g + 2 * f + 1) as u32;
        out.extend_from_slice(&{
            let mut items = own_item(gi as u32);
            items.extend_from_slice(&closure_call_functype(
                own_ty,
                &gr.arg_bytes,
                gr.result_byte,
            ));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
        ));
        f += 1;
    }
    // sec 11: RE-EXPORT each resource DIRECTLY as `t<g>` → exported types E..E+g (E = g + 2*total_fns).
    let e = (g + 2 * total_fns) as u32;
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_type_direct_item(&format!("t{gi}"), gi as u32)),
        ));
    }
    // EXPORT each fn ascribed against its group's EXPORTED resource (exp type E + gi). Types after the
    // re-exports continue at E + g; each fn adds own<exp_t_g> + functype.
    let mut ti = e + g as u32;
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        let exp_rty = e + gi as u32;
        for mk in &gr.makes {
            out.extend_from_slice(&{
                let mut items = own_item(exp_rty);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(ti),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_func_ascribed_item(&mk.name, f as u32, ti + 1)),
            ));
            ti += 2;
            f += 1;
        }
        out.extend_from_slice(&{
            let mut items = own_item(exp_rty);
            items.extend_from_slice(&closure_call_functype(ti, &gr.arg_bytes, gr.result_byte));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(
                1,
                &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ti + 1),
            ),
        ));
        ti += 2;
        f += 1;
    }
    out
}

/// The distinct-signature instantiate item: supply each imported resource (`import-type-t<g>` → its outer
/// resource type) + each fn (`import-func-<make>`/`import-func-call-<g>` → its lifted comp func, flat group
/// order starting at `first_fn`).
fn component_instantiate_distinct_sig_item(
    res_type_idx: &[u32],
    first_fn: u32,
    groups: &[SigGroupAbi],
) -> Vec<u8> {
    let mut item = vec![0x00];
    uleb128(0, &mut item);
    let mut arg_items = Vec::new();
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    let mut n_args = 0usize;
    for (gi, &rty) in res_type_idx.iter().enumerate() {
        push(&format!("import-type-t{gi}"), 0x03, rty, &mut arg_items);
        n_args += 1;
    }
    // `f` is the comp-func INDEX (the arg value); `wire` is the 0-based wire NAME index (`import-func-f<n>`,
    // matching the inner component). They advance together but name/value are distinct.
    let mut f = first_fn;
    let mut wire = 0usize;
    for gr in groups.iter() {
        for _ in &gr.makes {
            push(&import_wire_name(wire), 0x01, f, &mut arg_items);
            f += 1;
            wire += 1;
            n_args += 1;
        }
        push(&import_wire_name(wire), 0x01, f, &mut arg_items);
        f += 1;
        wire += 1;
        n_args += 1;
    }
    item.extend_from_slice(&wasm_vec(n_args, &arg_items));
    item
}

/// The DISTINCT-SIGNATURE ROUND-TRIP inner re-export component: like `resource_inner_component_distinct_sig`
/// but each group's functions are its makes (`(params…)->own<t_g>`) THEN its consumers (source-ordered
/// params via `consumer_functype`), rather than makes + one `call-g`. Imports G resources + all funcs typed
/// against their group's imported resource, then re-exports the G resources + all funcs ascribed against the
/// exported identity. Type-index layout identical to the distinct-sig one (own<t> + functype per fn, flat).
fn resource_inner_component_distinct_sig_rt(groups: &[RtSigGroupAbi]) -> Vec<u8> {
    let g = groups.len();
    let total_fns: usize = groups
        .iter()
        .map(|gr| gr.makes.len() + gr.consumers.len())
        .sum();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_subresource_item(&format!("import-type-t{gi}"))),
        ));
    }
    // IMPORT each fn (flat, group order: makes then consumers): own<t_g> (type g+2f) + functype (g+2f+1).
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        for mk in &gr.makes {
            let own_ty = (g + 2 * f) as u32;
            let ft_ty = (g + 2 * f + 1) as u32;
            out.extend_from_slice(&{
                let mut items = own_item(gi as u32);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(own_ty),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            f += 1;
        }
        for c in &gr.consumers {
            let _ = c;
            let own_ty = (g + 2 * f) as u32;
            let ft_ty = (g + 2 * f + 1) as u32;
            out.extend_from_slice(&{
                let mut items = own_item(gi as u32);
                items.extend_from_slice(&consumer_functype(own_ty, &c.params, c.result_byte));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            f += 1;
        }
    }
    // RE-EXPORT G resources → exported types E..E+g; then per fn re-ascribe against its group's exported rty.
    let e = (g + 2 * total_fns) as u32;
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_type_direct_item(&format!("t{gi}"), gi as u32)),
        ));
    }
    let mut ti = e + g as u32;
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        let exp_rty = e + gi as u32;
        for mk in &gr.makes {
            out.extend_from_slice(&{
                let mut items = own_item(exp_rty);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(ti),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_func_ascribed_item(&mk.name, f as u32, ti + 1)),
            ));
            ti += 2;
            f += 1;
        }
        for c in &gr.consumers {
            out.extend_from_slice(&{
                let mut items = own_item(exp_rty);
                items.extend_from_slice(&consumer_functype(ti, &c.params, c.result_byte));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_func_ascribed_item(&c.name, f as u32, ti + 1)),
            ));
            ti += 2;
            f += 1;
        }
    }
    out
}

/// The distinct-signature round-trip instantiate item: each resource + each fn (makes then consumers per
/// group), matching `resource_inner_component_distinct_sig_rt`'s import names + order.
fn component_instantiate_distinct_sig_rt_item(
    res_type_idx: &[u32],
    first_fn: u32,
    groups: &[RtSigGroupAbi],
) -> Vec<u8> {
    let mut item = vec![0x00];
    uleb128(0, &mut item);
    let mut arg_items = Vec::new();
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    let mut n_args = 0usize;
    for (gi, &rty) in res_type_idx.iter().enumerate() {
        push(&format!("import-type-t{gi}"), 0x03, rty, &mut arg_items);
        n_args += 1;
    }
    // `f` is the comp-func INDEX (arg value); `wire` is the 0-based wire NAME index matching the inner component.
    let mut f = first_fn;
    let mut wire = 0usize;
    for gr in groups.iter() {
        for _ in &gr.makes {
            push(&import_wire_name(wire), 0x01, f, &mut arg_items);
            f += 1;
            wire += 1;
            n_args += 1;
        }
        for _ in &gr.consumers {
            push(&import_wire_name(wire), 0x01, f, &mut arg_items);
            f += 1;
            wire += 1;
            n_args += 1;
        }
    }
    item.extend_from_slice(&wasm_vec(n_args, &arg_items));
    item
}

/// The MULTI-EXPORT-plus-CONSUMER (round-trip) inner re-export component: imports the abstract resource +
/// N `import-func-<make>` (each `(params…) -> own<t>`) + M `import-func-<consumer>` (each `(g: own<t>,
/// args…) -> R`), then re-exports the resource + all N+M funcs ascribed. A make and a consumer are both
/// "own<t>-shaped" component funcs, so they interleave uniformly here (makes first, then consumers), each
/// contributing an `own<0>` + functype pair on import and an `own<R>` + functype pair on export. Type-index
/// layout mirrors `resource_inner_component_multi_closure` with M extra funcs appended.
fn resource_inner_component_roundtrip(
    makes: &[ClosureMakeAbi],
    consumers: &[ClosureConsumeAbi],
) -> Vec<u8> {
    let nfns = makes.len() + consumers.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // IMPORT each fn: make[i] `(params…) -> own<0>`, then consumer[j] `(g: own<0>, args…) -> R`. Each pins
    // `own<0>` (type 1+2f) + its functype (type 2+2f), then imports the func → func f.
    let mut f = 0usize;
    for mk in makes {
        let own_ty = (1 + 2 * f) as u32;
        let ft_ty = (2 + 2 * f) as u32;
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
            &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
        ));
        f += 1;
    }
    for c in consumers {
        let _ = c;
        let own_ty = (1 + 2 * f) as u32;
        let ft_ty = (2 + 2 * f) as u32;
        out.extend_from_slice(&{
            let mut items = own_item(0);
            items.extend_from_slice(&consumer_functype(own_ty, &c.params, c.result_byte));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
        ));
        f += 1;
    }
    // sec 11: RE-EXPORT the resource type 0 DIRECTLY as `t` → exported type R = 2*nfns+1.
    let r = (2 * nfns + 1) as u32;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // EXPORT each fn ascribed against the exported resource identity, in the same order.
    let mut f = 0usize;
    for mk in makes {
        let own_ty = r + (1 + 2 * f) as u32;
        let ft_ty = r + (2 + 2 * f) as u32;
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
            &wasm_vec(1, &export_func_ascribed_item(&mk.name, f as u32, ft_ty)),
        ));
        f += 1;
    }
    for c in consumers {
        let own_ty = r + (1 + 2 * f) as u32;
        let ft_ty = r + (2 + 2 * f) as u32;
        out.extend_from_slice(&{
            let mut items = own_item(r);
            items.extend_from_slice(&consumer_functype(own_ty, &c.params, c.result_byte));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_func_ascribed_item(&c.name, f as u32, ft_ty)),
        ));
        f += 1;
    }
    out
}

/// The round-trip instantiate item: supply the resource type + each make (`import-func-<make>` → comp func
/// `first_fn + i`) + each consumer (`import-func-<consumer>` → the following comp funcs). The inner
/// component imports under these same names, makes first then consumers.
fn component_instantiate_roundtrip_item(
    res_ty: u32,
    first_fn: u32,
    makes: &[ClosureMakeAbi],
    consumers: &[ClosureConsumeAbi],
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
    let mut f = 0u32; // 0-based wire index; comp func = first_fn + f, name = import-func-f<f>
    for _ in makes {
        push(
            &import_wire_name(f as usize),
            0x01,
            first_fn + f,
            &mut arg_items,
        );
        f += 1;
    }
    for _ in consumers {
        push(
            &import_wire_name(f as usize),
            0x01,
            first_fn + f,
            &mut arg_items,
        );
        f += 1;
    }
    item.extend_from_slice(&wasm_vec(1 + makes.len() + consumers.len(), &arg_items));
    item
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

/// The inner re-export component for make + encode + N extra SCALAR methods (VM-1/VM-2). Generalizes
/// `resource_inner_component_borrow` (N=0) and the former `_borrow_len` (N=1). Imports the abstract
/// resource + make + encode + each scalar method, then RE-EXPORTS the resource directly and re-declares
/// every method against the exported identity. BYTE-IDENTICAL to `tests::…::inner_reexport_component_*`
/// (the ComponentBuilder reference). Component defined-type progression, in emission order:
///   IMPORTS: own<0> 1, make-ft 2, borrow<0> 3, list 4, encode-ft 5, then per method i: borrow<0>
///            (6+2i), method-ft (7+2i). So after M methods, next type = 6 + 2M.
///   RE-EXPORT `t` → type `E` = 6 + 2M.
///   EXPORT re-decls: own<E> (E+1), make-ft (E+2), borrow<E> (E+3), list (E+4), encode-ft (E+5), then per
///            method i: borrow<E> (E+6+2i), method-ft (E+7+2i).
/// Funcs: make 0, encode 1, method i = 2+i.
fn resource_inner_component_scalar_methods(methods: &[ScalarMethod]) -> Vec<u8> {
    let m = methods.len() as u32;
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: own<0> (type 1) + make functype `() -> own<0>` (type 2).
    out.extend_from_slice(&{
        let mut items = own_item(0);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 10: import `import-func-make` : type 2 → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: borrow<0> (type 3), list u8 (type 4), encode functype (type 5).
    out.extend_from_slice(&{
        let mut items = borrow_item(0);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(3, 4));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    });
    // sec 10: import `import-func-encode` : type 5 → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-encode", 5)),
    ));
    // Per method i (IMPORT side): borrow<0> (type 6+2i) + method functype (type 7+2i), then import
    // `import-func-<name>` : that functype → func 2+i. A `list<u8>` result reuses the encode `list u8`
    // type 4 (identity-free); a scalar uses its primitive byte.
    for (i, meth) in methods.iter().enumerate() {
        let bt = 6 + 2 * i as u32;
        let ft = match meth.result {
            MethodResult::Scalar(prim) => self_borrow_to_scalar_functype(bt, prim),
            MethodResult::ListU8 => self_borrow_to_list_functype(bt, 4),
        };
        out.extend_from_slice(&{
            let mut items = borrow_item(0);
            items.extend_from_slice(&ft);
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(
                1,
                &import_func_item(&format!("import-func-{}", meth.boundary_name), bt + 1),
            ),
        ));
    }
    // sec 11: RE-EXPORT the resource directly as `t` → exported type E = 6 + 2M.
    let e = 6 + 2 * m;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: own<E> (type E+1) + make functype re-typed (type E+2).
    out.extend_from_slice(&{
        let mut items = own_item(e);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(e + 1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 11: export `make` (func 0) ascribed to functype E+2.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, e + 2)),
    ));
    // sec 7: borrow<E> (type E+3), list u8 (type E+4), encode functype re-typed (type E+5).
    out.extend_from_slice(&{
        let mut items = borrow_item(e);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(e + 3, e + 4));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    });
    // sec 11: export `encode` (func 1) ascribed to functype E+5.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(ENCODE_BOUNDARY_NAME, 1, e + 5),
        ),
    ));
    // Per method i (EXPORT side): borrow<E> (type E+6+2i) + method functype re-typed (type E+7+2i), then
    // export `<name>` (func 2+i) ascribed to that functype. A `list<u8>` result reuses the export-side
    // `list u8` type E+4.
    for (i, meth) in methods.iter().enumerate() {
        let bt = e + 6 + 2 * i as u32;
        let ft = match meth.result {
            MethodResult::Scalar(prim) => self_borrow_to_scalar_functype(bt, prim),
            MethodResult::ListU8 => self_borrow_to_list_functype(bt, e + 4),
        };
        out.extend_from_slice(&{
            let mut items = borrow_item(e);
            items.extend_from_slice(&ft);
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(
                1,
                &export_func_ascribed_item(meth.boundary_name, 2 + i as u32, bt + 1),
            ),
        ));
    }
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

/// A component functype `(self: borrow<t>) -> <scalar prim>`: `40 01 <"self"> <borrow-valtype> 00
/// <prim-byte>` — like [`self_borrow_to_list_functype`] but the result is a PRIMITIVE valtype (its
/// negative-space byte, e.g. `COMP_U32`), not a defined-type index. Used for a scalar-result value-resource
/// method such as `len : borrow<t> -> u32` (`bytes-len`/`vec-len` over the borrow rep) — a method that
/// needs NO Memory/Realloc canon options (nothing crosses through linear memory). `borrow_type_idx` is the
/// component-type index of the `borrow<t>` defined type laid just before the functype.
/// (`#[allow(dead_code)]`: wired into the value-resource envelope in the `len`-method increment; the
/// byte-shape test below already exercises it, mirroring how `borrow_item` was staged before its use.)
#[allow(dead_code)]
fn self_borrow_to_scalar_functype(borrow_type_idx: u32, result_prim: u8) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x01];
    item.extend_from_slice(&uleb_bytes("self".len() as u64));
    item.extend_from_slice(b"self");
    item.extend_from_slice(&owned_valtype(borrow_type_idx));
    item.push(0x00); // result form: one result
    item.push(result_prim); // a primitive valtype byte (not a type index)
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

/// The `call` functype for a COMPOUND-RESULT closure: `(self: own<t>, args…) -> list<u8>` — like
/// [`closure_call_functype`] but the result references the `list<u8>` DEFINED type by index (not an inline
/// scalar byte). `self_handle_type_idx` is the `own<t>` defined type; `list_type_idx` the `list<u8>` type
/// laid just before this functype. Its lift carries Memory/Realloc (the caller uses `canon_lift_list_item`).
fn closure_call_list_functype(
    self_handle_type_idx: u32,
    arg_bytes: &[u8],
    list_type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    for (i, &vt) in arg_bytes.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(1 + arg_bytes.len(), &param_items));
    // One result — the `list<u8>` defined type, referenced by index.
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A round-trip CONSUMER's component functype: its params in SOURCE ORDER (each an `own<t>` closure handle
/// or a scalar byte) → `result_byte`. Unlike [`closure_call_functype`] (which hardcodes `own<t>` FIRST +
/// scalar args), this follows the actual param order — so a closure param may sit anywhere, and there may
/// be several (all `own<t>` of the same resource). `own_ty` is the `own<t>` defined-type index every
/// closure param references. Params named `p0`,`p1`,… (positional; names cosmetic).
fn consumer_functype(own_ty: u32, params: &[ConsumeParamAbi], result_byte: u8) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, p) in params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        match p {
            ConsumeParamAbi::Closure => param_items.extend_from_slice(&owned_valtype(own_ty)),
            ConsumeParamAbi::Scalar(vt) => param_items.push(*vt),
        }
    }
    item.extend_from_slice(&wasm_vec(params.len(), &param_items));
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

/// The PRIVATE wiring name for the `f`-th function an inner re-export component imports from the outer
/// envelope. Indexed (`import-func-f0`, `f1`, …) rather than the user export name, because a user name may
/// not be valid kebab-case (e.g. `mkA` — wasmtime rejects a non-kebab extern name at parse time), whereas
/// this internal name is always kebab. The instantiate item pairs its args by this same `f<i>` sequence,
/// so the wiring stays consistent. The HOST-facing EXPORT names still use the user names (component export
/// extern names are unrestricted); only these internal imports are indexed.
fn import_wire_name(f: usize) -> String {
    format!("import-func-f{f}")
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
    // This is a PUBLIC component-boundary export name — it MUST be kebab-case (wasmtime rejects a
    // non-kebab extern name). A closure export named from source (`make-<src>`, a consumer's own name)
    // may carry uppercase/underscore (`mkA`, `my_func`); normalize it the same way `comp_export_item`
    // does for a bare scalar export. Already-kebab names (`make`, `call`, `call-g0`, `make-adder`) are
    // the identity, so the byte layout of every existing corpus case is unchanged. The runner resolves
    // a source-derived name through the SAME `kebab_extern_name` rule, so both sides agree.
    let name = crate::backend::wasm::kebab_extern_name(name);
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

/// Like [`component_instantiate_item`] but for the value-resource-with-scalar-methods inner component
/// (VM-1/VM-2): the three fixed args (`import-type-t`=`res_ty`, `import-func-make`=`first_fn`,
/// `import-func-encode`=`first_fn+1`) plus one `import-func-<name>` per scalar method (comp func
/// `first_fn+2+i`, in method order). The inner component ([`resource_inner_component_scalar_methods`])
/// imports under these same names.
fn component_instantiate_scalar_methods_item(
    res_ty: u32,
    first_fn: u32,
    methods: &[ScalarMethod],
) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (component 0)
    let mut arg_items = Vec::new();
    let mut n_args = 0usize;
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    push("import-type-t", 0x03, res_ty, &mut arg_items);
    push("import-func-make", 0x01, first_fn, &mut arg_items);
    push("import-func-encode", 0x01, first_fn + 1, &mut arg_items);
    n_args += 3;
    for (i, meth) in methods.iter().enumerate() {
        push(
            &format!("import-func-{}", meth.boundary_name),
            0x01,
            first_fn + 2 + i as u32,
            &mut arg_items,
        );
        n_args += 1;
    }
    item.extend_from_slice(&wasm_vec(n_args, &arg_items));
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
    let _ = makes;
    push("import-type-t", 0x03, res_ty, &mut arg_items);
    for i in 0..nmk {
        push(
            &import_wire_name(i),
            0x01,
            first_make_fn + i as u32,
            &mut arg_items,
        );
    }
    push(
        &import_wire_name(nmk),
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
///
/// This is a PLAIN `params → result` function type: nothing beyond the input and output — no resume
/// parameter, no suspension/trap arm on the result. A trap is the wasm-level out-of-band halt the
/// embedder observes, not a variant the result declares; how a host suspends/resumes a host call is its
/// own policy the ABI does not represent. The params and result each carry a boundary valtype fixed by
/// this contract (`BoundaryExport` built from `export_result`/param selection), lowered/lifted by the
/// same canonical-ABI convention any boundary value uses.
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# The entry's exported signature MUST be a plain function from the program's input type to its result type, carrying no additional outcome arm — no suspension outcome and no injected trap outcome — so that a run either returns its result value or halts out-of-band, and the interface declares nothing beyond `input -> output`.
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# The entry MUST NOT carry a resume parameter and its result MUST NOT encode a pending host call or a position in the program's execution, so that how a host call suspends and resumes is host runtime policy the ABI does not represent (capabilities-and-effects.md §A Host Call Returns A Response) and the same emitted bytes serve a host that answers inline, one that suspends a fiber and resumes in place, and one that tears down and replays from a log.
//= spec/contracts/component-abi.md#the-entry-signature-crosses-the-boundary-by-the-same-rules
//# The entry's parameter and result types MUST each have a boundary representation fixed by this contract.
//= spec/contracts/component-abi.md#the-entry-signature-crosses-the-boundary-by-the-same-rules
//# The entry's input and output MUST lower and lift across the boundary by the same calling convention as any other boundary value.
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
///
/// The extern name is NORMALIZED to kebab-case (`kebab_extern_name`): a source export name may be a
/// valid Cadenza identifier that is NOT a valid component extern name (an uppercase letter or underscore
/// — `fA`, `my_func`), which would make the component fail to validate. An already-kebab name (the
/// common case — every corpus export) normalizes to itself, so this is byte-identical for existing
/// programs. A collision (two source names → one extern name) is rejected at export planning, before
/// emit, so this site never silently merges two exports. The CORE module export + its alias keep the
/// verbatim source name (a valid core wasm name); only this component-boundary extern is kebab.
fn comp_export_item(name: &str, func_idx: u32) -> Vec<u8> {
    let extern_name = crate::backend::wasm::kebab_extern_name(name);
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(extern_name.len() as u64));
    item.extend_from_slice(extern_name.as_bytes());
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
            0x02,                         // param count = 2 (self + p0)
            0x04,
            b's',
            b'e',
            b'l',
            b'f', // "self"
            0x05, // own<t> defined type, index 5 (bare uleb)
            0x02,
            b'p',
            b'0', // "p0"
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
            0x04,
            b's',
            b'e',
            b'l',
            b'f',
            0x07, // own<t> index 7
            0x02,
            b'p',
            b'0',
            s64,
            0x02,
            b'p',
            b'1',
            s64,
            0x00,
            s64,
        ];
        assert_eq!(got2, want2, "two-arg call-method functype byte shape");
    }

    /// VM-1 (byte-neutral): a scalar-result value-resource method functype `(self: borrow<t>) -> u32`
    /// encodes to the exact component-model bytes — form `0x40`, a one-param vec `[self : borrow<t>]`,
    /// result `00 <u32-prim>`. `self` references the `borrow<t>` DEFINED type by index (a bare type-index
    /// uleb, same encoding as an `own<t>` valtype); the result is a PRIMITIVE valtype byte (`COMP_U32`),
    /// NOT a type index — the distinction that keeps a scalar method free of Memory/Realloc canon options.
    /// Pins the item shape so the envelope-wiring increment (adding `len` to the value resource) builds on
    /// a checked primitive, exactly as `closure_call_functype_encodes_the_call_method_shape` did for `call`.
    #[test]
    fn self_borrow_to_scalar_functype_encodes_a_scalar_method_shape() {
        // A `len : borrow<t> -> u32` whose `borrow<t>` is component defined-type index 4.
        let got = self_borrow_to_scalar_functype(4, wasm_abi::COMP_U32);
        let want: Vec<u8> = vec![
            wasm_abi::COMP_FUNCTYPE_FORM, // 0x40 functype form
            0x01,                         // param count = 1 (self only)
            0x04,
            b's',
            b'e',
            b'l',
            b'f',               // "self"
            0x04, // borrow<t> defined type, index 4 (bare uleb — same as an own<t> valtype)
            0x00, // result form: one result
            wasm_abi::COMP_U32, // result valtype u32 (a PRIMITIVE byte, not a type index)
        ];
        assert_eq!(
            got, want,
            "scalar value-resource method functype byte shape"
        );
    }
}
