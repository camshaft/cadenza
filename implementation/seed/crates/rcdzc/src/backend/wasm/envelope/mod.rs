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
use crate::wit_world::WitType;

mod inner;
use inner::*;

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

/// The PROVIDER shape (X4b, `DESIGN-cross-component-interop-rcdzc.md`): a component that publishes its
/// scalar boundary exports as members of a NAMED INTERFACE INSTANCE `iface` (`cadenza:pkg/iface`), so a
/// peer consumer's `(effect …)` `(bind "iface")` binds to it (the effects-unified surface, U2). Identical to [`assemble_bare`] through the
/// canon-lift (embed core, instantiate, one functype + alias + lift per export), but instead of exporting
/// each lifted func at TOP LEVEL, it bundles them into a COMPONENT INSTANCE (a component-instance
/// export-items section) and exports THAT one instance under the interface name. SCOPE: scalar/unit
/// exports (a `list<u8>`/compound export as an interface member is a later increment — declined upstream).
pub fn assemble_provider(core: &[u8], exports: &[BoundaryExport], iface: &str) -> Vec<u8> {
    let n = exports.len();

    // sec 7: one component functype per export.
    let mut type_items = Vec::new();
    for e in exports {
        type_items.extend_from_slice(&comp_functype(e, 0));
    }
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(n, &type_items));

    // sec 6: one core-func alias per export (off core-instance 0).
    let mut alias_items = Vec::new();
    for e in exports {
        alias_items.extend_from_slice(&core_alias_item(0, &e.name));
    }
    let alias_sec = section(sec::ALIAS, &wasm_vec(n, &alias_items));

    // sec 8: one canon-lift per export (component func i, using component type i).
    let mut canon_items = Vec::new();
    for i in 0..n {
        canon_items.extend_from_slice(&canon_lift_item(i as u32, i as u32));
    }
    let canon_sec = section(sec::CANON, &wasm_vec(n, &canon_items));

    // sec 5: a COMPONENT INSTANCE bundling the lifted funcs (export-items form `0x01`), each member
    // `<name> 0x01 <comp-func>` (`0x01` = ComponentExportKind::Func) — kebab-normalized member names so a
    // non-kebab export name is a valid interface member (matching the consumer's aliased name). Instance 0.
    let instance_sec = {
        let mut item = vec![0x01]; // export-items form
        let mut members = Vec::new();
        for (i, e) in exports.iter().enumerate() {
            let name = crate::backend::common::export_name::kebab_extern_name(&e.name);
            // Each member name is a component EXTERN NAME (`0x00 <len> <name>`), not a bare string.
            members.extend_from_slice(&extern_name(&name));
            members.push(0x01); // ComponentExportKind::Func
            uleb128(i as u64, &mut members);
        }
        item.extend_from_slice(&wasm_vec(n, &members));
        section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &item))
    };

    // sec 11: export the single instance (0) under the interface name (kebab-normalized).
    let export_sec = {
        let iface_name = crate::backend::common::export_name::kebab_extern_name(iface);
        section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_instance_item(&iface_name, 0)),
        )
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&core_module_section(core)); // 1
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &[0x01, 0x00, 0x00, 0x00])); // 2
    out.extend_from_slice(&type_sec); // 7
    out.extend_from_slice(&alias_sec); // 6
    out.extend_from_slice(&canon_sec); // 8
    out.extend_from_slice(&instance_sec); // 5: bundle the lifted funcs into one instance
    out.extend_from_slice(&export_sec); // 11: export the instance under the interface name
    out
}

/// §3c full-A — assemble the BYTES-ROUNDTRIP provider component for ANY WIT member (the fold's `apply` is
/// the first such member, not the contract): wrap a `core` module (which exports the member core-func under
/// `member_name` + `memory` + `cabi_realloc`, built by [`super::serialize::bytes_roundtrip_core_module`]) as
/// a component exporting interface `iface` with the single member `member_name : func(list<u8>) -> list<u8>`.
/// Combines the provider iface-INSTANCE export shape ([`assemble_provider`]) with the `list<u8>` param+result
/// canon lift (Memory+Realloc, [`canon_lift_list_item`], as [`assemble_bare_bytes`]): the ONE canon-lift's
/// Memory/Realloc options serve BOTH directions — the host lowers the incoming `list<u8>` document into the
/// guest's memory via `cabi_realloc`, the core value-decodes it, runs the member body, value-encodes the
/// result, and the lift reads the `(ptr,len)` result back out. The declared boundary is `list<u8>`↔`list<u8>`;
/// the compound param/result lives in the value-form document (DESIGN §3b). One member for now; widened to N
/// members as full-A grows.
///
/// The core module IMPORTS the value-heap runtime (its value-decode/encode + bytes-* ops), so — exactly
/// like [`assemble_with_imports`] — the component imports `cadenza:runtime/heap@…` as an instance, lowers
/// each op into a `"heap"` core instance, and instantiates the program module threading that instance in.
/// A bytes-roundtrip member ALWAYS imports the runtime (value-decode of the param + value-encode of the
/// result), so `imports` is never empty here; the wiring below is unconditional.
///
/// The bytes-roundtrip boundary type section (component type section, the "second" one): the shared
/// `list<u8>` defined type followed by the `apply(input: list<u8>) -> list<u8>` component functype whose
/// param and result reference that list type BY INDEX (its valtype is the uleb type index, not an inline
/// primitive byte). `list_type_idx` is the caller's component-type index for the `list<u8>` defined type —
/// it differs between providers (1 in the pure provider, 2 in the host-fused provider, which has an extra
/// preceding import instance-type), so the caller passes its own. Shared by
/// [`assemble_bytes_roundtrip_provider`] and [`assemble_bytes_roundtrip_host_provider`] (identical functype
/// form, only the index differs); mirrors the [`host_effect_instance_type`] extraction precedent.
fn bytes_roundtrip_boundary_type_sec(list_type_idx: u32) -> Vec<u8> {
    let mut type_items = list_u8_defined_type();
    let mut t = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut params = Vec::new();
    let pname = "input";
    params.extend_from_slice(&uleb_bytes(pname.len() as u64));
    params.extend_from_slice(pname.as_bytes());
    uleb128(list_type_idx as u64, &mut params); // param valtype = the list<u8> defined-type index
    t.extend_from_slice(&wasm_vec(1, &params));
    t.push(0x00); // result-form: one result
    uleb128(list_type_idx as u64, &mut t); // result valtype = the list<u8> defined-type index
    type_items.extend_from_slice(&t);
    section(sec::COMPONENT_TYPE, &wasm_vec(2, &type_items))
}

/// Index spaces (with `k = imports.len()`):
///   * lowered ops → core funcs `0..k`; `apply` alias → core func `k`; `cabi_realloc` alias → core func
///     `k+1`; `memory` alias → memory 0 (a memory alias takes no func index).
///   * import instance-type → component type 0; `list<u8>` defined type → component type 1; the apply
///     functype → component type 2 (its `list<u8>` param/result reference the list type by index 1).
///   * op aliases → component funcs `0..k`; the apply lift → component func `k`.
///   * heap-exports core-instance → core instance 0; program → core instance 1 (its exports the aliases read).
pub fn assemble_bytes_roundtrip_provider(
    core: &[u8],
    iface: &str,
    member_name: &str,
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    let k = imports.len();
    let list_type_idx: u32 = 1; // component type 0 is the import instance-type; the list type follows it
    let apply_functype_idx: u32 = 2;
    let apply_core_func: u32 = k as u32; // after the k lowered ops
    let realloc_core_func: u32 = k as u32 + 1;

    // sec 7 (first): the import instance-type — component type 0. A `ty` decl (the op's component functype)
    // then an `export` decl naming the op, INTERLEAVED per op — mirrors `assemble_with_imports`.
    let instance_type = runtime_op_instance_type(imports);
    let import_type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1, &instance_type));

    // sec 10: import the runtime interface as an instance of component type 0.
    let import_sec = {
        let mut item = extern_name(import_name);
        item.push(0x05); // ComponentTypeRef::Instance sort
        uleb128(0, &mut item); // type index 0
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };

    // sec 6 (first): alias each op out of the imported instance (component instance 0) → component funcs 0..k.
    let op_alias_sec = {
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    };

    // sec 8 (first): canon-lower each aliased op (component func i) → core funcs 0..k.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    };

    // sec 2: TWO core instances — (0) the lowered ops exported under their names (the `"heap"` instance);
    // (1) the program module instantiated with `"heap"` bound to instance 0.
    let core_instance_sec = {
        let mut items = Vec::new();
        let mut heap = vec![0x01]; // export-items form
        let mut heap_exports = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00); // ExportKind::Func
            uleb128(i as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        // instance 1: instantiate module 0 with one arg `"heap" = core instance 0`.
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

    // sec 6 (second): alias the apply core-func (core func k), the memory (memory 0), and cabi_realloc
    // (core func k+1) out of the PROGRAM instance (core instance 1).
    let member_alias_sec = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(1, member_name));
        items.extend_from_slice(&memory_alias_item(1, "memory"));
        items.extend_from_slice(&core_alias_item(1, "cabi_realloc"));
        section(sec::ALIAS, &wasm_vec(3, &items))
    };

    // sec 7 (second): the shared `list<u8>` defined type (component type 1), then the member functype
    // (component type 2): `(input: list<u8>) -> list<u8>` — the defined-type param/result reference the
    // list type by INDEX (its valtype is the uleb type index), not an inline primitive byte.
    let boundary_type_sec = bytes_roundtrip_boundary_type_sec(list_type_idx);

    // sec 8 (second): lift apply (core func k) with Memory(0) + Realloc(core func k+1), apply functype
    // (component type 2) → component func k.
    let lift_sec = section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item(apply_core_func, 0, realloc_core_func, apply_functype_idx),
        ),
    );

    // sec 5: bundle the lifted apply (component func k) into one component instance, member = kebab(name).
    let instance_sec = {
        let mut item = vec![0x01]; // export-items form
        let mname = crate::backend::common::export_name::kebab_extern_name(member_name);
        let mut members = Vec::new();
        members.extend_from_slice(&extern_name(&mname));
        members.push(0x01); // ComponentExportKind::Func
        uleb128(k as u64, &mut members); // component func k (the apply lift)
        item.extend_from_slice(&wasm_vec(1, &members));
        section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &item))
    };

    // sec 11: export the bundled instance under the interface name (kebab-normalized). The imported
    // RUNTIME instance is component-instance 0; the bundle (`instance_sec`) is component-instance 1 — so
    // export index 1 (exporting 0 re-exports the imported heap ops as unimplemented reexports, which
    // wasmtime rejects at load: "arr-alloc is a reexport of an imported function which is not implemented").
    let export_sec = {
        let iface_name = crate::backend::common::export_name::kebab_extern_name(iface);
        section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_instance_item(&iface_name, 1)),
        )
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&import_type_sec); // 7: import instance-type (component type 0)
    out.extend_from_slice(&import_sec); // 10: component import of the runtime interface
    out.extend_from_slice(&op_alias_sec); // 6: alias ops out of the import
    out.extend_from_slice(&lower_sec); // 8: lower ops → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: embedded program
    out.extend_from_slice(&core_instance_sec); // 2: heap-instance + program-instance
    out.extend_from_slice(&member_alias_sec); // 6: alias apply/memory/realloc off the program
    out.extend_from_slice(&boundary_type_sec); // 7: list<u8> type + apply functype
    out.extend_from_slice(&lift_sec); // 8: lift apply
    out.extend_from_slice(&instance_sec); // 5: bundle into the provider interface-instance
    out.extend_from_slice(&export_sec); // 11: export the instance under iface
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

// ── Typed INTERFACE export (step W4c) ─────────────────────────────────────────────────────────────────
// A guest that exports funcs carrying NAMED WIT types (records/variants) cannot export them at the
// component top level — the component-model validator rejects a top-level func export whose signature
// references a named type ("func not valid to be used as export"; only structural `list`/`option`/`result`
// work bare, which is why the `list<u8>` bare shape above validates). Such funcs must live inside an
// exported INSTANCE (a WIT interface) that also exports the named types, so the interface is self-contained.
// This assembler emits that shape: the reducer world's `export guest`.

/// One func of a typed interface: its name + WIT param/result types (the per-contract payload rides as a
/// `list<u8>` leaf, §12 — the ENVELOPE record/variant is what is typed here).
#[allow(dead_code)]
pub struct TypedFunc {
    pub name: String,
    pub params: Vec<(String, WitType)>,
    pub result: Option<WitType>,
}

/// A WIT interface a guest exports as a component instance: the named types it exports (each must be
/// exported by name for the interface to be self-contained) and its funcs.
#[allow(dead_code)]
pub struct TypedInterface {
    /// The fully-qualified interface name the instance is exported under, e.g. `cadenza:platform/guest`.
    pub name: String,
    /// The named value types the interface exports (declaration order); each becomes an indexed defined
    /// type the instance re-exports by name, and a func param/result of the same type references it.
    pub types: Vec<(String, WitType)>,
    /// The interface's funcs; the embedded `core` module must export, per func, a core function named
    /// `<func.name>` whose signature is that func's canonical-ABI flattening.
    pub funcs: Vec<TypedFunc>,
}

/// Add `ty` to `table`, DEDUPING against the whole-type occurrences already in `memo` — so a func param of
/// a named interface type references that named type's index rather than a duplicate (matching how a WIT
/// `func(m: msg)` references the interface's `record msg`). Top-level dedup: nested shared sub-types may
/// still duplicate, which is valid (structural), just not minimal.
fn add_memo(
    ty: &WitType,
    table: &mut Vec<crate::backend::wasm::wit_ctype::CDef>,
    memo: &mut Vec<(WitType, crate::backend::wasm::wit_ctype::CRef)>,
) -> crate::backend::wasm::wit_ctype::CRef {
    // Dedup at EVERY level (not just the top-level `ty`): a nested compound shared between a named type and a
    // func's param — or between two params — must resolve to one table index, so the exported interface
    // instance re-exports a self-consistent type set (a field references an exported type, never a duplicate).
    crate::backend::wasm::wit_ctype::add_wit_type_deduped(ty, table, memo)
        .expect("an interface type / param / result must be a value type")
}

/// Assemble a component that exports the WIT interface `iface` as an instance (the reducer world's
/// `export guest`). Section order: core module(1), core-instance(2), func aliases(6), type section(7:
/// defined types then functypes), canon lifts(8), the exported instance(5: `FromExports` of the named
/// types + the lifted funcs), the top-level instance export(11).
///
/// A func whose signature touches linear memory — a `list`/`string` leaf anywhere, or a spilling
/// param/result (see [`wit_ctype::sig_needs_memory`]) — lifts with the Memory+Realloc canon options, and the
/// embedded `core` module must then export `memory` + `cabi_realloc` (aliased after the func aliases). A
/// pure fixed-scalar func lifts with no options and needs neither. Per func `j`, the core exports a function
/// named `<func.name>` whose signature is that func's canonical-ABI flattening.
#[allow(dead_code)]
pub fn assemble_typed_interface(core: &[u8], iface: &TypedInterface) -> Vec<u8> {
    use crate::backend::wasm::wit_ctype::{CRef, emit_cdef, emit_functype};
    // One func's lifted signature: its (name, type-ref) params and its optional result type-ref.
    type Sig = (Vec<(String, CRef)>, Option<CRef>);
    let m = iface.funcs.len();

    // Shared defined-type table: the named types first (their indices are what the instance re-exports),
    // then each func's param/result types (deduped so a func param of a named type reuses its index).
    let mut table = Vec::new();
    let mut memo: Vec<(WitType, CRef)> = Vec::new();
    let mut named: Vec<(String, CRef)> = Vec::new();
    for (n, ty) in &iface.types {
        named.push((n.clone(), add_memo(ty, &mut table, &mut memo)));
    }
    let mut sigs: Vec<Sig> = Vec::new();
    for f in &iface.funcs {
        let prefs = f
            .params
            .iter()
            .map(|(n, ty)| (n.clone(), add_memo(ty, &mut table, &mut memo)))
            .collect();
        let rref = f
            .result
            .as_ref()
            .map(|t| add_memo(t, &mut table, &mut memo));
        sigs.push((prefs, rref));
    }
    let functype_base = table.len() as u32;

    // Which funcs' lift touches linear memory (a `list`/`string` leaf, or a spilling param/result) — those
    // lift with Memory+Realloc and require the core to export `memory` + `cabi_realloc`. If ANY does, the
    // core-func aliases (0..m) are followed by a `memory` alias (memory 0) and a `cabi_realloc` alias (core
    // func m), and those funcs lift with the options bound.
    let needs_mem: Vec<bool> = iface
        .funcs
        .iter()
        .map(|f| {
            let ptys: Vec<WitType> = f.params.iter().map(|(_, t)| t.clone()).collect();
            crate::backend::wasm::wit_ctype::sig_needs_memory(&ptys, f.result.as_ref())
        })
        .collect();
    let any_mem = needs_mem.iter().any(|&b| b);
    let mem_idx: u32 = 0;
    let realloc_func: u32 = m as u32; // core func after the m func aliases

    // sec 6: alias each func's core export (core funcs 0..m); then, if any func needs memory, alias the
    // core's `memory` and `cabi_realloc`.
    let mut alias_items = Vec::new();
    for f in &iface.funcs {
        alias_items.extend_from_slice(&core_alias_item(0, &f.name));
    }
    let n_aliases = if any_mem {
        alias_items.extend_from_slice(&memory_alias_item(0, "memory"));
        alias_items.extend_from_slice(&core_alias_item(0, "cabi_realloc"));
        m + 2
    } else {
        m
    };
    let alias_sec = section(sec::ALIAS, &wasm_vec(n_aliases, &alias_items));

    // sec 7: defined types (component types 0..d), then one functype per func (types d..d+m).
    let mut type_items = Vec::new();
    for def in &table {
        type_items.extend_from_slice(&emit_cdef(def));
    }
    for (prefs, rref) in &sigs {
        type_items.extend_from_slice(&emit_functype(prefs, rref.as_ref()));
    }
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(table.len() + m, &type_items));

    // sec 8: canon lift func j (core func j) with its functype (component type functype_base + j) — a
    // memory-touching func binds Memory + Realloc, a pure-scalar func lifts with no options.
    let mut canon_items = Vec::new();
    for (j, &nm) in needs_mem.iter().enumerate() {
        let type_idx = functype_base + j as u32;
        if nm {
            canon_items.extend_from_slice(&canon_lift_list_item(
                j as u32,
                mem_idx,
                realloc_func,
                type_idx,
            ));
        } else {
            canon_items.extend_from_slice(&canon_lift_item(j as u32, type_idx));
        }
    }
    let canon_sec = section(sec::CANON, &wasm_vec(m, &canon_items));

    // sec 5: the exported instance, built `FromExports` (tag 0x01) — export each named type then each lifted
    // func. Item grammar: `<00 extern-name-kind=plain> <namelen> <name> <sort> <idx>`; sorts are type=0x03,
    // func=0x01.
    let mut inst_items = Vec::new();
    for (n, r) in &named {
        let idx = match r {
            CRef::Idx(i) => *i,
            CRef::Prim(_) => {
                panic!("a named interface type must be a compound (an indexed defined type)")
            }
        };
        inst_items.push(0x00); // extern-name kind: plain
        inst_items.extend_from_slice(&uleb_bytes(n.len() as u64));
        inst_items.extend_from_slice(n.as_bytes());
        inst_items.push(0x03); // sort: type
        uleb128(idx as u64, &mut inst_items);
    }
    for (j, f) in iface.funcs.iter().enumerate() {
        inst_items.push(0x00); // extern-name kind: plain
        inst_items.extend_from_slice(&uleb_bytes(f.name.len() as u64));
        inst_items.extend_from_slice(f.name.as_bytes());
        inst_items.push(0x01); // sort: func
        uleb128(j as u64, &mut inst_items);
    }
    let n_inst_exports = named.len() + m;
    let mut inst_def = vec![0x01]; // instance definition: FromExports
    uleb128(n_inst_exports as u64, &mut inst_def);
    inst_def.extend_from_slice(&inst_items);
    let instance_sec = section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &inst_def));

    // sec 11: export instance 0 under the interface name — `<00 kind=plain> <namelen> <name> <05 sort=instance> <idx> <00 no-type>`.
    let mut export_item = vec![0x00];
    export_item.extend_from_slice(&uleb_bytes(iface.name.len() as u64));
    export_item.extend_from_slice(iface.name.as_bytes());
    export_item.push(0x05); // sort: instance
    uleb128(0u64, &mut export_item); // the sole instance
    export_item.push(0x00); // no type ascription
    let export_sec = section(sec::COMPONENT_EXPORT, &wasm_vec(1, &export_item));

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&core_module_section(core)); // sec 1
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &[0x01, 0x00, 0x00, 0x00])); // sec 2
    out.extend_from_slice(&alias_sec); // sec 6
    out.extend_from_slice(&type_sec); // sec 7
    out.extend_from_slice(&canon_sec); // sec 8
    out.extend_from_slice(&instance_sec); // sec 5
    out.extend_from_slice(&export_sec); // sec 11
    out
}

/// [`assemble_typed_interface`] for a guest that IMPORTS the value-heap runtime (`imports` non-empty) — a
/// record/variant wrapper builds guest values via `arr-alloc`/`box-*`, so the emitted core imports
/// `cadenza:runtime/heap`. Composes the runtime import (import the instance, alias+canon-lower each op to a
/// core func, instantiate the core threading them as `"heap"`) — the shape [`assemble_provider_runtime`]
/// uses — with this module's TYPED defined types + functypes and the interface-instance export.
///
/// MVP (matches `mod.rs::record_interface_export`): scalar-field record params + scalar/unit results — no
/// `list<u8>`/`string` leaf, so no Memory/Realloc canon options and the core needs no memory. Index spaces
/// (`k = imports`, `m = funcs`, `d = defined types`): component types — defined types `0..d`, the import
/// instance-type `d`, functypes `d+1..d+1+m`; component funcs — op aliases `0..k`, lifts `k..k+m`; core funcs
/// — lowered ops `0..k`, boundary aliases `k..k+m`; component instances — imported runtime `0`, the exported
/// bundle `1`. Laying the defined types FIRST (before the import instance-type) keeps `wit_ctype`'s 0-based
/// [`CRef`] indices valid.
#[allow(dead_code)]
pub fn assemble_typed_interface_with_runtime(
    core: &[u8],
    iface: &TypedInterface,
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    use crate::backend::wasm::wit_ctype::{CRef, emit_cdef, emit_functype};
    type Sig = (Vec<(String, CRef)>, Option<CRef>);
    let k = imports.len();
    let m = iface.funcs.len();

    let mut table = Vec::new();
    let mut memo: Vec<(WitType, CRef)> = Vec::new();
    let mut named: Vec<(String, CRef)> = Vec::new();
    for (n, ty) in &iface.types {
        named.push((n.clone(), add_memo(ty, &mut table, &mut memo)));
    }
    let mut sigs: Vec<Sig> = Vec::new();
    for f in &iface.funcs {
        let prefs = f
            .params
            .iter()
            .map(|(n, ty)| (n.clone(), add_memo(ty, &mut table, &mut memo)))
            .collect();
        let rref = f
            .result
            .as_ref()
            .map(|t| add_memo(t, &mut table, &mut memo));
        sigs.push((prefs, rref));
    }
    let d = table.len();
    let functype_base = (d + 1) as u32; // defined types 0..d, import instance-type d, functypes d+1..

    // sec 7: DEFINED types (comp types 0..d; CRefs are 0-based so they land at their own index), then the
    // import INSTANCE-TYPE (comp type d), then the per-func functypes (d+1..d+1+m).
    let type_sec = {
        let mut items = Vec::new();
        for def in &table {
            items.extend_from_slice(&emit_cdef(def));
        }
        items.extend_from_slice(&runtime_op_instance_type(imports));
        for (prefs, rref) in &sigs {
            items.extend_from_slice(&emit_functype(prefs, rref.as_ref()));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(d + 1 + m, &items))
    };
    // sec 10: import the runtime interface as an instance of component type `d`.
    let import_sec = {
        let mut item = extern_name(import_name);
        item.push(0x05); // ComponentTypeRef::Instance
        uleb128(d as u64, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };
    // sec 6 (first): alias each op out of the imported instance (comp instance 0) → comp funcs 0..k.
    let op_alias_sec = {
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    };
    // sec 8 (first): canon-lower each aliased op → core funcs 0..k.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    };
    // sec 2: heap core instance (0, the lowered ops) + program core instance (1, bound to `"heap"`).
    let core_instance_sec = {
        let mut items = Vec::new();
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128(i as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        let mut prog = vec![0x00];
        uleb128(0, &mut prog);
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(0, &mut args);
        prog.extend_from_slice(&wasm_vec(1, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(2, &items))
    };
    // A boundary func whose signature TOUCHES LINEAR MEMORY (a `list<u8>` leaf in a record param, a spilling
    // sig) lifts with Memory+Realloc options: the canon lift lowers the incoming list into the program's
    // memory, and the wrapper reads it out (`emit_bytes_leaf_copy_in`). `sig_needs_memory` here mirrors the
    // core's `wrapper_needs_memory` (both keyed off the SAME WIT signatures), so the two agree on which funcs
    // and whether the program exports `memory` + `cabi_realloc` at all.
    let touches = |f: &TypedFunc| {
        let ptys: Vec<WitType> = f.params.iter().map(|(_, t)| t.clone()).collect();
        crate::backend::wasm::wit_ctype::sig_needs_memory(&ptys, f.result.as_ref())
    };
    let needs_memory = iface.funcs.iter().any(touches);
    // `cabi_realloc`'s CORE func index — aliased right after the m boundary funcs (present iff needs_memory).
    let realloc_core_func = (k + m) as u32;
    // sec 6 (second): alias each func's core export off the PROGRAM instance (core instance 1) → k..k+m;
    // when memory is needed, also alias its `memory` (core memory 0, no func index) + `cabi_realloc` (core
    // func k+m) so the lift's Memory+Realloc options can reference them.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for f in &iface.funcs {
            items.extend_from_slice(&core_alias_item(1, &f.name));
        }
        let mut count = m;
        if needs_memory {
            items.extend_from_slice(&memory_alias_item(1, "memory"));
            items.extend_from_slice(&core_alias_item(1, "cabi_realloc"));
            count += 2;
        }
        section(sec::ALIAS, &wasm_vec(count, &items))
    };
    // sec 8 (second): lift each boundary core func (`k+j`) with its functype (`functype_base + j`) — no
    // options for a pure-scalar sig, Memory(0)+Realloc(k+m) for a memory-touching one.
    let lift_sec = {
        let mut items = Vec::new();
        for (j, f) in iface.funcs.iter().enumerate() {
            if touches(f) {
                items.extend_from_slice(&canon_lift_list_item(
                    (k + j) as u32,
                    0,
                    realloc_core_func,
                    functype_base + j as u32,
                ));
            } else {
                items.extend_from_slice(&canon_lift_item((k + j) as u32, functype_base + j as u32));
            }
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };
    // sec 5: the exported instance (FromExports) — the named types + the lifted funcs (comp funcs k+j).
    let instance_sec = {
        let mut inst_items = Vec::new();
        for (n, r) in &named {
            let idx = match r {
                CRef::Idx(i) => *i,
                CRef::Prim(_) => panic!("a named interface type must be a compound"),
            };
            inst_items.push(0x00);
            inst_items.extend_from_slice(&uleb_bytes(n.len() as u64));
            inst_items.extend_from_slice(n.as_bytes());
            inst_items.push(0x03); // sort: type
            uleb128(idx as u64, &mut inst_items);
        }
        for (j, f) in iface.funcs.iter().enumerate() {
            inst_items.push(0x00);
            inst_items.extend_from_slice(&uleb_bytes(f.name.len() as u64));
            inst_items.extend_from_slice(f.name.as_bytes());
            inst_items.push(0x01); // sort: func
            uleb128((k + j) as u64, &mut inst_items);
        }
        let mut inst_def = vec![0x01];
        uleb128((named.len() + m) as u64, &mut inst_def);
        inst_def.extend_from_slice(&inst_items);
        section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &inst_def))
    };
    // sec 11: export the bundled instance (component instance 1 — the imported runtime is instance 0).
    let export_sec = {
        let mut item = vec![0x00];
        item.extend_from_slice(&uleb_bytes(iface.name.len() as u64));
        item.extend_from_slice(iface.name.as_bytes());
        item.push(0x05); // sort: instance
        uleb128(1u64, &mut item);
        item.push(0x00);
        section(sec::COMPONENT_EXPORT, &wasm_vec(1, &item))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: defined types + import instance-type + functypes
    out.extend_from_slice(&import_sec); // 10: component import of the runtime
    out.extend_from_slice(&op_alias_sec); // 6: alias ops out of the import
    out.extend_from_slice(&lower_sec); // 8: lower ops → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: the embedded program
    out.extend_from_slice(&core_instance_sec); // 2: heap-instance + program-instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&instance_sec); // 5: the exported interface instance
    out.extend_from_slice(&export_sec); // 11: export the instance
    out
}

/// [`assemble_typed_interface_with_runtime`] but exporting each lifted func at the COMPONENT TOP LEVEL
/// (a bare `func` export) instead of bundling them into a named interface INSTANCE. The plain-export
/// (no imposed WIT world) entry path: a compiled `main` whose param is memory-bearing (`String`/`Bytes`,
/// crossing as `string`/`list<u8>`) needs the SAME wrapper + runtime + Memory/Realloc lift as a typed
/// record-param export, but the driver calls the sole TOP-LEVEL func export by name (`get_func("main")`),
/// not an interface member — so the lifted funcs are exported directly, not wrapped in an instance.
///
/// SCOPE (entry-param slice 1): no NAMED interface types (a `String`/`Bytes`/scalar param references no
/// defined-type NAME — a `list<u8>` is an anonymous defined type, laid in the table but never a named
/// export); a compound (record/variant) top-level param, which WOULD need a named type export, is not on
/// this path (it declines upstream). Index spaces are identical to
/// [`assemble_typed_interface_with_runtime`] through the lift; only the final instance-bundle + instance
/// export are replaced by per-func top-level exports.
pub fn assemble_bare_typed_with_runtime(
    core: &[u8],
    funcs: &[TypedFunc],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    use crate::backend::wasm::wit_ctype::{CRef, emit_cdef, emit_functype};
    type Sig = (Vec<(String, CRef)>, Option<CRef>);
    let k = imports.len();
    let m = funcs.len();

    let mut table = Vec::new();
    let mut memo: Vec<(WitType, CRef)> = Vec::new();
    let mut sigs: Vec<Sig> = Vec::new();
    for f in funcs {
        let prefs = f
            .params
            .iter()
            .map(|(n, ty)| (n.clone(), add_memo(ty, &mut table, &mut memo)))
            .collect();
        let rref = f
            .result
            .as_ref()
            .map(|t| add_memo(t, &mut table, &mut memo));
        sigs.push((prefs, rref));
    }
    let d = table.len();
    let functype_base = (d + 1) as u32; // defined types 0..d, import instance-type d, functypes d+1..

    // sec 7: DEFINED types (comp types 0..d), then the runtime import INSTANCE-TYPE (comp type d), then the
    // per-func functypes (d+1..d+1+m).
    let type_sec = {
        let mut items = Vec::new();
        for def in &table {
            items.extend_from_slice(&emit_cdef(def));
        }
        items.extend_from_slice(&runtime_op_instance_type(imports));
        for (prefs, rref) in &sigs {
            items.extend_from_slice(&emit_functype(prefs, rref.as_ref()));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(d + 1 + m, &items))
    };
    // sec 10: import the runtime interface as an instance of component type `d`.
    let import_sec = {
        let mut item = extern_name(import_name);
        item.push(0x05); // ComponentTypeRef::Instance
        uleb128(d as u64, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };
    // sec 6 (first): alias each op out of the imported instance (comp instance 0) → comp funcs 0..k.
    let op_alias_sec = {
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    };
    // sec 8 (first): canon-lower each aliased op → core funcs 0..k.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    };
    // sec 2: heap core instance (0, the lowered ops) + program core instance (1, bound to `"heap"`).
    let core_instance_sec = {
        let mut items = Vec::new();
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128(i as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        let mut prog = vec![0x00];
        uleb128(0, &mut prog);
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(0, &mut args);
        prog.extend_from_slice(&wasm_vec(1, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(2, &items))
    };
    let touches = |f: &TypedFunc| {
        let ptys: Vec<WitType> = f.params.iter().map(|(_, t)| t.clone()).collect();
        crate::backend::wasm::wit_ctype::sig_needs_memory(&ptys, f.result.as_ref())
    };
    let needs_memory = funcs.iter().any(touches);
    let realloc_core_func = (k + m) as u32;
    // sec 6 (second): alias each func's core export off the PROGRAM instance (core instance 1) → k..k+m;
    // plus `memory` + `cabi_realloc` when a memory-bearing param needs the Memory/Realloc lift options.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for f in funcs {
            items.extend_from_slice(&core_alias_item(1, &f.name));
        }
        let mut count = m;
        if needs_memory {
            items.extend_from_slice(&memory_alias_item(1, "memory"));
            items.extend_from_slice(&core_alias_item(1, "cabi_realloc"));
            count += 2;
        }
        section(sec::ALIAS, &wasm_vec(count, &items))
    };
    // sec 8 (second): lift each boundary core func (`k+j`) with its functype (`functype_base + j`) —
    // Memory(0)+Realloc(k+m) for a memory-touching sig, no options for a pure-scalar one.
    let lift_sec = {
        let mut items = Vec::new();
        for (j, f) in funcs.iter().enumerate() {
            if touches(f) {
                items.extend_from_slice(&canon_lift_list_item(
                    (k + j) as u32,
                    0,
                    realloc_core_func,
                    functype_base + j as u32,
                ));
            } else {
                items.extend_from_slice(&canon_lift_item((k + j) as u32, functype_base + j as u32));
            }
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };
    // sec 11: export each lifted component func (`k+j`) under its verbatim boundary name (TOP-LEVEL, no
    // interface instance).
    let export_sec = {
        let mut items = Vec::new();
        for (j, f) in funcs.iter().enumerate() {
            items.extend_from_slice(&comp_export_item(&f.name, (k + j) as u32));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: defined types + import instance-type + functypes
    out.extend_from_slice(&import_sec); // 10: component import of the runtime
    out.extend_from_slice(&op_alias_sec); // 6: alias ops out of the import
    out.extend_from_slice(&lower_sec); // 8: lower ops → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: the embedded program
    out.extend_from_slice(&core_instance_sec); // 2: heap-instance + program-instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs (+ memory/realloc) off the program
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&export_sec); // 11: export the lifted funcs at top level
    out
}

/// [`assemble_typed_interface_with_runtime`] PLUS a HOST-effect import (W4c-b-iii) — a reducer guest that
/// EXPORTS the typed interface AND PERFORMS a world import (`identity.id`, `state.get`, …). Fuses this
/// module's two-import bookkeeping (`assemble_host_runtime`: host effect + runtime) with the typed
/// interface-instance export. SINGLE host effect (one interface — every host op shares `effect_iface`); a
/// multi-effect guest is a later increment. The wrapper `core` imports BOTH `"host"` (the h host ops) and
/// `"heap"` (the k runtime ops); `host_fns` are the host ops (their `comp_functype`s built by the caller with
/// the instance-local `(list u8)` index) and `effect_iface` is the host interface's FQ world name.
///
/// Index spaces (`h`=host ops, `k`=runtime ops, `m`=funcs, `d`=defined types): component types — defined
/// `0..d`, host instance-type `d`, runtime instance-type `d+1`, functypes `d+2..d+2+m`; component funcs — host
/// op aliases `0..h`, runtime op aliases `h..h+k`, lifts `h+k..h+k+m`; core funcs — lowered host `0..h`,
/// lowered runtime `h..h+k`, boundary aliases `h+k..h+k+m`, `cabi_realloc` alias `h+k+m` (if memory);
/// component instances — imported host `0`, imported runtime `1`, the exported bundle `2`; core instances —
/// host ops `0`, runtime ops `1`, program `2`.
#[allow(clippy::too_many_arguments)]
pub fn assemble_typed_interface_with_host_runtime(
    core: &[u8],
    iface: &TypedInterface,
    groups: &[HostGroup],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    use crate::backend::common::export_name::kebab_extern_name;
    use crate::backend::wasm::wit_ctype::{CRef, emit_cdef, emit_functype};
    type Sig = (Vec<(String, CRef)>, Option<CRef>);
    // Each interface is its own imported component instance-type; the ops FLATTEN across groups into one
    // ordered core-func run (group 0's ops, then group 1's, …), invisible to the core side (all bind under
    // one `"host"` module). `G == 1` is byte-identical to the single-interface emit.
    let g = groups.len();
    let all_host_fns: Vec<&HostFn> = groups.iter().flat_map(|gr| &gr.host_fns).collect();
    let h = all_host_fns.len();
    let k = imports.len();
    let m = iface.funcs.len();

    let mut table = Vec::new();
    let mut memo: Vec<(WitType, CRef)> = Vec::new();
    let mut named: Vec<(String, CRef)> = Vec::new();
    for (n, ty) in &iface.types {
        named.push((n.clone(), add_memo(ty, &mut table, &mut memo)));
    }
    let mut sigs: Vec<Sig> = Vec::new();
    for f in &iface.funcs {
        let prefs = f
            .params
            .iter()
            .map(|(n, ty)| (n.clone(), add_memo(ty, &mut table, &mut memo)))
            .collect();
        let rref = f
            .result
            .as_ref()
            .map(|t| add_memo(t, &mut table, &mut memo));
        sigs.push((prefs, rref));
    }
    let d = table.len();
    // defined 0..d, host instance-types d..d+g, runtime instance-type d+g, functypes d+g+1..
    let functype_base = (d + g + 1) as u32;

    // sec 7: defined types (comp types 0..d), the g host instance-types (comp types d..d+g), the runtime
    // instance-type (comp type d+g), then the m per-func functypes (d+g+1..).
    let type_sec = {
        let mut items = Vec::new();
        for def in &table {
            items.extend_from_slice(&emit_cdef(def));
        }
        for gr in groups {
            items.extend_from_slice(&host_effect_instance_type(
                &gr.host_fns,
                gr.needs_list,
                &gr.result_defs,
                &gr.record_defs,
            ));
        }
        items.extend_from_slice(&runtime_op_instance_type(imports));
        for (prefs, rref) in &sigs {
            items.extend_from_slice(&emit_functype(prefs, rref.as_ref()));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(d + g + 1 + m, &items))
    };
    // sec 10: import each host interface (instance of comp type d+i, under its kebab FQ name) → comp instance
    // i, then the runtime (instance of comp type d+g, under import_name) → comp instance g.
    let import_sec = {
        let mut items = Vec::new();
        for (i, gr) in groups.iter().enumerate() {
            let mut eff = extern_name(&kebab_extern_name(&gr.effect_iface));
            eff.push(0x05); // ComponentTypeRef::Instance
            uleb128((d + i) as u64, &mut eff);
            items.extend_from_slice(&eff);
        }
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128((d + g) as u64, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(g + 1, &items))
    };
    // sec 6 (first): alias each group's ops out of ITS comp instance i (in group order → comp funcs 0..h),
    // then k runtime ops out of comp instance g (→ comp funcs h..h+k).
    let op_alias_sec = {
        let mut items = Vec::new();
        for (i, gr) in groups.iter().enumerate() {
            for f in &gr.host_fns {
                items.extend_from_slice(&comp_alias_item(i as u32, &kebab_extern_name(&f.op)));
            }
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(g as u32, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
    };
    // sec 8 (first): lower each aliased op (comp funcs 0..h+k) → core funcs 0..h+k.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    };
    // sec 2: THREE core instances — (0) lowered HOST ops (ALL groups, flattened) under their op names →
    // `"host"`; (1) lowered RUNTIME ops under their names → `"heap"`; (2) the program bound to BOTH.
    let core_instance_sec = {
        let mut items = Vec::new();
        let mut host = vec![0x01];
        let mut host_exports = Vec::new();
        for (i, f) in all_host_fns.iter().enumerate() {
            host_exports.extend_from_slice(&uleb_bytes(f.op.len() as u64));
            host_exports.extend_from_slice(f.op.as_bytes());
            host_exports.push(0x00); // ExportKind::Func
            uleb128(i as u64, &mut host_exports);
        }
        host.extend_from_slice(&wasm_vec(h, &host_exports));
        items.extend_from_slice(&host);
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (j, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128((h + j) as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        let mut prog = vec![0x00]; // instantiate
        uleb128(0, &mut prog); // module 0
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HOST_MODULE.len() as u64));
        args.extend_from_slice(HOST_MODULE.as_bytes());
        args.push(0x12); // ModuleArg::Instance
        uleb128(0, &mut args); // core instance 0
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(1, &mut args); // core instance 1
        prog.extend_from_slice(&wasm_vec(2, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(3, &items))
    };
    // Does any boundary func's signature touch linear memory (a `list<u8>`/`string` leaf, or a spilling sig)?
    let touches = |f: &TypedFunc| {
        let ptys: Vec<WitType> = f.params.iter().map(|(_, t)| t.clone()).collect();
        crate::backend::wasm::wit_ctype::sig_needs_memory(&ptys, f.result.as_ref())
    };
    let needs_memory = iface.funcs.iter().any(touches);
    let realloc_core_func = (h + k + m) as u32;
    // sec 6 (second): alias each boundary func off the PROGRAM instance (core instance 2) → core funcs
    // h+k..h+k+m; when memory is needed, also its `memory` + `cabi_realloc`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for f in &iface.funcs {
            items.extend_from_slice(&core_alias_item(2, &f.name));
        }
        let mut count = m;
        if needs_memory {
            items.extend_from_slice(&memory_alias_item(2, "memory"));
            items.extend_from_slice(&core_alias_item(2, "cabi_realloc"));
            count += 2;
        }
        section(sec::ALIAS, &wasm_vec(count, &items))
    };
    // sec 8 (second): lift each boundary core func (h+k+j) with its functype (functype_base+j).
    let lift_sec = {
        let mut items = Vec::new();
        for (j, f) in iface.funcs.iter().enumerate() {
            if touches(f) {
                items.extend_from_slice(&canon_lift_list_item(
                    (h + k + j) as u32,
                    0,
                    realloc_core_func,
                    functype_base + j as u32,
                ));
            } else {
                items.extend_from_slice(&canon_lift_item(
                    (h + k + j) as u32,
                    functype_base + j as u32,
                ));
            }
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };
    // sec 5: the exported instance (FromExports) — named types + the m lifted funcs (comp funcs h+k+j).
    let instance_sec = {
        let mut inst_items = Vec::new();
        for (n, r) in &named {
            let idx = match r {
                CRef::Idx(i) => *i,
                CRef::Prim(_) => panic!("a named interface type must be a compound"),
            };
            inst_items.push(0x00);
            inst_items.extend_from_slice(&uleb_bytes(n.len() as u64));
            inst_items.extend_from_slice(n.as_bytes());
            inst_items.push(0x03); // sort: type
            uleb128(idx as u64, &mut inst_items);
        }
        for (j, f) in iface.funcs.iter().enumerate() {
            inst_items.push(0x00);
            inst_items.extend_from_slice(&uleb_bytes(f.name.len() as u64));
            inst_items.extend_from_slice(f.name.as_bytes());
            inst_items.push(0x01); // sort: func
            uleb128((h + k + j) as u64, &mut inst_items);
        }
        let mut inst_def = vec![0x01];
        uleb128((named.len() + m) as u64, &mut inst_def);
        inst_def.extend_from_slice(&inst_items);
        section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &inst_def))
    };
    // sec 11: export the bundled instance (comp instance g+1 — the g imported host instances 0..g, then the
    // imported runtime g, then this FromExports bundle).
    let export_sec = {
        let mut item = vec![0x00];
        item.extend_from_slice(&uleb_bytes(iface.name.len() as u64));
        item.extend_from_slice(iface.name.as_bytes());
        item.push(0x05); // sort: instance
        uleb128((g + 1) as u64, &mut item);
        item.push(0x00);
        section(sec::COMPONENT_EXPORT, &wasm_vec(1, &item))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: defined types + host-its + runtime-it + functypes
    out.extend_from_slice(&import_sec); // 10: import host interfaces + runtime
    out.extend_from_slice(&op_alias_sec); // 6: alias host ops then runtime ops
    out.extend_from_slice(&lower_sec); // 8: lower both op sets → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: the embedded program (imports "host" + "heap")
    out.extend_from_slice(&core_instance_sec); // 2: host-instance + heap-instance + program-instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&instance_sec); // 5: the exported interface instance
    out.extend_from_slice(&export_sec); // 11: export the instance
    out
}

/// [`assemble_typed_interface_with_host_runtime`] for a host op that needs LINEAR MEMORY (a `list<u8>`/
/// `string` param, or a `list<u8>`/`option<list<u8>>`/`list<tuple>` RESULT the guest lifts): the host op's
/// canon LOWER carries a Memory option, which needs the memory available at lower-time (before the program
/// core), so the component composes the SHARED `"mem"` module (like [`assemble_bytes_roundtrip_host_provider`])
/// and the wrapper `core` IMPORTS `"mem"`.`"mem"` as its memory (its `list<u8>`-leaf handling + the host-op
/// lift share the one memory). Merges that mem-shape wiring with the typed interface-instance export.
///
/// Index spaces (`g`/`h`/`k`/`m`/`d` = interface/host-op/runtime/func/defined-type counts, `h` summed over
/// all `g` groups): component types — defined `0..d`, the `g` host instance-types `d..d+g`, runtime
/// instance-type `d+g`, functypes `d+g+1..`; component funcs — host op aliases `0..h` (group order), runtime
/// `h..h+k`, lifts `h+k..h+k+m`; core funcs — lowered host `0..h`, lowered runtime `h..h+k`, boundary aliases
/// `h+k..h+k+m`, `cabi_realloc` `h+k+m`; component instances — imported host `0..g`, imported runtime `g`,
/// exported bundle `g+1`; CORE instances — mem `0`, host ops `1`, runtime ops `2`, program `3`; core modules —
/// mem `0`, program `1`; core memory `0` (the shared `"mem"`). Group boundaries are invisible to the core side
/// (all host ops bind under one `"host"` module by name); `g == 1` is byte-identical to the single-interface emit.
#[allow(clippy::too_many_arguments)]
pub fn assemble_typed_interface_with_host_runtime_mem(
    core: &[u8],
    iface: &TypedInterface,
    groups: &[HostGroup],
    imports: &[&RtOp],
    import_name: &str,
    // A host op with a COMPOUND result needs the SHARED cabi_realloc (from the mem module) at lower-time: the
    // mem module exports memory + cabi_realloc, both aliased BEFORE the host-op lowers. cabi_realloc becomes
    // CORE FUNC 0, so every lowered op / boundary / lift core-func index shifts by +1 (`rs`). When false
    // (a `list<u8>`/`string` PARAM + scalar/unit result), the mem module exports memory only and the program
    // owns its own defined cabi_realloc — no shift.
    needs_realloc: bool,
) -> Vec<u8> {
    use crate::backend::common::export_name::kebab_extern_name;
    use crate::backend::wasm::wit_ctype::{CRef, emit_cdef, emit_functype};
    type Sig = (Vec<(String, CRef)>, Option<CRef>);
    // g imported host instance-types (one per interface); host ops FLATTEN across groups into one core-func
    // run under `"host"` (invisible to the core side). `g == 1` is byte-identical to the single-interface emit.
    let g = groups.len();
    let all_host_fns: Vec<&HostFn> = groups.iter().flat_map(|gr| &gr.host_fns).collect();
    let h = all_host_fns.len();
    let k = imports.len();
    let m = iface.funcs.len();
    let rs = needs_realloc as u32; // core-func shift: +1 when cabi_realloc is aliased as core func 0

    let mut table = Vec::new();
    let mut memo: Vec<(WitType, CRef)> = Vec::new();
    let mut named: Vec<(String, CRef)> = Vec::new();
    for (n, ty) in &iface.types {
        named.push((n.clone(), add_memo(ty, &mut table, &mut memo)));
    }
    let mut sigs: Vec<Sig> = Vec::new();
    for f in &iface.funcs {
        let prefs = f
            .params
            .iter()
            .map(|(n, ty)| (n.clone(), add_memo(ty, &mut table, &mut memo)))
            .collect();
        let rref = f
            .result
            .as_ref()
            .map(|t| add_memo(t, &mut table, &mut memo));
        sigs.push((prefs, rref));
    }
    let d = table.len();
    // defined 0..d, host instance-types d..d+g, runtime instance-type d+g, functypes d+g+1..
    let functype_base = (d + g + 1) as u32;

    let type_sec = {
        let mut items = Vec::new();
        for def in &table {
            items.extend_from_slice(&emit_cdef(def));
        }
        for gr in groups {
            items.extend_from_slice(&host_effect_instance_type(
                &gr.host_fns,
                gr.needs_list,
                &gr.result_defs,
                &gr.record_defs,
            ));
        }
        items.extend_from_slice(&runtime_op_instance_type(imports));
        for (prefs, rref) in &sigs {
            items.extend_from_slice(&emit_functype(prefs, rref.as_ref()));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(d + g + 1 + m, &items))
    };
    let import_sec = {
        let mut items = Vec::new();
        for (i, gr) in groups.iter().enumerate() {
            let mut eff = extern_name(&kebab_extern_name(&gr.effect_iface));
            eff.push(0x05);
            uleb128((d + i) as u64, &mut eff);
            items.extend_from_slice(&eff);
        }
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128((d + g) as u64, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(g + 1, &items))
    };
    let op_alias_sec = {
        let mut items = Vec::new();
        for (i, gr) in groups.iter().enumerate() {
            for f in &gr.host_fns {
                items.extend_from_slice(&comp_alias_item(i as u32, &kebab_extern_name(&f.op)));
            }
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(g as u32, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
    };
    // sec 1 (first): the SHARED-MEMORY module (core module 0) — with a bump `cabi_realloc` when a compound
    // host result needs the shared allocator; sec 2 (first): instantiate it → core instance 0; sec 6 (mem
    // alias): alias `mem`.`mem` → core memory 0, AND (realloc mode) `mem`.`cabi_realloc` → CORE FUNC 0 — both
    // BEFORE the host-op lowers, so a compound-result host op's lower can reference the realloc.
    let mem_module_sec = core_module_section(&if needs_realloc {
        shared_mem_realloc_module()
    } else {
        shared_mem_module()
    });
    let mem_instance_sec = section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[])),
    );
    let mem_alias_sec = if needs_realloc {
        let mut it = memory_alias_item(0, "mem");
        it.extend_from_slice(&core_alias_item(0, "cabi_realloc")); // → core func 0
        section(sec::ALIAS, &wasm_vec(2, &it))
    } else {
        section(sec::ALIAS, &wasm_vec(1, &memory_alias_item(0, "mem")))
    };
    // sec 8 (first): lower host ops WITH the Memory option (core memory 0) — plus a Realloc option (the shared
    // cabi_realloc, core func 0) in realloc mode so a compound host RESULT is allocated into the shared memory;
    // runtime ops memoryless. The lowered ops are core funcs `rs..rs+h+k` (core func 0 is the realloc alias in
    // realloc mode).
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..h {
            if needs_realloc {
                items.extend_from_slice(&canon_lower_item_mem_realloc(i as u32, 0, 0));
            } else {
                items.extend_from_slice(&canon_lower_item_mem(i as u32, 0));
            }
        }
        for i in h..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    };
    // sec 1 (second): the embedded program (core module 1) — imports `"host"`+`"heap"`+`"mem"`.
    let prog_module_sec = core_module_section(core);
    // sec 2 (second): host ops (core instance 1), runtime ops (core instance 2), program (core instance 3,
    // bound `"host"`=1, `"heap"`=2, `"mem"`=0).
    let prog_instance_sec = {
        let mut items = Vec::new();
        let mut host = vec![0x01];
        let mut host_exports = Vec::new();
        for (i, f) in all_host_fns.iter().enumerate() {
            host_exports.extend_from_slice(&uleb_bytes(f.op.len() as u64));
            host_exports.extend_from_slice(f.op.as_bytes());
            host_exports.push(0x00);
            uleb128((rs + i as u32) as u64, &mut host_exports); // lowered host op i = core func rs+i
        }
        host.extend_from_slice(&wasm_vec(h, &host_exports));
        items.extend_from_slice(&host);
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (j, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128((rs + (h + j) as u32) as u64, &mut heap_exports); // runtime op j = core func rs+h+j
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        let mut prog = vec![0x00];
        uleb128(1, &mut prog); // module 1 (module 0 is the mem module)
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HOST_MODULE.len() as u64));
        args.extend_from_slice(HOST_MODULE.as_bytes());
        args.push(0x12);
        uleb128(1, &mut args); // "host" = core instance 1
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(2, &mut args); // "heap" = core instance 2
        args.extend_from_slice(&uleb_bytes("mem".len() as u64));
        args.extend_from_slice(b"mem");
        args.push(0x12);
        uleb128(0, &mut args); // "mem" = core instance 0
        prog.extend_from_slice(&wasm_vec(3, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(3, &items))
    };
    let touches = |f: &TypedFunc| {
        let ptys: Vec<WitType> = f.params.iter().map(|(_, t)| t.clone()).collect();
        crate::backend::wasm::wit_ctype::sig_needs_memory(&ptys, f.result.as_ref())
    };
    let needs_memory = iface.funcs.iter().any(touches);
    // The boundary lift's Realloc option: in realloc mode it is the SHARED cabi_realloc (core func 0, aliased
    // off the mem instance); otherwise the PROGRAM's own cabi_realloc, aliased off the program AFTER the m
    // boundary funcs (core func rs+h+k+m = h+k+m since rs=0 there).
    let realloc_core_func = if needs_realloc { 0 } else { (h + k + m) as u32 };
    // sec 6 (boundary alias): alias the m boundary funcs off the PROGRAM instance (core instance 3) → core
    // funcs `rs+h+k .. rs+h+k+m`. In DEFINE mode, ALSO alias the program's `cabi_realloc` (for the lift's
    // Realloc); in realloc mode the realloc is core func 0 (the shared allocator) and the program neither owns
    // nor exports one. The boundary lift's MEMORY is core memory 0 (the shared `"mem"`, aliased above).
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for f in &iface.funcs {
            items.extend_from_slice(&core_alias_item(3, &f.name));
        }
        let mut count = m;
        if needs_memory && !needs_realloc {
            items.extend_from_slice(&core_alias_item(3, "cabi_realloc"));
            count += 1;
        }
        section(sec::ALIAS, &wasm_vec(count, &items))
    };
    let lift_sec = {
        let mut items = Vec::new();
        for (j, f) in iface.funcs.iter().enumerate() {
            let boundary_core = rs + (h + k + j) as u32;
            if touches(f) {
                items.extend_from_slice(&canon_lift_list_item(
                    boundary_core,
                    0,
                    realloc_core_func,
                    functype_base + j as u32,
                ));
            } else {
                items.extend_from_slice(&canon_lift_item(boundary_core, functype_base + j as u32));
            }
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };
    let instance_sec = {
        let mut inst_items = Vec::new();
        for (n, r) in &named {
            let idx = match r {
                CRef::Idx(i) => *i,
                CRef::Prim(_) => panic!("a named interface type must be a compound"),
            };
            inst_items.push(0x00);
            inst_items.extend_from_slice(&uleb_bytes(n.len() as u64));
            inst_items.extend_from_slice(n.as_bytes());
            inst_items.push(0x03);
            uleb128(idx as u64, &mut inst_items);
        }
        for (j, f) in iface.funcs.iter().enumerate() {
            inst_items.push(0x00);
            inst_items.extend_from_slice(&uleb_bytes(f.name.len() as u64));
            inst_items.extend_from_slice(f.name.as_bytes());
            inst_items.push(0x01);
            uleb128((h + k + j) as u64, &mut inst_items);
        }
        let mut inst_def = vec![0x01];
        uleb128((named.len() + m) as u64, &mut inst_def);
        inst_def.extend_from_slice(&inst_items);
        section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &inst_def))
    };
    let export_sec = {
        let mut item = vec![0x00];
        item.extend_from_slice(&uleb_bytes(iface.name.len() as u64));
        item.extend_from_slice(iface.name.as_bytes());
        item.push(0x05);
        uleb128((g + 1) as u64, &mut item); // exported bundle = comp instance g+1 (g host + 1 runtime imported)
        item.push(0x00);
        section(sec::COMPONENT_EXPORT, &wasm_vec(1, &item))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7
    out.extend_from_slice(&import_sec); // 10
    out.extend_from_slice(&op_alias_sec); // 6: host + runtime op aliases
    out.extend_from_slice(&mem_module_sec); // 1: shared-memory module (module 0)
    out.extend_from_slice(&mem_instance_sec); // 2: mem instance (core instance 0)
    out.extend_from_slice(&mem_alias_sec); // 6: mem.mem → core memory 0
    out.extend_from_slice(&lower_sec); // 8: lower host (Memory) + runtime ops
    out.extend_from_slice(&prog_module_sec); // 1: program (module 1)
    out.extend_from_slice(&prog_instance_sec); // 2: host + heap + program instances
    out.extend_from_slice(&boundary_alias_sec); // 6: boundary funcs + cabi_realloc off the program
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&instance_sec); // 5: the exported interface instance
    out.extend_from_slice(&export_sec); // 11: export the instance
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
    let instance_type = runtime_op_instance_type(imports);
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

/// The PROVIDER + RUNTIME shape (X5c): a PROVIDER whose exports BUILD runtime values (importing the
/// value-heap runtime) AND publishes them as a named interface instance `iface` so a peer binds them.
/// Identical to [`assemble_with_imports`] through the lift, but bundles the lifted funcs into a COMPONENT
/// INSTANCE exported under `iface` (like [`assemble_provider`] does over the bare shape) instead of
/// top-level funcs. A compound export result crosses as its `u32` handle over the shared runtime.
pub fn assemble_provider_runtime(
    core: &[u8],
    exports: &[BoundaryExport],
    imports: &[&RtOp],
    import_name: &str,
    iface: &str,
) -> Vec<u8> {
    let k = imports.len();
    let m = exports.len();

    // sec 7: import instance-type (comp type 0) — the runtime ops.
    let type_sec = section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &runtime_op_instance_type(imports)),
    );
    // sec 10: import the runtime interface (comp instance 0).
    let import_sec = {
        let mut item = extern_name(import_name);
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };
    // sec 6 (first): alias each runtime op out of comp instance 0 → comp funcs `0..k`.
    let op_alias_sec = {
        let mut items = Vec::new();
        for op in imports {
            items.extend_from_slice(&comp_alias_item(0, op.name));
        }
        section(sec::ALIAS, &wasm_vec(k, &items))
    };
    // sec 8 (first): lower each aliased op → core funcs `0..k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..k {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(k, &items))
    };
    // sec 2: heap core instance (0) + program core instance (1) bound to `"heap"`.
    let core_instance_sec = {
        let mut items = Vec::new();
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (i, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128(i as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        let mut prog = vec![0x00];
        uleb128(0, &mut prog);
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(0, &mut args);
        prog.extend_from_slice(&wasm_vec(1, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(2, &items))
    };
    // sec 6 (second): alias each boundary func off the program instance (core instance 1) → `k..k+m`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(1, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };
    // sec 7 (second): one component functype per boundary export → comp types `1..=m`.
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            debug_assert!(e.result != BoundaryResult::Bytes);
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };
    // sec 8 (second): lift each boundary core func (`k+j`) using comp type `1+j` → comp funcs `k..k+m`.
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((k + j) as u32, (1 + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };
    // sec 5: bundle the lifted funcs (comp funcs `k..k+m`) into a component instance (export-items form).
    let instance_sec = {
        let mut item = vec![0x01];
        let mut members = Vec::new();
        for (j, e) in exports.iter().enumerate() {
            let name = crate::backend::common::export_name::kebab_extern_name(&e.name);
            members.extend_from_slice(&extern_name(&name));
            members.push(0x01); // ComponentExportKind::Func
            uleb128((k + j) as u64, &mut members);
        }
        item.extend_from_slice(&wasm_vec(m, &members));
        section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &item))
    };
    // sec 11: export the bundled instance under the interface name. The imported RUNTIME instance is
    // component-instance 0; the bundle (`instance_sec`) is component-instance 1 — so export index 1.
    let export_sec = {
        let iface_name = crate::backend::common::export_name::kebab_extern_name(iface);
        section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_instance_item(&iface_name, 1)),
        )
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7
    out.extend_from_slice(&import_sec); // 10
    out.extend_from_slice(&op_alias_sec); // 6
    out.extend_from_slice(&lower_sec); // 8
    out.extend_from_slice(&core_module_section(core)); // 1
    out.extend_from_slice(&core_instance_sec); // 2
    out.extend_from_slice(&boundary_alias_sec); // 6
    out.extend_from_slice(&boundary_type_sec); // 7
    out.extend_from_slice(&lift_sec); // 8
    out.extend_from_slice(&instance_sec); // 5: bundle into one instance (component-instance 1)
    out.extend_from_slice(&export_sec); // 11: export the instance under the interface name
    out
}

/// One host-import function the [`assemble_host`] shape imports: the operation NAME (the func the effect
/// interface exports) and its component functype BYTES (a `0x40 <params> <result>` item — the caller
/// builds it from the op's scalar signature). The declaring effect (the interface name) is a separate
/// argument since this increment delegates a SINGLE effect.
pub struct HostFn {
    pub op: String,
    /// The op's component functype item bytes (`0x40 …`) — declared in the effect's instance-type AND
    /// (re)used for the core import functype indirectly via the lowered form. When `has_list_param` is
    /// true, this functype references the shared `(list u8)` DEFINED type by INDEX (built with
    /// `host_op_comp_functype(h, 0)` — index 0 is the list type the instance-type prepends).
    pub comp_functype: Vec<u8>,
    /// The op's CORE functype item bytes (`0x60 <params> <results>`) — the type the program's core module
    /// imports the lowered op under. Built by the caller from the op's core valtypes.
    pub core_functype: Vec<u8>,
    /// Whether this op has a `list<u8>` (Bytes) parameter. When ANY host fn in the set does, the import
    /// instance-type PREPENDS a `(list u8)` defined type as its type index 0 (so a Bytes param's
    /// `comp_functype` reference resolves) and the per-op func types shift to 1..=h — the export decls
    /// reference `i+1`. A pure scalar/string set (`has_list_param` false for all) is byte-identical (no
    /// prepend, func types 0..h). See `list_u8_defined_type` + the export-side `comp_functype` mirror.
    pub has_list_param: bool,
}

/// One host INTERFACE a reducer delegates to — its FQ WIT import name (`cadenza:platform/graph`), the ops it
/// declares, and the component DEFINED types those ops reference (all LOCAL to this interface's own
/// instance-type index space). A reducer performing ops from N interfaces (e.g. `graph` + `deliver`) emits N
/// of these: each becomes one imported component instance-type with its own prepended `(list u8)` / spilled-
/// result / record-or-enum defined types (`host_effect_instance_type`), and each op's `comp_functype`
/// references those types by the LOCAL index the caller computed for THIS group. The types are per-group (not
/// component-wide) because an import instance-type must STRUCTURALLY match the host's interface — a spurious
/// type export another interface needs would make the host fail to satisfy this one. `needs_list` /
/// `result_defs` / `record_defs` are exactly the per-interface subset of what the single-interface path
/// computed globally; a single-group slice is byte-identical to the pre-multi-interface emit.
pub struct HostGroup {
    /// The FQ WIT import interface name (the component-import extern name), e.g. `cadenza:platform/deliver`.
    pub effect_iface: String,
    /// The ops of THIS interface the reducer performs, in emit order.
    pub host_fns: Vec<HostFn>,
    /// Whether this interface's instance-type prepends the shared `(list u8)` at its local index 0 (any
    /// `list<u8>` param or `list<u8>`-leaf spilled result among its ops).
    pub needs_list: bool,
    /// This interface's spilled-RESULT component defined types (children-first, deduped), laid after
    /// `(list u8)` — each op's result `CRef` indexes into them. The `bool` marks a NOMINAL def (`variant`/
    /// `enum`/`record`) that must be laid define+EXPORT (a result's err enum), not a bare anonymous define.
    pub result_defs: Vec<(Vec<u8>, bool)>,
    /// This interface's RECORD-or-ENUM param defined types (define+export pairs), laid after the result defs.
    pub record_defs: Vec<Vec<u8>>,
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
            decls.extend_from_slice(&extern_name(
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
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
        let mut item = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            iface,
        ));
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
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
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

/// The core-module import-bind name for a cross-component PEER interface (the analogue of
/// [`HOST_MODULE`] for a host effect / [`HEAP_MODULE`] for the runtime). A consumer component's core
/// imports each peer operation from this module; the envelope binds it to the imported peer instance.
const PEER_MODULE: &str = "peer";

/// The DISTINCT peer interfaces named in `op_ifaces`, in FIRST-APPEARANCE order — the order the extern
/// envelope imports them as component instances/types `0..g`. `op_ifaces[i]` is the interface op `i`
/// (in `extern_order`) is imported from; a single-interface consumer yields `["cadenza:pkg/iface"]`.
fn distinct_ifaces<'a>(op_ifaces: &[&'a str]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for &iface in op_ifaces {
        if !out.contains(&iface) {
            out.push(iface);
        }
    }
    out
}

/// The position of `iface` in the distinct-interface list — the component instance/type index the extern
/// envelope imports it under. `iface` is always present (it came from `op_ifaces`).
fn iface_index(ifaces: &[&str], iface: &str) -> usize {
    ifaces.iter().position(|&i| i == iface).unwrap_or(0)
}

/// The ops (in `extern_fns`/`extern_order` order) belonging to interface `iface` — those whose `op_ifaces`
/// entry names it. Used to declare each imported interface's instance-type with only ITS ops (the func
/// index in the instance-type is LOCAL to that instance, `0..ops-in-iface`).
fn peer_group_ops<'a>(
    extern_fns: &'a [HostFn],
    op_ifaces: &[&str],
    iface: &str,
) -> Vec<&'a HostFn> {
    extern_fns
        .iter()
        .zip(op_ifaces)
        .filter(|(_, oi)| **oi == iface)
        .map(|(f, _)| f)
        .collect()
}

/// The CROSS-COMPONENT import shape (X3, `DESIGN-cross-component-interop-rcdzc.md`): a CONSUMER component
/// that binds a PEER Cadenza component's exported interface `iface` and calls its operations across the
/// live component boundary. Structurally IDENTICAL to [`assemble_host`] — import an instance-type
/// declaring each peer op as a func, alias + lower each to a core func, bind them into the program core,
/// export the consumer's own boundary — differing only in (1) the imported instance is a peer Cadenza
/// interface rather than a host effect, and (2) the core binds it under module `"peer"` rather than
/// `"host"` (matching what the consumer core imports its peer ops from). The peer's ops carry MONOMORPHIC
/// signatures (component-abi.md §Generics Do Not Cross The Boundary; §The Exchanged Signature Is
/// Monomorphic). This X3 increment lands the envelope + a structural oracle; the front-end that emits a
/// consumer core importing `"peer"` and the runner that binds the peer instance arrive in X4. SCOPE:
/// scalar peer ops (a `value`-handle op is X5), and — like `assemble_host` — no value-heap runtime import
/// fused in yet (a peer op composing with the consumer's own runtime is a later increment, the analogue of
/// [`assemble_host_runtime`]).
///
/// Index spaces (`p = extern_fns.len()` peer ops, `m = exports.len()`): lowered peer ops → core funcs
/// `0..p`; boundary core-aliases → core funcs `p..p+m`. Peer instance-type → component type 0; boundary
/// functypes → component types `1..=m`. Peer op aliases → component funcs `0..p`; lifts → component funcs
/// `p..p+m`. Imported peer instance → component instance 0; peer core instance → core instance 0; program
/// → core instance 1.
///
/// MULTI-INTERFACE (U9): a consumer may bind MORE THAN ONE distinct peer interface. `op_ifaces[i]` names
/// the interface op `i` (in `extern_fns`, i.e. `extern_order`) is imported from; the distinct interfaces
/// (first-appearance order) become component instances/types `0..g`, and each op is aliased out of ITS
/// interface's instance. The ONE `"peer"` core instance still exports every lowered op FLAT by name (the
/// consumer core imports them all from `"peer"`), so op names must be globally unique across the bound
/// interfaces — the front-end declines a cross-interface collision. A single interface (`g == 1`, every
/// `op_ifaces` entry equal) reproduces the byte-exact X3 shape above.
pub fn assemble_extern(
    core: &[u8],
    exports: &[BoundaryExport],
    op_ifaces: &[&str],
    extern_fns: &[HostFn],
    publish_iface: Option<&str>,
) -> Vec<u8> {
    let p = extern_fns.len();
    let m = exports.len();
    // The distinct peer interfaces, in first-appearance order — imported as component instances `0..g`
    // (and instance-types `0..g`). `op_ifaces[i]` is the interface op `i` (in `extern_fns`/extern_order) is
    // imported from; a single interface (`g == 1`) reproduces the byte-exact X3 shape. See [`peer_groups`].
    let ifaces = distinct_ifaces(op_ifaces);
    let g = ifaces.len();

    // sec 7: one instance-type per distinct peer interface (component types `0..g`). Each declares ITS ops
    // (those whose `op_ifaces` names it), interleaved `ty` decl + `export` decl, the export's func index
    // LOCAL to that instance-type (`0..ops-in-iface`). For `g == 1` this is the single 2p-decl X3 shape.
    let type_sec = {
        let mut items = Vec::new();
        for iface in &ifaces {
            let ops = peer_group_ops(extern_fns, op_ifaces, iface);
            let mut decls = Vec::new();
            for (local, f) in ops.iter().enumerate() {
                decls.push(0x01); // ty decl
                decls.extend_from_slice(&f.comp_functype);
                decls.push(0x04); // export decl — the op's COMPONENT extern name (kebab-normalized).
                decls.extend_from_slice(&extern_name(
                    &crate::backend::common::export_name::kebab_extern_name(&f.op),
                ));
                decls.push(0x01); // sort: component func
                uleb128(local as u64, &mut decls);
            }
            let mut it = vec![0x42]; // instance type form
            it.extend_from_slice(&wasm_vec(2 * ops.len(), &decls));
            items.extend_from_slice(&it);
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(g, &items))
    };

    // sec 10: import each peer interface as an instance of its component type (`g_idx`), under its
    // (kebab-normalized) interface name → component instances `0..g`.
    let import_sec = {
        let mut items = Vec::new();
        for (g_idx, iface) in ifaces.iter().enumerate() {
            let mut item = extern_name(&crate::backend::common::export_name::kebab_extern_name(
                iface,
            ));
            item.push(0x05); // ComponentTypeRef::Instance sort
            uleb128(g_idx as u64, &mut item); // type index g_idx
            items.extend_from_slice(&item);
        }
        section(sec::COMPONENT_IMPORT, &wasm_vec(g, &items))
    };

    // sec 6 (first): alias each op (flat, in extern_order) out of ITS interface's imported instance →
    // component funcs `0..p`, by the op's kebab-normalized component extern name. A single interface aliases
    // every op from instance 0 (byte-identical to X3).
    let op_alias_sec = {
        let mut items = Vec::new();
        for (f, &oi) in extern_fns.iter().zip(op_ifaces) {
            let inst = iface_index(&ifaces, oi);
            items.extend_from_slice(&comp_alias_item(
                inst as u32,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        section(sec::ALIAS, &wasm_vec(p, &items))
    };

    // sec 8 (first): canon-lower each aliased peer op (component func `i`) → core funcs `0..p`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..p {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(p, &items))
    };

    // sec 2: TWO core instances — (0) the lowered peer ops exported under their names, forming the
    // `"peer"` instance; (1) the program module instantiated with `"peer"` bound to instance 0.
    let core_instance_sec = {
        let mut items = Vec::new();
        let mut peer = vec![0x01]; // export-items form
        let mut peer_exports = Vec::new();
        for (i, f) in extern_fns.iter().enumerate() {
            peer_exports.extend_from_slice(&uleb_bytes(f.op.len() as u64));
            peer_exports.extend_from_slice(f.op.as_bytes());
            peer_exports.push(0x00); // ExportKind::Func
            uleb128(i as u64, &mut peer_exports);
        }
        peer.extend_from_slice(&wasm_vec(p, &peer_exports));
        items.extend_from_slice(&peer);
        // instance 1: instantiate module 0 with one arg `"peer" = instance 0`.
        let mut prog = vec![0x00]; // instantiate form
        uleb128(0, &mut prog); // module index 0
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(PEER_MODULE.len() as u64));
        args.extend_from_slice(PEER_MODULE.as_bytes());
        args.push(0x12); // ModuleArg::Instance sort
        uleb128(0, &mut args); // core instance 0
        prog.extend_from_slice(&wasm_vec(1, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(2, &items))
    };

    // sec 6 (second): alias each boundary func off the PROGRAM instance (core instance 1) → core funcs
    // `p..p+m`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(1, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };

    // sec 7 (second): one component functype per boundary export → component types `g..g+m` (after the g
    // peer instance-types `0..g`; `g == 1` gives `1..=m`, the X3 shape).
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            debug_assert!(
                e.result != BoundaryResult::Bytes,
                "a list<u8> boundary result takes the resource path, not the extern shape"
            );
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };

    // sec 8 (second): lift each boundary core func (`p+j`) using its component type (`g+j`) → component
    // funcs `p..p+m`.
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((p + j) as u32, (g + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };

    // sec 5 + 11 (publish) OR sec 11 (top-level): when this component is ALSO a provider (`publish_iface`
    // = `Some(iface)`, a MIDDLE-of-chain component that both binds a peer and publishes its own interface,
    // U11), BUNDLE the lifted boundary funcs (comp funcs `p..p+m`) into a component instance and export
    // THAT under `iface` — exactly the `assemble_provider` shape, but with the peer imports/lowers ahead of
    // it. The bundle is comp instance `g` (after the g imported peer instances `0..g`). Otherwise
    // (`None`, a pure consumer) export each lifted func at TOP LEVEL under its verbatim name (byte-identical
    // to the X3 shape — the `instance_sec` is empty and never appended).
    let (instance_sec, export_sec) = match publish_iface {
        Some(iface) => {
            let mut item = vec![0x01]; // export-items form
            let mut members = Vec::new();
            for (j, e) in exports.iter().enumerate() {
                let name = crate::backend::common::export_name::kebab_extern_name(&e.name);
                members.extend_from_slice(&extern_name(&name));
                members.push(0x01); // ComponentExportKind::Func
                uleb128((p + j) as u64, &mut members);
            }
            item.extend_from_slice(&wasm_vec(m, &members));
            let instance_sec = section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &item));
            let iface_name = crate::backend::common::export_name::kebab_extern_name(iface);
            let export_sec = section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_instance_item(&iface_name, g as u32)),
            );
            (instance_sec, export_sec)
        }
        None => {
            let mut items = Vec::new();
            for (j, e) in exports.iter().enumerate() {
                items.extend_from_slice(&comp_export_item(&e.name, (p + j) as u32));
            }
            (
                Vec::new(),
                section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items)),
            )
        }
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: peer instance-type
    out.extend_from_slice(&import_sec); // 10: component import of the peer interface
    out.extend_from_slice(&op_alias_sec); // 6: alias peer ops out of the import
    out.extend_from_slice(&lower_sec); // 8: lower peer ops → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: embedded consumer program
    out.extend_from_slice(&core_instance_sec); // 2: peer-instance + program-instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&boundary_type_sec); // 7: boundary functypes
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&instance_sec); // 5: (publish) bundle lifts into an instance — empty for a consumer
    out.extend_from_slice(&export_sec); // 11: export the instance (publish) or each func (top-level)
    out
}

/// The CROSS-COMPONENT + RUNTIME composed shape (X5): a CONSUMER that binds a PEER interface `peer_iface`
/// AND uses the value-heap runtime — the case where a compound value crosses from the peer as an opaque
/// `u32` handle and the consumer INSPECTS it (a projection/read imports `arr-get`/`get-int` etc.). It
/// imports BOTH interfaces (the peer as `"peer"`, the runtime as `import_name` = `"heap"`), aliases +
/// lowers BOTH op sets, and instantiates the program core with BOTH bound. The peer analogue of
/// [`assemble_host_runtime`] (peer interface in place of the host effect, bound under `"peer"`); the core
/// serializer already composes both import spaces (`core_module_with_extern` lays peer ops, and the shared
/// runtime ops follow), so this lays the component-side imports in the SAME order.
///
/// Index spaces (`p = extern_fns.len()`, `k = imports.len()`, `m = exports.len()`, `g` = distinct peer
/// interfaces): lowered PEER ops → core funcs `0..p`; lowered RUNTIME ops → core funcs `p..p+k`; boundary
/// aliases → `p+k..p+k+m`. Peer instance-types → comp types `0..g`; runtime instance-type → comp type `g`;
/// boundary functypes → comp types `g+1..=g+m`. Peer op aliases → comp funcs `0..p`; runtime aliases →
/// `p..p+k`; lifts → `p+k..`. Imported peer instances → comp instances `0..g`; runtime instance → comp
/// instance `g`; peer core instance 0; runtime core instance 1; program core instance 2.
///
/// MULTI-INTERFACE (U9): `op_ifaces[i]` names the interface op `i` is imported from (see [`assemble_extern`]);
/// the distinct interfaces become comp instances/types `0..g` and each op aliases out of ITS instance. The
/// one merged `"peer"` core instance still exports every lowered peer op FLAT by name, so op names are
/// globally unique across the bound interfaces (the front-end declines a collision). `g == 1` reproduces
/// the byte-exact single-peer X5 shape.
///
/// The handle a peer hands this consumer is meaningful only within the ONE shared runtime instance (both
/// import the same runtime), and the consumer NEVER dereferences it — it reads the value only through the
/// shared runtime's accessors (`arr-get`/`get-int`/…) at the value's statically-known type, so it depends
/// on the runtime interface rather than the handle's byte representation and no peer's heap is aliased.
//= spec/contracts/component-abi.md#a-cross-component-handle-is-meaningful-only-in-the-shared-runtime-instance
//# A `value` handle that crosses between composed components MUST be meaningful only within the single runtime instance the composition shares, consistent with a runtime handle being meaningful only within the instance that produced it (§A Runtime Value Crosses As An Opaque Handle), so that composing components against a shared runtime is what makes a handle one produces intelligible to another and a handle never denotes a value in a different instance.
//= spec/contracts/component-abi.md#a-cross-component-handle-is-meaningful-only-in-the-shared-runtime-instance
//# A composed component MUST NOT dereference or interpret a handle it receives from a peer, reading the value only through the shared runtime's accessors as the value's statically-known type, so that the receiving component depends on the runtime's interface rather than on the handle's byte representation and no peer's heap is aliased by another linear memory.
pub fn assemble_extern_runtime(
    core: &[u8],
    exports: &[BoundaryExport],
    op_ifaces: &[&str],
    extern_fns: &[HostFn],
    imports: &[&RtOp],
    import_name: &str,
    publish_iface: Option<&str>,
) -> Vec<u8> {
    let p = extern_fns.len();
    let k = imports.len();
    let m = exports.len();
    // The distinct peer interfaces (first-appearance order) → comp instances/types `0..g`; the runtime is
    // comp instance/type `g`. `g == 1` reproduces the byte-exact single-peer X5 shape (peer 0, runtime 1).
    let ifaces = distinct_ifaces(op_ifaces);
    let g = ifaces.len();

    // sec 7: g+1 instance-types — one per distinct peer interface (comp types `0..g`, each declaring ITS
    // ops with instance-local func indices) then the runtime (comp type `g`).
    let type_sec = {
        let mut items = Vec::new();
        for iface in &ifaces {
            let ops = peer_group_ops(extern_fns, op_ifaces, iface);
            let mut decls = Vec::new();
            for (local, f) in ops.iter().enumerate() {
                decls.push(0x01);
                decls.extend_from_slice(&f.comp_functype);
                decls.push(0x04);
                decls.extend_from_slice(&extern_name(
                    &crate::backend::common::export_name::kebab_extern_name(&f.op),
                ));
                decls.push(0x01);
                uleb128(local as u64, &mut decls);
            }
            let mut it = vec![0x42];
            it.extend_from_slice(&wasm_vec(2 * ops.len(), &decls));
            items.extend_from_slice(&it);
        }
        let rt_it = runtime_op_instance_type(imports);
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(g + 1, &items))
    };

    // sec 10: import each peer interface (comp type `g_idx`, kebab name) → comp instances `0..g`, THEN the
    // runtime (comp type `g`, `import_name`) → comp instance `g`.
    let import_sec = {
        let mut items = Vec::new();
        for (g_idx, iface) in ifaces.iter().enumerate() {
            let mut pe = extern_name(&crate::backend::common::export_name::kebab_extern_name(
                iface,
            ));
            pe.push(0x05);
            uleb128(g_idx as u64, &mut pe);
            items.extend_from_slice(&pe);
        }
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(g as u64, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(g + 1, &items))
    };

    // sec 6 (first): alias each peer op out of ITS interface's instance (→ comp funcs `0..p`), then each
    // runtime op out of comp instance `g` (→ comp funcs `p..p+k`).
    let op_alias_sec = {
        let mut items = Vec::new();
        for (f, &oi) in extern_fns.iter().zip(op_ifaces) {
            let inst = iface_index(&ifaces, oi);
            items.extend_from_slice(&comp_alias_item(
                inst as u32,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(g as u32, op.name));
        }
        section(sec::ALIAS, &wasm_vec(p + k, &items))
    };

    // sec 8 (first): canon-lower each aliased op (comp funcs `0..p+k`) → core funcs `0..p+k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..(p + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(p + k, &items))
    };

    // sec 2: THREE core instances — (0) lowered PEER ops → `"peer"`; (1) lowered RUNTIME ops → `"heap"`;
    // (2) program module instantiated with BOTH `"peer"`=instance 0 and `"heap"`=instance 1.
    let core_instance_sec = {
        let mut items = Vec::new();
        // instance 0: peer ops (core funcs `0..p`) under their op names.
        let mut peer = vec![0x01];
        let mut peer_exports = Vec::new();
        for (i, f) in extern_fns.iter().enumerate() {
            peer_exports.extend_from_slice(&uleb_bytes(f.op.len() as u64));
            peer_exports.extend_from_slice(f.op.as_bytes());
            peer_exports.push(0x00);
            uleb128(i as u64, &mut peer_exports);
        }
        peer.extend_from_slice(&wasm_vec(p, &peer_exports));
        items.extend_from_slice(&peer);
        // instance 1: runtime ops (core funcs `p..p+k`) under their names.
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (j, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128((p + j) as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        // instance 2: instantiate module 0 with `"peer"`=core instance 0 and `"heap"`=core instance 1.
        let mut prog = vec![0x00];
        uleb128(0, &mut prog);
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(PEER_MODULE.len() as u64));
        args.extend_from_slice(PEER_MODULE.as_bytes());
        args.push(0x12);
        uleb128(0, &mut args);
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(1, &mut args);
        prog.extend_from_slice(&wasm_vec(2, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(3, &items))
    };

    // sec 6 (second): alias each boundary func off the PROGRAM instance (core instance 2) → `p+k..p+k+m`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(2, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };

    // sec 7 (second): one component functype per boundary export → comp types `g+1..=g+m` (after the g
    // peer instance-types `0..g` and the runtime instance-type `g`).
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            debug_assert!(
                e.result != BoundaryResult::Bytes,
                "a list<u8> boundary result takes the resource path, not the extern+runtime shape"
            );
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };

    // sec 8 (second): lift each boundary core func (`p+k+j`) using its component type (`g+1+j`).
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((p + k + j) as u32, (g + 1 + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };

    // sec 5 + 11 (publish) OR sec 11 (top-level): when this component is ALSO a provider (`publish_iface`
    // = `Some(iface)`, a MIDDLE-of-chain component binding a peer AND publishing its own interface while
    // inspecting a compound handle, U11), BUNDLE the lifted boundary funcs (comp funcs `p+k..p+k+m`) into a
    // component instance and export THAT under `iface`. The bundle is comp instance `g+1` (after the g
    // imported peer instances `0..g` and the runtime instance `g`). Otherwise (`None`, a pure consumer)
    // export each lifted func at TOP LEVEL (byte-identical to the X5 shape — `instance_sec` stays empty).
    let (instance_sec, export_sec) = match publish_iface {
        Some(iface) => {
            let mut item = vec![0x01]; // export-items form
            let mut members = Vec::new();
            for (j, e) in exports.iter().enumerate() {
                let name = crate::backend::common::export_name::kebab_extern_name(&e.name);
                members.extend_from_slice(&extern_name(&name));
                members.push(0x01); // ComponentExportKind::Func
                uleb128((p + k + j) as u64, &mut members);
            }
            item.extend_from_slice(&wasm_vec(m, &members));
            let instance_sec = section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &item));
            let iface_name = crate::backend::common::export_name::kebab_extern_name(iface);
            let export_sec = section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_instance_item(&iface_name, (g + 1) as u32)),
            );
            (instance_sec, export_sec)
        }
        None => {
            let mut items = Vec::new();
            for (j, e) in exports.iter().enumerate() {
                items.extend_from_slice(&comp_export_item(&e.name, (p + k + j) as u32));
            }
            (
                Vec::new(),
                section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items)),
            )
        }
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: peer + runtime instance-types
    out.extend_from_slice(&import_sec); // 10: import both interfaces
    out.extend_from_slice(&op_alias_sec); // 6: alias both op sets
    out.extend_from_slice(&lower_sec); // 8: lower both op sets → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: embedded consumer program
    out.extend_from_slice(&core_instance_sec); // 2: peer + heap + program instances
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&boundary_type_sec); // 7: boundary functypes
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&instance_sec); // 5: (publish) bundle lifts into an instance — empty for a consumer
    out.extend_from_slice(&export_sec); // 11: export the instance (publish) or each func (top-level)
    out
}

/// The HOST + RUNTIME composed shape: a program that BOTH delegates a host effect `iface` AND uses the
/// value-heap runtime (e.g. `(host (ask) (Map.size (Map.insert (map …) (ask.ask) …)))` — the host op
/// returns a value fed into a runtime collection op). It imports TWO interfaces — the effect (as `"host"`)
/// and the runtime (as `import_name`, the `"heap"` interface) — aliases + lowers BOTH op sets, and
/// instantiates the program core with BOTH bound. The fusion of [`assemble_host`] (single effect import)
/// and [`assemble_with_imports`] (single runtime import); the core module already composes both import
/// spaces (`serialize::core_module_with_host` lays host funcs `0..h` then runtime `h..h+k`), so this lays
/// the component-side imports in the SAME order.
///
/// Index spaces (`h = host_fns.len()`, `k = imports.len()`, `m = exports.len()`):
///   * lowered HOST ops → core funcs `0..h`; lowered RUNTIME ops → core funcs `h..h+k`; boundary
///     core-aliases → core funcs `h+k..h+k+m`.
///   * host effect instance-type → component type 0; runtime instance-type → component type 1; boundary
///     functypes → component types `2..=1+m`.
///   * host op aliases → component funcs `0..h`; runtime op aliases → component funcs `h..h+k`; lifts →
///     component funcs `h+k..h+k+m`.
///   * imported effect instance → component instance 0; imported runtime instance → component instance 1;
///     host core instance → core instance 0; runtime core instance → core instance 1; program → core
///     instance 2.
///
/// SCOPE: host-only + value-heap runtime, SINGLE effect, scalar/unit host ops (a string/compound host op
/// still declines upstream via the representability guard), NO memory (a string host arg composing with
/// the runtime is a later increment — this fires only for a scalar host op + runtime collection ops).
pub fn assemble_host_runtime(
    core: &[u8],
    exports: &[BoundaryExport],
    iface: &str,
    host_fns: &[HostFn],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    let h = host_fns.len();
    let k = imports.len();
    let m = exports.len();

    // sec 7: TWO instance-types — component type 0 (the effect) then component type 1 (the runtime). Each
    // is `0x42` + a vec of 2*count interleaved (ty, export) decls, exactly as the single-import shapes.
    let type_sec = {
        let host_it = host_effect_instance_type(
            host_fns,
            host_fns.iter().any(|f| f.has_list_param),
            &[],
            &[],
        );
        let rt_it = runtime_op_instance_type(imports);
        let mut items = host_it;
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };

    // sec 10: import the effect interface (instance of comp type 0, under the kebab effect name) THEN the
    // runtime interface (instance of comp type 1, under `import_name`) → component instances 0 and 1.
    let import_sec = {
        let mut items = Vec::new();
        let mut eff = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            iface,
        ));
        eff.push(0x05); // ComponentTypeRef::Instance
        uleb128(0, &mut eff);
        items.extend_from_slice(&eff);
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(1, &mut rt); // runtime uses component type 1
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(2, &items))
    };

    // sec 6 (first): alias each host op out of comp instance 0 (→ comp funcs `0..h`), then each runtime op
    // out of comp instance 1 (→ comp funcs `h..h+k`).
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(1, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
    };

    // sec 8 (first): canon-lower each aliased op (comp funcs `0..h+k`) → core funcs `0..h+k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    };

    // sec 2: THREE core instances — (0) the lowered HOST ops exported under their op names → `"host"`;
    // (1) the lowered RUNTIME ops exported under their names → `"heap"`; (2) the program module
    // instantiated with BOTH `"host"`=instance 0 and `"heap"`=instance 1.
    let core_instance_sec = {
        let mut items = Vec::new();
        // instance 0: host ops (core funcs `0..h`) under their op names.
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
        // instance 1: runtime ops (core funcs `h..h+k`) under their names.
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (j, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128((h + j) as u64, &mut heap_exports); // core func index of this lowered runtime op
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        // instance 2: instantiate module 0 with `"host"`=core instance 0 and `"heap"`=core instance 1.
        let mut prog = vec![0x00]; // instantiate form
        uleb128(0, &mut prog); // module index 0
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HOST_MODULE.len() as u64));
        args.extend_from_slice(HOST_MODULE.as_bytes());
        args.push(0x12); // ModuleArg::Instance
        uleb128(0, &mut args); // core instance 0
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(1, &mut args); // core instance 1
        prog.extend_from_slice(&wasm_vec(2, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(3, &items))
    };

    // sec 6 (second): alias each boundary func out of the PROGRAM instance (core instance 2) → core funcs
    // `h+k..h+k+m`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(2, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };

    // sec 7 (second): one component functype per boundary export → component types `2..=1+m`. A `list<u8>`
    // boundary result takes the resource path, never this shape.
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            debug_assert!(
                e.result != BoundaryResult::Bytes,
                "a list<u8> boundary result takes the resource path, not the host+runtime shape"
            );
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };

    // sec 8 (second): lift each boundary core func (`h+k+j`) using its component type (`2+j`) → component
    // funcs `h+k..h+k+m`.
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((h + k + j) as u32, (2 + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };

    // sec 11: export each lifted component func (`h+k+j`) under its verbatim boundary name.
    let export_sec = {
        let mut items = Vec::new();
        for (j, e) in exports.iter().enumerate() {
            items.extend_from_slice(&comp_export_item(&e.name, (h + k + j) as u32));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: effect + runtime instance-types
    out.extend_from_slice(&import_sec); // 10: import both interfaces
    out.extend_from_slice(&op_alias_sec); // 6: alias host ops then runtime ops
    out.extend_from_slice(&lower_sec); // 8: lower both op sets → core funcs
    out.extend_from_slice(&core_module_section(core)); // 1: embedded program
    out.extend_from_slice(&core_instance_sec); // 2: host-instance + heap-instance + program-instance
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&boundary_type_sec); // 7: boundary functypes
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&export_sec); // 11: export
    out
}

/// The HOST-STRING-PARAM + value-heap RUNTIME shape: [`assemble_host_runtime`] PLUS the shared-memory
/// core module that [`assemble_host_mem`] adds, so a host op taking a `string` parameter composes with the
/// runtime import space. The `(ptr,len)` a `string` lowers to is read from a memory both the program and
/// each host op's canon-lower bind (a separate `shared_mem_module`, breaking the lower↔instance
/// circularity), while the runtime ops lower memoryless (they carry scalar `u32` handles, not strings).
///
/// Delta over `assemble_host_runtime` (which embeds ONLY the program module and has 3 core instances):
/// insert the mem module as core MODULE 0 (program becomes module 1) + its instance as core INSTANCE 0 +
/// a `mem`.`mem` → core memory 0 alias; the host ops lower with `canon_lower_item_mem(_, 0)`; and the
/// program instance is instantiated with `"mem"` (core instance 0) alongside `"host"` and `"heap"`.
///
/// Index spaces (`h`/`k`/`m` = host/runtime/export counts): core memory `0`; lowered HOST ops → core funcs
/// `0..h`; lowered RUNTIME ops → core funcs `h..h+k`; boundary core-aliases → core funcs `h+k..h+k+m`.
/// Component: host effect instance-type → type 0; runtime instance-type → type 1; boundary functypes →
/// types `2..=1+m`; host op aliases → comp funcs `0..h`; runtime op aliases → comp funcs `h..h+k`; lifts →
/// comp funcs `h+k..h+k+m`; imported effect instance → comp instance 0; imported runtime → comp instance 1.
/// Core instances: mem `0`, host-ops `1`, heap-ops `2`, program `3`. Core modules: mem `0`, program `1`.
///
/// SCOPE: host + value-heap runtime, SINGLE effect, scalar/unit host RESULT, `string`-or-scalar host params.
pub fn assemble_host_runtime_mem(
    core: &[u8],
    exports: &[BoundaryExport],
    iface: &str,
    host_fns: &[HostFn],
    imports: &[&RtOp],
    import_name: &str,
) -> Vec<u8> {
    let h = host_fns.len();
    let k = imports.len();
    let m = exports.len();

    // sec 7: TWO instance-types — host effect (comp type 0) then runtime (comp type 1) — identical to the
    // memoryless host+runtime shape (the shared memory is a CORE detail, invisible to the component types).
    let type_sec = {
        let host_it = host_effect_instance_type(
            host_fns,
            host_fns.iter().any(|f| f.has_list_param),
            &[],
            &[],
        );
        let rt_it = runtime_op_instance_type(imports);
        let mut items = host_it;
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };

    // sec 10: import the effect interface (comp type 0, under the kebab effect name) THEN the runtime
    // interface (comp type 1, under `import_name`) → component instances 0 and 1.
    let import_sec = {
        let mut items = Vec::new();
        let mut eff = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            iface,
        ));
        eff.push(0x05); // ComponentTypeRef::Instance
        uleb128(0, &mut eff);
        items.extend_from_slice(&eff);
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(1, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(2, &items))
    };

    // sec 6 (first): alias each host op out of comp instance 0 (→ comp funcs `0..h`), then each runtime op
    // out of comp instance 1 (→ comp funcs `h..h+k`).
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(1, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
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

    // sec 8 (first): canon-lower each aliased op → core funcs `0..h+k`. The HOST ops carry the MEMORY option
    // (core memory 0 — their `string` params read from the shared memory); the RUNTIME ops lower memoryless
    // (scalar `u32` handles, no string args).
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..h {
            items.extend_from_slice(&canon_lower_item_mem(i as u32, 0));
        }
        for i in h..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    };

    // sec 1 (second): the embedded program core module (module 1).
    let prog_module_sec = core_module_section(core);

    // sec 2 (second): THREE core instances — (1) lowered HOST ops as `"host"`, (2) lowered RUNTIME ops as
    // `"heap"`, (3) the program instantiated with `"host"`=instance 1, `"heap"`=instance 2, `"mem"`=instance
    // 0. (Core instance 0 is the mem instance emitted above.)
    let prog_instance_sec = {
        let mut items = Vec::new();
        // instance 1: host ops (core funcs `0..h`) under their op names.
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
        // instance 2: runtime ops (core funcs `h..h+k`) under their names.
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (j, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128((h + j) as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        // instance 3: instantiate module 1 with `"host"`=instance 1, `"heap"`=instance 2, `"mem"`=instance 0.
        let mut prog = vec![0x00]; // instantiate form
        uleb128(1, &mut prog); // module index 1 (the program; module 0 is the mem module)
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HOST_MODULE.len() as u64));
        args.extend_from_slice(HOST_MODULE.as_bytes());
        args.push(0x12); // ModuleArg::Instance
        uleb128(1, &mut args); // core instance 1
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(2, &mut args); // core instance 2
        args.extend_from_slice(&uleb_bytes("mem".len() as u64));
        args.extend_from_slice(b"mem");
        args.push(0x12);
        uleb128(0, &mut args); // core instance 0 (the mem instance)
        prog.extend_from_slice(&wasm_vec(3, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(3, &items))
    };

    // sec 6 (boundary alias): alias each boundary func out of the PROGRAM instance (core instance 3) → core
    // funcs `h+k..h+k+m`.
    let boundary_alias_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&core_alias_item(3, &e.name));
        }
        section(sec::ALIAS, &wasm_vec(m, &items))
    };

    // sec 7 (second): one component functype per boundary export → component types `2..=1+m`.
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            debug_assert!(
                e.result != BoundaryResult::Bytes,
                "a list<u8> boundary result takes the resource path, not the host+runtime shape"
            );
            items.extend_from_slice(&comp_functype(e, 0));
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(m, &items))
    };

    // sec 8 (second): lift each boundary core func (`h+k+j`) using its component type (`2+j`).
    let lift_sec = {
        let mut items = Vec::new();
        for j in 0..m {
            items.extend_from_slice(&canon_lift_item((h + k + j) as u32, (2 + j) as u32));
        }
        section(sec::CANON, &wasm_vec(m, &items))
    };

    // sec 11: export each lifted component func (`h+k+j`) under its verbatim boundary name.
    let export_sec = {
        let mut items = Vec::new();
        for (j, e) in exports.iter().enumerate() {
            items.extend_from_slice(&comp_export_item(&e.name, (h + k + j) as u32));
        }
        section(sec::COMPONENT_EXPORT, &wasm_vec(m, &items))
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: effect + runtime instance-types
    out.extend_from_slice(&import_sec); // 10: import both interfaces
    out.extend_from_slice(&op_alias_sec); // 6: alias host ops then runtime ops
    out.extend_from_slice(&mem_module_sec); // 1: shared-memory module (module 0)
    out.extend_from_slice(&mem_instance_sec); // 2: instantiate mem → core instance 0
    out.extend_from_slice(&mem_alias_sec); // 6: alias mem.mem → core memory 0
    out.extend_from_slice(&lower_sec); // 8: lower host ops (Memory option) then runtime ops
    out.extend_from_slice(&prog_module_sec); // 1: embedded program (module 1)
    out.extend_from_slice(&prog_instance_sec); // 2: host + heap + program instances
    out.extend_from_slice(&boundary_alias_sec); // 6: alias boundary funcs off the program
    out.extend_from_slice(&boundary_type_sec); // 7: boundary functypes
    out.extend_from_slice(&lift_sec); // 8: lift boundary funcs
    out.extend_from_slice(&export_sec); // 11: export
    out
}

/// §3c GAP B — the HOST-FUSED bytes-roundtrip PROVIDER: a reducer whose `apply` crosses as `list<u8>`
/// (value-form) AND whose body calls a HOST interface (e.g. `kv`). Combines [`assemble_host_runtime_mem`]'s
/// host+runtime+shared-memory import wiring with [`assemble_bytes_roundtrip_provider`]'s `list<u8>` member
/// canon-lift + provider-instance export. `core` is [`super::serialize::bytes_roundtrip_host_core_module`]
/// (imports `"host"`+`"heap"`+`"mem"`). `host_iface` is the kv interface's component-import name (the world's
/// FQ import interface, e.g. `cadenza:agent-kernel/kv`, matching the host bind); `provider_iface` is the
/// EXPORT interface (`--component-name`, e.g. `cadenza:agent-kernel/fold`). Component types: host (0),
/// runtime (1), `list<u8>` (2), apply functype (3). Comp funcs: host op aliases `0..h`, runtime `h..h+k`,
/// apply lift `h+k`. Core: mem module → core instance 0 (memory 0); lowered host ops (Memory 0) `0..h` +
/// runtime `h..h+k`; program → host instance 1, heap instance 2, program instance 3 (bound host/heap/mem);
/// apply + cabi_realloc aliased off the program (core funcs `h+k`, `h+k+1`).
#[allow(clippy::too_many_arguments)]
pub fn assemble_bytes_roundtrip_host_provider(
    core: &[u8],
    provider_iface: &str,
    member_name: &str,
    host_fns: &[HostFn],
    host_iface: &str,
    imports: &[&RtOp],
    import_name: &str,
    needs_list: bool,
    result_defs: &[(Vec<u8>, bool)],
    nominal_defs: &[Vec<u8>],
) -> Vec<u8> {
    let h = host_fns.len();
    let k = imports.len();
    let list_type_idx: u32 = 2; // comp types: 0 host, 1 runtime, 2 list<u8>, 3 apply functype
    let apply_functype_idx: u32 = 3;

    // sec 7 (first): host effect instance-type (comp type 0) + runtime instance-type (comp type 1).
    let type_sec = {
        let host_it = host_effect_instance_type(host_fns, needs_list, result_defs, nominal_defs);
        let mut items = host_it;
        items.extend_from_slice(&runtime_op_instance_type(imports));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };

    // sec 10: import the host interface (comp type 0, FQ host_iface) → comp instance 0, then the runtime
    // (comp type 1, import_name) → comp instance 1.
    let import_sec = {
        let mut items = Vec::new();
        let mut he = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            host_iface,
        ));
        he.push(0x05); // ComponentTypeRef::Instance
        uleb128(0, &mut he);
        items.extend_from_slice(&he);
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(1, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(2, &items))
    };

    // sec 6 (first): alias host ops off comp instance 0 (comp funcs 0..h), runtime off comp instance 1.
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(1, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
    };

    // sec 1/2/6: the shared-memory+realloc module (core module 0) → core instance 0 → memory alias (core
    // memory 0). Carries a bump `cabi_realloc` (the S0 single allocator over memory 0); the put path leaves
    // it unused (the guest still owns its own realloc for the apply lift) — a harmless additive export.
    let mem_module_sec = core_module_section(&shared_mem_realloc_module());
    let mem_instance_sec = section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[])),
    );
    // Alias the shared memory (core memory 0) AND the shared `cabi_realloc` (core func 0) off the mem
    // instance — BEFORE `lower_sec`, so the kv.get canon-LOWER (which returns `option<list<u8>>`, needing
    // realloc to lift the returned list into memory 0) can reference it; the apply canon-LIFT uses the same
    // realloc. Placing the realloc alias here makes it core func 0, so the lowered ops shift to `1..1+h+k`.
    let mem_alias_sec = section(
        sec::ALIAS,
        &wasm_vec(2, &{
            let mut it = memory_alias_item(0, "mem");
            it.extend_from_slice(&core_alias_item(0, "cabi_realloc"));
            it
        }),
    );

    // sec 8 (first): lower host ops WITH Memory 0 + Realloc (core func 0, the shared cabi_realloc aliased
    // pre-lower). Memory 0: a host op's list<u8> args are read from the shared mem. Realloc: a host op that
    // RETURNS a heap value (kv.get's `option<list<u8>>`) needs it to lift the returned list into memory 0; a
    // unit/scalar-result host op (kv.put) carries it unused (harmless). Runtime ops stay memoryless.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..h {
            items.extend_from_slice(&canon_lower_item_mem_realloc(i as u32, 0, 0));
        }
        for i in h..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    };

    // sec 1 (second): the embedded host-fused program core module (module 1).
    let prog_module_sec = core_module_section(core);

    // sec 2 (second): host ops instance (1), runtime ops instance (2), program (3) bound host/heap/mem.
    // Core-func indices are +1 vs the pre-realloc-alias layout: core func 0 is the shared cabi_realloc alias,
    // so the lowered host op `i` is core func `1+i` and the lowered runtime op `j` is core func `1+h+j`.
    let prog_instance_sec = {
        let mut items = Vec::new();
        let mut host = vec![0x01];
        let mut host_exports = Vec::new();
        for (i, f) in host_fns.iter().enumerate() {
            host_exports.extend_from_slice(&uleb_bytes(f.op.len() as u64));
            host_exports.extend_from_slice(f.op.as_bytes());
            host_exports.push(0x00);
            uleb128((i + 1) as u64, &mut host_exports);
        }
        host.extend_from_slice(&wasm_vec(h, &host_exports));
        items.extend_from_slice(&host);
        let mut heap = vec![0x01];
        let mut heap_exports = Vec::new();
        for (j, op) in imports.iter().enumerate() {
            heap_exports.extend_from_slice(&uleb_bytes(op.name.len() as u64));
            heap_exports.extend_from_slice(op.name.as_bytes());
            heap_exports.push(0x00);
            uleb128((h + j + 1) as u64, &mut heap_exports);
        }
        heap.extend_from_slice(&wasm_vec(k, &heap_exports));
        items.extend_from_slice(&heap);
        let mut prog = vec![0x00];
        uleb128(1, &mut prog); // module 1 (module 0 is the mem module)
        let mut args = Vec::new();
        args.extend_from_slice(&uleb_bytes(HOST_MODULE.len() as u64));
        args.extend_from_slice(HOST_MODULE.as_bytes());
        args.push(0x12);
        uleb128(1, &mut args); // "host" = core instance 1
        args.extend_from_slice(&uleb_bytes(HEAP_MODULE.len() as u64));
        args.extend_from_slice(HEAP_MODULE.as_bytes());
        args.push(0x12);
        uleb128(2, &mut args); // "heap" = core instance 2
        args.extend_from_slice(&uleb_bytes("mem".len() as u64));
        args.extend_from_slice(b"mem");
        args.push(0x12);
        uleb128(0, &mut args); // "mem" = core instance 0
        prog.extend_from_slice(&wasm_vec(3, &args));
        items.extend_from_slice(&prog);
        section(sec::CORE_INSTANCE, &wasm_vec(3, &items))
    };

    // sec 6 (member alias): alias the apply member func off the PROGRAM instance (core instance 3) → core
    // func `1+h+k` (core func 0 = the shared cabi_realloc alias, `1..1+h+k` = the lowered ops). The realloc
    // is already aliased (core func 0, in mem_alias_sec) so the apply canon-LIFT + kv.get lower both use it.
    let member_alias_sec = {
        let items = core_alias_item(3, member_name);
        section(sec::ALIAS, &wasm_vec(1, &items))
    };

    // sec 7 (second): the shared list<u8> defined type (comp type 2) + the apply functype (comp type 3).
    let boundary_type_sec = bytes_roundtrip_boundary_type_sec(list_type_idx);

    // sec 8 (second): lift apply with Memory(0) + Realloc. Core func 0 = shared cabi_realloc alias, lowered
    // ops = `1..1+h+k`, apply alias = `1+h+k`. So apply is core func `1+h+k` and realloc is core func 0.
    let apply_core_func = (h + k) as u32 + 1;
    let realloc_core_func = 0u32;
    let lift_sec = section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item(apply_core_func, 0, realloc_core_func, apply_functype_idx),
        ),
    );

    // sec 5: bundle the lifted apply (comp func h+k) into a provider interface-instance.
    let instance_sec = {
        let mut item = vec![0x01];
        let mname = crate::backend::common::export_name::kebab_extern_name(member_name);
        let mut members = Vec::new();
        members.extend_from_slice(&extern_name(&mname));
        members.push(0x01); // ComponentExportKind::Func
        uleb128((h + k) as u64, &mut members);
        item.extend_from_slice(&wasm_vec(1, &members));
        section(sec::COMPONENT_INSTANCE, &wasm_vec(1, &item))
    };

    // sec 11: export the bundled provider instance under provider_iface. Imports are comp instances 0..1,
    // the bundle is comp instance 2 → export index 2.
    let export_sec = {
        let iface_name = crate::backend::common::export_name::kebab_extern_name(provider_iface);
        section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_instance_item(&iface_name, 2)),
        )
    };

    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    out.extend_from_slice(&type_sec); // 7: host + runtime instance-types
    out.extend_from_slice(&import_sec); // 10: import host + runtime
    out.extend_from_slice(&op_alias_sec); // 6: alias host ops + runtime ops
    out.extend_from_slice(&mem_module_sec); // 1: shared-memory module (module 0)
    out.extend_from_slice(&mem_instance_sec); // 2: mem instance (core instance 0)
    out.extend_from_slice(&mem_alias_sec); // 6: mem.mem → core memory 0
    out.extend_from_slice(&lower_sec); // 8: lower host (mem) + runtime
    out.extend_from_slice(&prog_module_sec); // 1: embedded program (module 1)
    out.extend_from_slice(&prog_instance_sec); // 2: host + heap + program instances
    out.extend_from_slice(&member_alias_sec); // 6: alias apply + cabi_realloc off the program
    out.extend_from_slice(&boundary_type_sec); // 7: list<u8> type + apply functype
    out.extend_from_slice(&lift_sec); // 8: lift apply
    out.extend_from_slice(&instance_sec); // 5: bundle into the provider instance
    out.extend_from_slice(&export_sec); // 11: export the instance under provider_iface
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

/// [`shared_mem_module`] PLUS a bump-allocator `cabi_realloc` exported alongside `mem` — the SINGLE
/// allocator over the shared memory 0 for the bytes-roundtrip host provider (S0). A host op whose result
/// is `option<list<u8>>` (kv.get) is canon-LOWERED with `(memory 0, realloc <this>)`, and the apply
/// canon-LIFT uses the SAME realloc — so both the returned-list lift and the apply-result encode bump ONE
/// cursor over memory 0 (no dual-allocator conflict). Living in this pre-instance shared module (core
/// instance 0) it is available BEFORE the program instance, breaking the lower↔realloc circularity a
/// list-returning host import would otherwise face (the guest imports both `mem` and `cabi_realloc`). The
/// bump cursor inits at 16 (above the fixed `OUT=8` retarea), matching the guest's own former allocator.
fn shared_mem_realloc_module() -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    // type 0: (i32,i32,i32,i32) -> i32 (the cabi_realloc signature).
    let type_sec = {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(1, &t))
    };
    // func 0: type 0.
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(1, &[0x00]));
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01])); // limits {min:1}
    // global 0: mutable i32 bump cursor, init 16 (above the fixed OUT=8 retarea).
    let global_sec = {
        let mut item = vec![wasm_abi::CORE_I32, 0x01];
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(16, &mut item);
        item.push(op::END);
        section(wasm_abi::CORE_SEC_GLOBAL, &wasm_vec(1, &item))
    };
    // export `mem` (memory 0) + `cabi_realloc` (func 0).
    let export_sec = {
        let mut items = uleb_bytes("mem".len() as u64);
        items.extend_from_slice(b"mem");
        items.push(wasm_abi::EXPORT_KIND_MEMORY);
        uleb128(0, &mut items);
        items.extend_from_slice(&uleb_bytes("cabi_realloc".len() as u64));
        items.extend_from_slice(b"cabi_realloc");
        items.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128(0, &mut items);
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(2, &items))
    };
    // code: the bump-allocator body (bump global 0).
    let code_sec = section(
        wasm_abi::CORE_SEC_CODE,
        &wasm_vec(
            1,
            &crate::backend::wasm::serialize::emit_bump_realloc_body(0),
        ),
    );
    let mut out = Vec::new();
    out.extend_from_slice(wasm_abi::CORE_MAGIC);
    out.extend_from_slice(&type_sec);
    out.extend_from_slice(&func_sec);
    out.extend_from_slice(&mem_sec);
    out.extend_from_slice(&global_sec);
    out.extend_from_slice(&export_sec);
    out.extend_from_slice(&code_sec);
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
            decls.extend_from_slice(&extern_name(
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
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
        let mut item = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            iface,
        ));
        item.push(0x05);
        uleb128(0, &mut item);
        section(sec::COMPONENT_IMPORT, &wasm_vec(1, &item))
    };

    // sec 6 (first): alias each op out of the imported effect instance (comp instance 0) → comp funcs. The
    // alias name is the kebab-normalized op name the instance-type export decl uses (they must match).
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
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
    make_slots: &[ArgSlot],
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: the import instance-type declaring the k used runtime ops (component type 0). Identical to
    // the import shape's instance-type: 2k interleaved (ty, export) decls.
    let instance_type = runtime_op_instance_type(imports);
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
    // sec 7: [one `tuple<…>` type per COMPOUND make-param, minted at types 2.., shifting the rest] then
    // `own<t>` and the `make` functype `(make-params…) -> own<t>`. The resource is component type 1 (the
    // import-instance-type is type 0), so `own` references type 1. A NULLARY export gives `make() ->
    // own<t>` (byte-identical to the old form); a SCALAR param is an inline byte; a COMPOUND param takes a
    // minted native `tuple<…>` (the ABI flattens it into the scalar core leaves the core `make` rebuilds).
    // `shift` = the number of compound params (= tuple types minted), so `own<t>`/make-ft/encode types
    // slide past them.
    let shift = call_arg_tuple_type_count(make_slots); // total tuple types (nesting mints >1 per compound param)
    let own_ty = 2 + shift; // own<t> sits after the minted tuple types (2..2+shift)
    let make_ft = 3 + shift;
    let make_types = {
        // Mint the per-compound tuple types starting at type 2, then own<1>, then the make functype
        // referencing each param's valtype (a scalar byte, or its minted tuple type index).
        let mut next_type = 2u32;
        let mut items = Vec::new();
        let tup_idxs = mint_call_arg_tuple_types(make_slots, &mut next_type, &mut items);
        items.extend_from_slice(&own_item(1));
        items.extend_from_slice(&make_functype_slots(
            make_slots,
            &tup_idxs,
            &owned_valtype(own_ty),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2 + shift as usize, &items))
    };
    out.extend_from_slice(&make_types);
    // sec 8: lift `make` (core func k+3) against the make functype → component func k.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((k + 3) as u32, make_ft)),
    ));
    // sec 7: `borrow<t>`, the shared `list u8` type, then the `encode` functype `(self: borrow<t>) ->
    // list<u8>` — each shifted +`shift` by the optional tuple type. `encode` BORROWS self (the host keeps
    // ownership; the dtor reclaims on drop); the core `t-encode` gets the rep directly (no `resource.rep`),
    // so the method is repeatable ([[rcdzc-r1-resource-encode-linking-findings]]).
    let borrow_ty = 4 + shift;
    let list_ty = 5 + shift;
    let encode_ft = 6 + shift;
    let encode_types = {
        let mut items = borrow_item(1); // borrow<resource> — resource is component type 1
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(borrow_ty, list_ty));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_types);
    // sec 8: lift `encode` (core func k+4) against the encode functype, carrying Memory 0 + Realloc (core
    // func k+5) → component func k+1.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((k + 4) as u32, 0, (k + 5) as u32, encode_ft),
        ),
    ));
    // sec 4: the nested re-export component — the BORROW variant (re-types `encode` against
    // `borrow<t>`), matching the borrow lift above; its `make` carries the same forwarded params.
    out.extend_from_slice(&component_section(&resource_inner_component_borrow(
        make_slots,
    )));
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

/// The resource-escape × peer-extern FUSION: [`assemble_runtime_resource`] plus a leading SINGLE peer
/// interface import (`peer_fns` from interface `peer_iface`). A peer-bound op reached in a body whose
/// ENTRYPOINT RESULT escapes as a runtime resource (main RETURNS the compound the peer produced) needs its
/// peer import carried into the resource component — neither `assemble_runtime_resource` (imports only the
/// runtime) nor `assemble_extern_runtime` (publishes no resource) does both. This composes them: the peer
/// interface is component instance 0 / type 0, the runtime is instance 1 / type 1 (shifted +1 from the
/// no-peer shape), and every runtime/resource CORE-func index shifts by `p = peer_fns.len()` (peer ops
/// lowered to core funcs `0..p`, runtime ops `p..p+k`, resource-new/rep `p+k`/`p+k+1`, make `p+k+3`,
/// t-encode `p+k+4`, cabi_realloc `p+k+5`). The core module (built by
/// `runtime_resource_core_module_form_ex2`) imports both `"peer"` (p ops) and `"heap"` (k ops + 2
/// intrinsics) in that order, matching the alias/lower order here. SCOPE: a SINGLE peer interface, matching
/// the byte-exact single-peer X5 shape; multi-interface is a later widening. `p = 0` is NOT valid here (the
/// caller routes a no-peer escape to `assemble_runtime_resource`).
#[allow(clippy::too_many_arguments)]
pub fn assemble_extern_runtime_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    peer_fns: &[HostFn],
    op_ifaces: &[&str],
    make_slots: &[ArgSlot],
) -> Vec<u8> {
    let p = peer_fns.len();
    let k = imports.len();
    // The distinct peer interfaces (first-appearance order) → comp instances/types `0..g`; the runtime is
    // comp instance/type `g`. `g == 1` reproduces the byte-exact single-peer shape. Component-level types/
    // instances shift by `g`; the peer ops still lower to core funcs `0..p` from ONE `"peer"` core module
    // (the core module imports all peer ops from `"peer"` regardless of interface), so core-func indices
    // and core-instances are INDEPENDENT of `g`.
    let ifaces = distinct_ifaces(op_ifaces);
    let g = ifaces.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: g+1 instance-types — one per distinct peer interface (comp types `0..g`, each declaring ITS
    // ops with instance-local func indices) then the runtime (comp type `g`, declaring the k runtime ops).
    let type_sec = {
        let mut items = Vec::new();
        for iface in &ifaces {
            let ops = peer_group_ops(peer_fns, op_ifaces, iface);
            let mut decls = Vec::new();
            for (local, f) in ops.iter().enumerate() {
                decls.push(0x01);
                decls.extend_from_slice(&f.comp_functype);
                decls.push(0x04);
                decls.extend_from_slice(&extern_name(
                    &crate::backend::common::export_name::kebab_extern_name(&f.op),
                ));
                decls.push(0x01);
                uleb128(local as u64, &mut decls);
            }
            let mut it = vec![0x42];
            it.extend_from_slice(&wasm_vec(2 * ops.len(), &decls));
            items.extend_from_slice(&it);
        }
        let rt_it = runtime_op_instance_type(imports);
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(g + 1, &items))
    };
    out.extend_from_slice(&type_sec);

    // sec 10: import each PEER interface (comp type `g_idx`, kebab name) → comp instances `0..g`, THEN the
    // runtime (comp type `g`, `import_name`) → comp instance `g`.
    let import_sec = {
        let mut items = Vec::new();
        for (g_idx, iface) in ifaces.iter().enumerate() {
            let mut pe = extern_name(&crate::backend::common::export_name::kebab_extern_name(
                iface,
            ));
            pe.push(0x05); // ComponentTypeRef::Instance
            uleb128(g_idx as u64, &mut pe);
            items.extend_from_slice(&pe);
        }
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(g as u64, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(g + 1, &items))
    };
    out.extend_from_slice(&import_sec);

    // sec 6: alias each PEER op out of ITS interface's instance (→ comp funcs `0..p`), then each RUNTIME op
    // out of comp instance `g` (→ comp funcs `p..p+k`).
    let op_alias_sec = {
        let mut items = Vec::new();
        for (f, &oi) in peer_fns.iter().zip(op_ifaces) {
            let inst = iface_index(&ifaces, oi);
            items.extend_from_slice(&comp_alias_item(
                inst as u32,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(g as u32, op.name));
        }
        section(sec::ALIAS, &wasm_vec(p + k, &items))
    };
    out.extend_from_slice(&op_alias_sec);

    // sec 8: canon-lower each aliased op (comp funcs `0..p+k`) → core funcs `0..p+k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..(p + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(p + k, &items))
    };
    out.extend_from_slice(&lower_sec);

    // sec 2: the peer core instance (export-items form) exporting the lowered PEER ops (core funcs `0..p`)
    // under their op names → core instance 0. ONE instance for ALL interfaces — the program module's single
    // `"peer"` import binds to this (the interface grouping is component-level only, above).
    let peer_core_inst = {
        let ex: Vec<(&str, u32)> = peer_fns
            .iter()
            .enumerate()
            .map(|(i, f)| (f.op.as_str(), i as u32))
            .collect();
        core_export_instance_item(&ex)
    };
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &wasm_vec(1, &peer_core_inst)));

    // sec 2: the `heap-dtor` core instance exporting the lowered `drop` op (at `p + drop's runtime index`)
    // as `drop` → core instance 1. (Shifted +p: the peer ops precede the runtime ops in the lowered set.)
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| (p + i) as u32)
        .expect("the runtime-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    // sec 1: the dtor core module (module 0) — imports `heap-dtor.drop`, calls it in `t-dtor`.
    out.extend_from_slice(&core_module_section(dtor_core));
    // sec 2: instantiate the dtor module threading `heap-dtor` = core instance 1 → core instance 2.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 1)])),
    ));
    // sec 6: alias `t-dtor` out of core instance 2 → core func `p+k`.
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(2, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func `p+k`) → component type `g+1` (after the g
    // peer instance-types `0..g` and the runtime instance-type `g`).
    let res_ty = (g + 1) as u32;
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item((p + k) as u32)),
    ));
    // sec 8: canon `resource.new` (→ core func `p+k+1`) AND `resource.rep` (→ core func `p+k+2`) for the
    // resource type (component type `g+1`) — both in one canon section (count 2).
    let resource_canons = {
        let mut items = resource_new_item(res_ty);
        items.extend_from_slice(&resource_rep_item(res_ty));
        section(sec::CANON, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&resource_canons);
    // sec 2: the `heap` core instance exporting the k lowered runtime ops (funcs `p..p+k`) + the two
    // resource intrinsics (`resource-new` = core func `p+k+1`, `resource-rep` = core func `p+k+2`) → core
    // instance 3 (what `main_core` binds its `heap` import to).
    let heap_exports = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, (p + i) as u32))
            .collect();
        ex.push((RESOURCE_NEW, (p + k + 1) as u32));
        ex.push((RESOURCE_REP, (p + k + 2) as u32));
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    // sec 1: the program core module (module 1).
    out.extend_from_slice(&core_module_section(main_core));
    // sec 2: instantiate the program module (module 1) threading `peer` = core instance 0 AND `heap` = core
    // instance 3 → core instance 4.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(
            1,
            &core_instantiate_item(1, &[(PEER_MODULE, 0), (HEAP_MODULE, 3)]),
        ),
    ));
    // sec 6: alias the boundary exports off the program instance (core instance 4). Program core funcs are
    // shifted by the `p+k+2` imports: `make` = core func `p+k+3`, `t-encode` = `p+k+4`, `memory` = memory 0,
    // `cabi_realloc` = `p+k+5`.
    let boundary_aliases = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(4, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(4, ENCODE_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(4, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(4, REALLOC_EXPORT));
        section(sec::ALIAS, &wasm_vec(4, &items))
    };
    out.extend_from_slice(&boundary_aliases);
    // sec 7: [minted tuple types per compound make-param] then `own<t>` and the `make` functype. The
    // resource is component type `g+1`, and the minted tuple types start AFTER it (at `g+2`), so
    // `own`/`borrow` reference `g+1`. `shift` = compound-param count (tuple types minted at `g+2..`).
    let base = (g + 2) as u32; // first free component type after the g+1 instance/resource types
    let shift = call_arg_tuple_type_count(make_slots);
    let own_ty = base + shift;
    let make_ft = base + 1 + shift;
    let make_types = {
        let mut next_type = base;
        let mut items = Vec::new();
        let tup_idxs = mint_call_arg_tuple_types(make_slots, &mut next_type, &mut items);
        items.extend_from_slice(&own_item(res_ty));
        items.extend_from_slice(&make_functype_slots(
            make_slots,
            &tup_idxs,
            &owned_valtype(own_ty),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2 + shift as usize, &items))
    };
    out.extend_from_slice(&make_types);
    // sec 8: lift `make` (core func `p+k+3`) against the make functype → component func `p+k`.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((p + k + 3) as u32, make_ft)),
    ));
    // sec 7: `borrow<t>`, the shared `list u8` type, then the `encode` functype `(self: borrow<t>) ->
    // list<u8>` — each after make's types.
    let borrow_ty = base + 2 + shift;
    let list_ty = base + 3 + shift;
    let encode_ft = base + 4 + shift;
    let encode_types = {
        let mut items = borrow_item(res_ty); // borrow<resource> — resource is component type g+1
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(borrow_ty, list_ty));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_types);
    // sec 8: lift `encode` (core func `p+k+4`) carrying Memory 0 + Realloc (core func `p+k+5`) → component
    // func `p+k+1`.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((p + k + 4) as u32, 0, (p + k + 5) as u32, encode_ft),
        ),
    ));
    // sec 4: the nested re-export component (BORROW variant), matching the borrow lift above.
    out.extend_from_slice(&component_section(&resource_inner_component_borrow(
        make_slots,
    )));
    // sec 5: instantiate the inner component (component 0) with the resource (comp type `g+1`) + the two
    // lifted funcs (comp funcs `p+k`, `p+k+1`) → component instance `g+1` (after the g+1 imported instances).
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_item(res_ty, (p + k) as u32, (p + k + 1) as u32),
        ),
    ));
    // sec 11: export the instantiated inner component as `cadenza:run/run`. The imports are component
    // instances `0..g` (peers) + `g` (runtime), so the inner re-export instantiation is comp instance `g+1`.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(RUN_INTERFACE, (g + 1) as u32)),
    ));
    out
}

/// The resource-escape × HOST-effect FUSION — the host-side mirror of [`assemble_extern_runtime_resource`].
/// A HOST-delegated op (not peer-bound) reached in a body whose ENTRYPOINT RESULT escapes as a runtime
/// resource (main RETURNS the compound the host op contributed to) needs its host import carried into the
/// resource component. Composes [`assemble_runtime_resource`] with a leading SINGLE host interface import.
///
/// SIMPLER than the peer twin: a host effect is delegated through ONE `"host"` core module (no per-interface
/// grouping), so `g == 1` always — one host instance-type (comp type 0), one host import (comp instance 0),
/// the runtime instance-type/import at comp type/instance 1, and the resource type at comp type 2. The `h`
/// host ops lower to core funcs `0..h`, runtime ops `h..h+k`, then the resource machinery at the SAME
/// op-count-relative offsets the peer shape uses (t-dtor `h+k`, resource.new `h+k+1`, resource.rep `h+k+2`,
/// make `h+k+3`, t-encode `h+k+4`, cabi_realloc `h+k+5`). The core module (built the same way as the host+
/// runtime shapes) imports `"host"` (h ops) then `"heap"` (k ops + 2 intrinsics). SCOPE: SCALAR/unit host
/// ops (a string-param host op needs the `_mem` variant — a later increment mirroring `assemble_host_runtime`
/// vs `assemble_host_runtime_mem`; a wasm memory import is import-desc 0x02 and shifts no core-func index,
/// only the core-instance numbering). `make`/`encode` only (no scalar methods — the `_with_methods` host
/// variant is the String/Bytes-result path, a separate increment).
#[allow(clippy::too_many_arguments)]
pub fn assemble_host_runtime_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    iface: &str,
    host_fns: &[HostFn],
    make_slots: &[ArgSlot],
) -> Vec<u8> {
    let h = host_fns.len();
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: TWO instance-types — the host effect (comp type 0, its h ops) then the runtime (comp type 1,
    // its k ops).
    let type_sec = {
        let host_it = host_effect_instance_type(
            host_fns,
            host_fns.iter().any(|f| f.has_list_param),
            &[],
            &[],
        );
        let rt_it = runtime_op_instance_type(imports);
        let mut items = host_it;
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&type_sec);

    // sec 10: import the host effect (comp type 0, kebab iface name via the ops' shared iface) → comp
    // instance 0, THEN the runtime (comp type 1, `import_name`) → comp instance 1. The host effect's
    // interface name is taken from the first host op's declared iface (all host ops here share one effect).
    let import_sec = {
        let mut items = Vec::new();
        let mut he = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            iface,
        ));
        he.push(0x05); // ComponentTypeRef::Instance
        uleb128(0, &mut he);
        items.extend_from_slice(&he);
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(1, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&import_sec);

    // sec 6: alias each HOST op out of comp instance 0 (→ comp funcs `0..h`), then each RUNTIME op out of
    // comp instance 1 (→ comp funcs `h..h+k`).
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(1, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
    };
    out.extend_from_slice(&op_alias_sec);

    // sec 8: canon-lower each aliased op (comp funcs `0..h+k`) → core funcs `0..h+k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    };
    out.extend_from_slice(&lower_sec);

    // sec 2: the host core instance exporting the lowered HOST ops (core funcs `0..h`) under their op names
    // → core instance 0 (the program module's `"host"` import binds here).
    let host_core_inst = {
        let ex: Vec<(&str, u32)> = host_fns
            .iter()
            .enumerate()
            .map(|(i, f)| (f.op.as_str(), i as u32))
            .collect();
        core_export_instance_item(&ex)
    };
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &wasm_vec(1, &host_core_inst)));

    // sec 2: the `heap-dtor` core instance exporting the lowered `drop` op (core func `h + drop's runtime
    // index`) as `drop` → core instance 1.
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| (h + i) as u32)
        .expect("the runtime-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    // sec 1: the dtor core module (module 0) — imports `heap-dtor.drop`, calls it in `t-dtor`.
    out.extend_from_slice(&core_module_section(dtor_core));
    // sec 2: instantiate the dtor module threading `heap-dtor` = core instance 1 → core instance 2.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 1)])),
    ));
    // sec 6: alias `t-dtor` out of core instance 2 → core func `h+k`.
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(2, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func `h+k`) → component type 2 (after the host
    // instance-type 0 and the runtime instance-type 1).
    let res_ty = 2u32;
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item((h + k) as u32)),
    ));
    // sec 8: canon `resource.new` (→ core func `h+k+1`) AND `resource.rep` (→ core func `h+k+2`).
    let resource_canons = {
        let mut items = resource_new_item(res_ty);
        items.extend_from_slice(&resource_rep_item(res_ty));
        section(sec::CANON, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&resource_canons);
    // sec 2: the `heap` core instance exporting the k lowered runtime ops (funcs `h..h+k`) + the two resource
    // intrinsics (`resource-new` = `h+k+1`, `resource-rep` = `h+k+2`) → core instance 3.
    let heap_exports = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, (h + i) as u32))
            .collect();
        ex.push((RESOURCE_NEW, (h + k + 1) as u32));
        ex.push((RESOURCE_REP, (h + k + 2) as u32));
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    // sec 1: the program core module (module 1).
    out.extend_from_slice(&core_module_section(main_core));
    // sec 2: instantiate the program module (module 1) threading `host` = core instance 0 AND `heap` = core
    // instance 3 → core instance 4.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(
            1,
            &core_instantiate_item(1, &[(HOST_MODULE, 0), (HEAP_MODULE, 3)]),
        ),
    ));
    // sec 6: alias the boundary exports off the program instance (core instance 4). Program core funcs are
    // shifted by the `h+k+2` imports: `make` = `h+k+3`, `t-encode` = `h+k+4`, `memory` = memory 0,
    // `cabi_realloc` = `h+k+5`.
    let boundary_aliases = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(4, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(4, ENCODE_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(4, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(4, REALLOC_EXPORT));
        section(sec::ALIAS, &wasm_vec(4, &items))
    };
    out.extend_from_slice(&boundary_aliases);
    // sec 7: [minted tuple types per compound make-param] then `own<t>` and the `make` functype. The
    // resource is comp type 2, minted tuple types start at comp type 3 (`base`), `own`/`borrow` ref comp
    // type 2.
    let base = 3u32; // first free component type after the 2 instance-types + resource type
    let shift = call_arg_tuple_type_count(make_slots);
    let own_ty = base + shift;
    let make_ft = base + 1 + shift;
    let make_types = {
        let mut next_type = base;
        let mut items = Vec::new();
        let tup_idxs = mint_call_arg_tuple_types(make_slots, &mut next_type, &mut items);
        items.extend_from_slice(&own_item(res_ty));
        items.extend_from_slice(&make_functype_slots(
            make_slots,
            &tup_idxs,
            &owned_valtype(own_ty),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2 + shift as usize, &items))
    };
    out.extend_from_slice(&make_types);
    // sec 8: lift `make` (core func `h+k+3`) against the make functype → component func `h+k`.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((h + k + 3) as u32, make_ft)),
    ));
    // sec 7: `borrow<t>`, the shared `list u8` type, then the `encode` functype → each after make's types.
    let borrow_ty = base + 2 + shift;
    let list_ty = base + 3 + shift;
    let encode_ft = base + 4 + shift;
    let encode_types = {
        let mut items = borrow_item(res_ty);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(borrow_ty, list_ty));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_types);
    // sec 8: lift `encode` (core func `h+k+4`) carrying Memory 0 + Realloc (core func `h+k+5`) → comp func
    // `h+k+1`.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((h + k + 4) as u32, 0, (h + k + 5) as u32, encode_ft),
        ),
    ));
    // sec 4: the nested re-export component (BORROW variant).
    out.extend_from_slice(&component_section(&resource_inner_component_borrow(
        make_slots,
    )));
    // sec 5: instantiate the inner component (component 0) with the resource (comp type 2) + the two lifted
    // funcs (comp funcs `h+k`, `h+k+1`) → component instance 2 (after the 2 imported instances 0/1).
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_item(res_ty, (h + k) as u32, (h + k + 1) as u32),
        ),
    ));
    // sec 11: export the instantiated inner component as `cadenza:run/run` → the imports are comp instances
    // 0 (host) + 1 (runtime), so the inner re-export instantiation is comp instance 2.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(RUN_INTERFACE, 2)),
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
    make_param_bytes: &[u8],
) -> Vec<u8> {
    // `len` is the single scalar method `len : borrow<t> -> u32`; the generic path is the one hand-emit.
    assemble_runtime_resource_with_scalar_methods(
        main_core,
        dtor_core,
        imports,
        import_name,
        make_param_bytes,
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
    make_param_bytes: &[u8],
    methods: &[ScalarMethod],
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // Sections through the boundary aliases are IDENTICAL to `assemble_runtime_resource` (the shared
    // prologue: import instance-type, runtime import, op aliases + lowers, dtor instance/module/instance,
    // t-dtor alias, resource type, resource.new/rep canons, heap instance, program module/instance).
    // Re-emitted here rather than factored to keep each hand-emit a single auditable byte stream.
    let instance_type = runtime_op_instance_type(imports);
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
    // `assemble_runtime_resource`, including the `make(make-params…) -> own<t>` param forwarding.
    let make_types = {
        let mut items = own_item(1);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(2)));
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
    // sec 4: the nested re-export component with make/encode + each scalar method; `make` carries the
    // same forwarded params as the outer lift.
    out.extend_from_slice(&component_section(
        &resource_inner_component_scalar_methods(make_param_bytes, methods),
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

/// The resource-escape × peer-extern FUSION for the WITH-METHODS shape (a String/Bytes result escaping the
/// entrypoint while a peer op is reached). [`assemble_runtime_resource_with_scalar_methods`] plus leading
/// peer interface imports — the exact composition [`assemble_extern_runtime_resource`] applies to the plain
/// (no-methods) resource, extended to carry the make + encode + N scalar methods. `g = distinct interfaces`
/// at comp instances/types `0..g`, runtime at instance/type `g`, resource type at comp type `g+1`; every
/// runtime/resource CORE-func index shifts by `p = peer_fns.len()` (peer ops `0..p`, runtime `p..p+k`,
/// resource-new/rep `p+k`/`p+k+1`, make `p+k+3`, t-encode `p+k+4`, cabi_realloc `p+k+5`, method `i` at
/// `p+k+6+i`); component funcs shift by `p` (make = comp func `p+k`, encode `p+k+1`, method `i` `p+k+2+i`);
/// the inner re-export instantiation is comp instance `g+1`. Core-func indices/instances are INDEPENDENT of
/// `g` (all peer ops import from one `"peer"` core module). The core module (built by
/// `runtime_resource_core_module_form_ex2` with the `RuntimeBytes` form + methods) imports both `"peer"`
/// and `"heap"`.
#[allow(clippy::too_many_arguments)]
pub fn assemble_extern_runtime_resource_with_scalar_methods(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    peer_fns: &[HostFn],
    op_ifaces: &[&str],
    make_param_bytes: &[u8],
    methods: &[ScalarMethod],
) -> Vec<u8> {
    let p = peer_fns.len();
    let k = imports.len();
    let ifaces = distinct_ifaces(op_ifaces);
    let g = ifaces.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: g+1 instance-types — one per distinct peer interface (comp types `0..g`) then runtime (`g`).
    let type_sec = {
        let mut items = Vec::new();
        for iface in &ifaces {
            let ops = peer_group_ops(peer_fns, op_ifaces, iface);
            let mut decls = Vec::new();
            for (local, f) in ops.iter().enumerate() {
                decls.push(0x01);
                decls.extend_from_slice(&f.comp_functype);
                decls.push(0x04);
                decls.extend_from_slice(&extern_name(
                    &crate::backend::common::export_name::kebab_extern_name(&f.op),
                ));
                decls.push(0x01);
                uleb128(local as u64, &mut decls);
            }
            let mut it = vec![0x42];
            it.extend_from_slice(&wasm_vec(2 * ops.len(), &decls));
            items.extend_from_slice(&it);
        }
        let rt_it = runtime_op_instance_type(imports);
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(g + 1, &items))
    };
    out.extend_from_slice(&type_sec);

    // sec 10: import each PEER interface (comp type `g_idx`) → comp instances `0..g`, then the runtime (comp
    // type `g`) → comp instance `g`.
    let import_sec = {
        let mut items = Vec::new();
        for (g_idx, iface) in ifaces.iter().enumerate() {
            let mut pe = extern_name(&crate::backend::common::export_name::kebab_extern_name(
                iface,
            ));
            pe.push(0x05);
            uleb128(g_idx as u64, &mut pe);
            items.extend_from_slice(&pe);
        }
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(g as u64, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(g + 1, &items))
    };
    out.extend_from_slice(&import_sec);

    // sec 6: alias each peer op out of ITS interface's instance (→ comp funcs `0..p`), then runtime ops out
    // of comp instance `g` (→ comp funcs `p..p+k`).
    let op_alias_sec = {
        let mut items = Vec::new();
        for (f, &oi) in peer_fns.iter().zip(op_ifaces) {
            let inst = iface_index(&ifaces, oi);
            items.extend_from_slice(&comp_alias_item(
                inst as u32,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(g as u32, op.name));
        }
        section(sec::ALIAS, &wasm_vec(p + k, &items))
    };
    out.extend_from_slice(&op_alias_sec);
    // sec 8: canon-lower each aliased op (comp funcs `0..p+k`) → core funcs `0..p+k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..(p + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(p + k, &items))
    };
    out.extend_from_slice(&lower_sec);
    // sec 2: peer core instance exporting the lowered peer ops (core funcs `0..p`) → core instance 0.
    let peer_core_inst = {
        let ex: Vec<(&str, u32)> = peer_fns
            .iter()
            .enumerate()
            .map(|(i, f)| (f.op.as_str(), i as u32))
            .collect();
        core_export_instance_item(&ex)
    };
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &wasm_vec(1, &peer_core_inst)));
    // sec 2: `heap-dtor` instance exporting the lowered `drop` (at `p + drop's runtime index`) → core
    // instance 1.
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| (p + i) as u32)
        .expect("the runtime-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    out.extend_from_slice(&core_module_section(dtor_core));
    // sec 2: instantiate the dtor module threading `heap-dtor` = core instance 1 → core instance 2.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 1)])),
    ));
    // sec 6: alias `t-dtor` out of core instance 2 → core func `p+k`.
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(2, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func `p+k`) → component type `g+1` (after the g
    // peer instance-types `0..g` and the runtime instance-type `g`).
    let res_ty = (g + 1) as u32;
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item((p + k) as u32)),
    ));
    // sec 8: canon resource.new (→ core func `p+k+1`) + resource.rep (→ core func `p+k+2`) for the resource
    // (comp type `g+1`).
    let resource_canons = {
        let mut items = resource_new_item(res_ty);
        items.extend_from_slice(&resource_rep_item(res_ty));
        section(sec::CANON, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&resource_canons);
    // sec 2: `heap` instance exporting the k runtime ops (funcs `p..p+k`) + resource-new (`p+k+1`) +
    // resource-rep (`p+k+2`) → core instance 3.
    let heap_exports = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, (p + i) as u32))
            .collect();
        ex.push((RESOURCE_NEW, (p + k + 1) as u32));
        ex.push((RESOURCE_REP, (p + k + 2) as u32));
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    // sec 1: the program core module (module 1).
    out.extend_from_slice(&core_module_section(main_core));
    // sec 2: instantiate the program module threading `peer` = core instance 0 AND `heap` = core instance 3
    // → core instance 4.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(
            1,
            &core_instantiate_item(1, &[(PEER_MODULE, 0), (HEAP_MODULE, 3)]),
        ),
    ));
    // sec 6: alias the boundary exports off the program instance (core instance 4): make `p+k+3`, t-encode
    // `p+k+4`, memory, cabi_realloc `p+k+5`, THEN each method's `t-<name>` (core func `p+k+6+i`).
    let boundary_aliases = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(4, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(4, ENCODE_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(4, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(4, REALLOC_EXPORT));
        for m in methods {
            items.extend_from_slice(&core_alias_item(4, m.core_export));
        }
        section(sec::ALIAS, &wasm_vec(4 + methods.len(), &items))
    };
    out.extend_from_slice(&boundary_aliases);
    // sec 7 + 8: make (own<t> at type `g+2`, make-ft at `g+3` → comp func `p+k`) and encode (borrow<t> type
    // `g+4` + list type `g+5` + encode-ft `g+6` → comp func `p+k+1`). All shifted by `g-1` vs the g=1 case.
    let own_ty = (g + 2) as u32;
    let make_ft = (g + 3) as u32;
    let borrow_ty = (g + 4) as u32;
    let list_ty = (g + 5) as u32;
    let encode_ft = (g + 6) as u32;
    let make_types = {
        let mut items = own_item(res_ty);
        items.extend_from_slice(&params_result_functype(
            make_param_bytes,
            &owned_valtype(own_ty),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_types);
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((p + k + 3) as u32, make_ft)),
    ));
    let encode_types = {
        let mut items = borrow_item(res_ty);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(borrow_ty, list_ty));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_types);
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((p + k + 4) as u32, 0, (p + k + 5) as u32, encode_ft),
        ),
    ));
    // Per method `i`: functype (comp type `g+7+i`) REUSING borrow<t> defined type `g+4` (+ list type `g+5`),
    // then a canon lift of core func `p+k+6+i` → comp func `p+k+2+i`.
    for (i, m) in methods.iter().enumerate() {
        let ty_idx = (g + 7) as u32 + i as u32;
        let functype = match m.result {
            MethodResult::Scalar(prim) => self_borrow_to_scalar_functype(borrow_ty, prim),
            MethodResult::ListU8 => self_borrow_to_list_functype(borrow_ty, list_ty),
        };
        out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(1, &functype)));
        let lift = match m.result {
            MethodResult::Scalar(_) => canon_lift_item((p + k + 6 + i) as u32, ty_idx),
            MethodResult::ListU8 => {
                canon_lift_list_item((p + k + 6 + i) as u32, 0, (p + k + 5) as u32, ty_idx)
            }
        };
        out.extend_from_slice(&section(sec::CANON, &wasm_vec(1, &lift)));
    }
    // sec 4: the nested re-export component (make/encode + methods).
    out.extend_from_slice(&component_section(
        &resource_inner_component_scalar_methods(make_param_bytes, methods),
    ));
    // sec 5: instantiate the inner component with the resource (comp type `g+1`) + lifted funcs (make comp
    // func `p+k`, encode `p+k+1`, method i `p+k+2+i`) → component instance `g+1`.
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_scalar_methods_item(res_ty, (p + k) as u32, methods),
        ),
    ));
    // sec 11: export the inner instance as `cadenza:run/run` (the imports are comp instances `0..g` peers +
    // `g` runtime, so the inner instantiation is comp instance `g+1`).
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(RUN_INTERFACE, (g + 1) as u32)),
    ));
    out
}

/// The resource-escape × HOST-effect FUSION for the WITH-METHODS shape — the host-side mirror of
/// [`assemble_extern_runtime_resource_with_scalar_methods`], exactly as [`assemble_host_runtime_resource`]
/// mirrors [`assemble_extern_runtime_resource`] for the plain (no-methods) resource. A host-delegated effect
/// (imported from the single `"host"` module) is reached in a body whose STRING/Bytes result escapes the
/// entrypoint, so the component carries make + encode + N scalar borrow methods (`len`/`is-empty`/`to-bytes`)
/// on top of the host interface + runtime imports. This is the SINGLE-host-interface case (`g == 1`): the host
/// effect at comp instance/type 0, runtime at comp instance/type 1, resource type at comp type 2 — the same
/// index layout as [`assemble_host_runtime_resource`], with the with-methods make/encode/method tail of the
/// peer twin. Host ops lower to core funcs `0..h` from the `"host"` core module; runtime `h..h+k`; resource-
/// new/rep `h+k+1`/`h+k+2`; make `h+k+3`, t-encode `h+k+4`, cabi_realloc `h+k+5`, method `i` at `h+k+6+i`;
/// component funcs: make comp func `h+k`, encode `h+k+1`, method `i` `h+k+2+i`; the inner re-export
/// instantiation is comp instance 2. (A String-param host op — the shared-memory `_mem` variant — is a later
/// increment, declined at the call site; a multi-host-effect shape is declined there too.)
#[allow(clippy::too_many_arguments)]
pub fn assemble_host_runtime_resource_with_scalar_methods(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    iface: &str,
    host_fns: &[HostFn],
    make_param_bytes: &[u8],
    methods: &[ScalarMethod],
) -> Vec<u8> {
    let h = host_fns.len();
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: TWO instance-types — the host effect (comp type 0, its h ops) then the runtime (comp type 1).
    let type_sec = {
        let host_it = host_effect_instance_type(
            host_fns,
            host_fns.iter().any(|f| f.has_list_param),
            &[],
            &[],
        );
        let rt_it = runtime_op_instance_type(imports);
        let mut items = host_it;
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&type_sec);

    // sec 10: import the host effect (comp type 0, kebab iface name) → comp instance 0, then the runtime
    // (comp type 1, `import_name`) → comp instance 1.
    let import_sec = {
        let mut items = Vec::new();
        let mut he = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            iface,
        ));
        he.push(0x05); // ComponentTypeRef::Instance
        uleb128(0, &mut he);
        items.extend_from_slice(&he);
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(1, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&import_sec);

    // sec 6: alias each HOST op out of comp instance 0 (→ comp funcs `0..h`), then each RUNTIME op out of
    // comp instance 1 (→ comp funcs `h..h+k`).
    let op_alias_sec = {
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(1, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
    };
    out.extend_from_slice(&op_alias_sec);
    // sec 8: canon-lower each aliased op (comp funcs `0..h+k`) → core funcs `0..h+k`.
    let lower_sec = {
        let mut items = Vec::new();
        for i in 0..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    };
    out.extend_from_slice(&lower_sec);
    // sec 2: the host core instance exporting the lowered HOST ops (core funcs `0..h`) → core instance 0
    // (the program module's `"host"` import binds here).
    let host_core_inst = {
        let ex: Vec<(&str, u32)> = host_fns
            .iter()
            .enumerate()
            .map(|(i, f)| (f.op.as_str(), i as u32))
            .collect();
        core_export_instance_item(&ex)
    };
    out.extend_from_slice(&section(sec::CORE_INSTANCE, &wasm_vec(1, &host_core_inst)));
    // sec 2: `heap-dtor` instance exporting the lowered `drop` (at `h + drop's runtime index`) → core
    // instance 1.
    let drop_core = imports
        .iter()
        .position(|op| op.name == RUNTIME_DROP)
        .map(|i| (h + i) as u32)
        .expect("the runtime-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    out.extend_from_slice(&core_module_section(dtor_core));
    // sec 2: instantiate the dtor module threading `heap-dtor` = core instance 1 → core instance 2.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 1)])),
    ));
    // sec 6: alias `t-dtor` out of core instance 2 → core func `h+k`.
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(2, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (rep i32, dtor = core func `h+k`) → component type 2 (after the host
    // instance-type 0 and the runtime instance-type 1).
    let res_ty = 2u32;
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item((h + k) as u32)),
    ));
    // sec 8: canon resource.new (→ core func `h+k+1`) + resource.rep (→ core func `h+k+2`).
    let resource_canons = {
        let mut items = resource_new_item(res_ty);
        items.extend_from_slice(&resource_rep_item(res_ty));
        section(sec::CANON, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&resource_canons);
    // sec 2: `heap` instance exporting the k runtime ops (funcs `h..h+k`) + resource-new (`h+k+1`) +
    // resource-rep (`h+k+2`) → core instance 3.
    let heap_exports = {
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, (h + i) as u32))
            .collect();
        ex.push((RESOURCE_NEW, (h + k + 1) as u32));
        ex.push((RESOURCE_REP, (h + k + 2) as u32));
        ex
    };
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&heap_exports)),
    ));
    // sec 1: the program core module (module 1).
    out.extend_from_slice(&core_module_section(main_core));
    // sec 2: instantiate the program module threading `host` = core instance 0 AND `heap` = core instance 3
    // → core instance 4.
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(
            1,
            &core_instantiate_item(1, &[(HOST_MODULE, 0), (HEAP_MODULE, 3)]),
        ),
    ));
    // sec 6: alias the boundary exports off the program instance (core instance 4): make `h+k+3`, t-encode
    // `h+k+4`, memory, cabi_realloc `h+k+5`, THEN each method's `t-<name>` (core func `h+k+6+i`).
    let boundary_aliases = {
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(4, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(4, ENCODE_CORE_EXPORT));
        items.extend_from_slice(&memory_alias_item(4, MEMORY_EXPORT));
        items.extend_from_slice(&core_alias_item(4, REALLOC_EXPORT));
        for m in methods {
            items.extend_from_slice(&core_alias_item(4, m.core_export));
        }
        section(sec::ALIAS, &wasm_vec(4 + methods.len(), &items))
    };
    out.extend_from_slice(&boundary_aliases);
    // sec 7 + 8: make (own<t> at comp type 3, make-ft at 4 → comp func `h+k`) and encode (borrow<t> type 5 +
    // list type 6 + encode-ft 7 → comp func `h+k+1`). The single-host-interface (g=1) layout matches the
    // peer twin's `g+2`/`g+3`/… with g=1.
    let own_ty = 3u32;
    let make_ft = 4u32;
    let borrow_ty = 5u32;
    let list_ty = 6u32;
    let encode_ft = 7u32;
    let make_types = {
        let mut items = own_item(res_ty);
        items.extend_from_slice(&params_result_functype(
            make_param_bytes,
            &owned_valtype(own_ty),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_types);
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((h + k + 3) as u32, make_ft)),
    ));
    let encode_types = {
        let mut items = borrow_item(res_ty);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(borrow_ty, list_ty));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_types);
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((h + k + 4) as u32, 0, (h + k + 5) as u32, encode_ft),
        ),
    ));
    // Per method `i`: functype (comp type `8+i`) REUSING borrow<t> defined type 5 (+ list type 6), then a
    // canon lift of core func `h+k+6+i` → comp func `h+k+2+i`.
    for (i, m) in methods.iter().enumerate() {
        let ty_idx = 8u32 + i as u32;
        let functype = match m.result {
            MethodResult::Scalar(prim) => self_borrow_to_scalar_functype(borrow_ty, prim),
            MethodResult::ListU8 => self_borrow_to_list_functype(borrow_ty, list_ty),
        };
        out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(1, &functype)));
        let lift = match m.result {
            MethodResult::Scalar(_) => canon_lift_item((h + k + 6 + i) as u32, ty_idx),
            MethodResult::ListU8 => {
                canon_lift_list_item((h + k + 6 + i) as u32, 0, (h + k + 5) as u32, ty_idx)
            }
        };
        out.extend_from_slice(&section(sec::CANON, &wasm_vec(1, &lift)));
    }
    // sec 4: the nested re-export component (make/encode + methods).
    out.extend_from_slice(&component_section(
        &resource_inner_component_scalar_methods(make_param_bytes, methods),
    ));
    // sec 5: instantiate the inner component with the resource (comp type 2) + lifted funcs (make comp func
    // `h+k`, encode `h+k+1`, method i `h+k+2+i`) → component instance 2.
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_scalar_methods_item(res_ty, (h + k) as u32, methods),
        ),
    ));
    // sec 11: export the inner instance as `cadenza:run/run` — imports are comp instances 0 (host) + 1
    // (runtime), so the inner instantiation is comp instance 2.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(RUN_INTERFACE, 2)),
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
    assemble_closure_resource_borrow(
        main_core,
        dtor_core,
        imports,
        import_name,
        make_param_bytes,
        arg_bytes,
        result_byte,
        false,
    )
}

/// [`assemble_closure_resource`] with a `call_borrow` switch. When TRUE the `call` method's `self` is typed
/// `borrow<t>` (both on the outer lift and in the nested re-export component) instead of `own<t>`, so the
/// host KEEPS the handle across calls (a REPEATABLE closure — the natural callback shape) and the `t-dtor`
/// reclaims the cell when the host finally drops it. Pairs with
/// [`serialize::closure_resource_core_module_borrow`]`(…, true)` (the `call` body uses the passed rep
/// directly, no `resource.rep`, no self-drop). `false` reproduces the shipped own/self-drop single-use
/// component byte-for-byte. Only `call`'s `own_item(1)`→`borrow_item(1)` differs; `make` stays `own<t>`
/// (it MINTS the handle and transfers ownership OUT to the host).
#[allow(clippy::too_many_arguments)]
pub fn assemble_closure_resource_borrow(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
    call_borrow: bool,
) -> Vec<u8> {
    assemble_closure_resource_borrow_tuple(
        main_core,
        dtor_core,
        imports,
        import_name,
        make_param_bytes,
        arg_bytes,
        result_byte,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`assemble_closure_resource_borrow`] with an optional FIXED-SHAPE SCALAR tuple ARGUMENT (the direct-call
/// compound-arg path). When `tuple_arg_bytes` is `Some(field_bytes)`, the `call` method's single argument is
/// a native `tuple<field_bytes…>` DEFINED type — minted just before the outer `call` lift functype (shifting
/// it from comp type 5 to 6) and inside the nested re-export component — instead of `arg_bytes`'s inline
/// scalar params. The canonical ABI FLATTENS the tuple into scalar core params on lift, which the core
/// `call` (built by `serialize`'s `TupleArgRebuild`) rebuilds into a cell. `None` reproduces the scalar path
/// byte-for-byte. Pairs with a core whose `call` was serialized with a matching `TupleArgRebuild`.
#[allow(clippy::too_many_arguments)]
pub fn assemble_closure_resource_borrow_tuple(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    // The scalar boundary bytes BEFORE / AFTER the tuple arg (in the closure's original arg order) when the
    // tuple sits AMONG scalar args; both empty when the tuple is the SOLE arg (or `tuple_arg_bytes` is None).
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    // When the tuple arg has a NESTED fixed-shape compound field, its recursive field shape (so the envelope
    // mints the inner `tuple<…>` types by index). `None` = an all-scalar tuple (the flat `tuple_arg_bytes`
    // path, byte-identical). Only the single-export scalar-result path threads this; other paths pass `None`.
    tuple_shape: Option<&[TupleFieldShape]>,
    // The N-COMPOUND-ARGS override: when `Some(slots)`, the `call` args are described by the ordered slot list
    // (scalars + fixed-shape tuples interleaved, TWO+ tuples allowed) — the slot model drives type minting +
    // the `call` functype, subsuming `tuple_arg_bytes`/`tuple_prefix_bytes`/`tuple_suffix_bytes`/`tuple_shape`.
    // `None` reproduces the single-tuple (or scalar) path byte-for-byte. Only the single-export scalar-result
    // path threads a `Some` (with ≥2 tuple slots); every other caller passes `None`.
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: the import instance-type declaring the k used runtime ops (component type 0).
    let instance_type = runtime_op_instance_type(imports);
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
    // sec 7: `own<t>`/`borrow<t>` (type 4) then — for a tuple arg — the `tuple<…>` defined type (type 5),
    // then the `call` functype `(self: <handle<t>>, <args>) -> R`. `own<t>` CONSUMES self per call
    // (single-use); `borrow<t>` keeps the handle across calls (repeatable). With a tuple arg the call
    // functype's own index shifts to 6 (the tuple sits between); the scalar path keeps it at 5.
    let call_ft_idx: u32;
    out.extend_from_slice(&{
        let mut items = if call_borrow {
            borrow_item(1)
        } else {
            own_item(1)
        };
        let n_items: usize;
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint every tuple slot's type(s) starting at type 5 (after the handle at 4), in
            // arg order; the `call` functype references each by index (a scalar slot inlines its byte). The
            // functype sits right after all the minted tuple types.
            let mut next_type = 5u32;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_functype_slots(
                4,
                slots,
                &tup_idxs,
                result_byte,
            ));
            call_ft_idx = next_type;
            n_items = 1 + call_arg_tuple_type_count(slots) as usize + 1; // handle + tuple types + functype
        } else if let Some(shape) = tuple_shape {
            // NESTED tuple arg: mint the (possibly multi-level) tuple types starting at type 5; the OUTERMOST
            // tuple index is what the `call` functype references. `nested_tuple_type_count` types precede it.
            let mut next_type = 5u32;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                4,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                result_byte,
            ));
            call_ft_idx = next_type; // the functype sits right after all the tuple types
            n_items = 1 + nested_tuple_type_count(shape) as usize + 1; // handle + tuple types + functype
        } else if let Some(fields) = tuple_arg_bytes {
            items.extend_from_slice(&tuple_defined_type(fields)); // type 5
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                4,
                tuple_prefix_bytes,
                5,
                tuple_suffix_bytes,
                result_byte,
            )); // type 6
            call_ft_idx = 6;
            n_items = 3;
        } else {
            items.extend_from_slice(&closure_call_functype(4, arg_bytes, result_byte)); // type 5
            call_ft_idx = 5;
            n_items = 2;
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(n_items, &items))
    });
    // sec 8: lift `call` (core func k+4) against the call functype → component func k+1. No canon options
    // (scalar/flattened-tuple args + scalar result — no memory/realloc needed).
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((k + 4) as u32, call_ft_idx)),
    ));
    // sec 4: the nested re-export component. sec 5: instantiate it (comp type 1 + comp funcs k, k+1) →
    // component instance 1 (the runtime import is component instance 0). sec 11: export as the closure
    // interface.
    out.extend_from_slice(&component_section(
        &resource_inner_component_closure_borrow_tuple(
            make_param_bytes,
            arg_bytes,
            result_byte,
            call_borrow,
            tuple_arg_bytes,
            tuple_prefix_bytes,
            tuple_suffix_bytes,
            tuple_shape,
            call_arg_slots,
        ),
    ));
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

/// The HOST + value-heap-runtime CLOSURE-RESOURCE component (the build-time-delegated closure-capture
/// case): a closure export whose `make` code delegates a host effect (`(host (ask) (let ((v (ask.ask)))
/// (fn (x) (+ x v))))`). A fork of [`assemble_closure_resource`] that ALSO imports the host effect
/// interface (as `"host"`), composing the two import spaces exactly as [`assemble_host_runtime`] fused
/// host+heap — but around the closure resource machinery. The core module is
/// [`serialize::multi_closure_resource_core_module_with_host`]'s output (host funcs `0..h`, runtime
/// `h..h+k`), so this lays the component-side host import FIRST too.
///
/// Index spaces (h = host_fns.len(), k = imports.len()): host import-instance-type → component type 0,
/// runtime import-instance-type → component type 1. Host op aliases → component funcs `0..h`, runtime op
/// aliases → `h..h+k`. Lowered ops → core funcs `0..h` (host) + `h..h+k` (runtime). Then `t-dtor` → core
/// func `h+k`, `resource.new` → `h+k+1`, `resource.rep` → `h+k+2`, aliased `make` → `h+k+3`, `call` →
/// `h+k+4`. The resource type → component type 2 (types 0,1 are the import-instance-types); make `own<t>`
/// 3 + make-ft 4; call `own<t>` 5 + call-ft 6. `make` lift → component func `h+k`, `call` lift → `h+k+1`.
/// Core instances: host (0), heap (1), dtor-source (2), dtor-module (3), program (4). SCOPE: scalar/unit
/// host ops (no host string param → no shared memory), scalar closure args/result.
#[allow(clippy::too_many_arguments)]
pub fn assemble_closure_host_runtime_resource(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    iface: &str,
    host_fns: &[HostFn],
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let h = host_fns.len();
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: TWO import instance-types — host effect (component type 0), then runtime (component type 1).
    out.extend_from_slice(&{
        let host_it = {
            let mut decls = Vec::new();
            for (i, f) in host_fns.iter().enumerate() {
                decls.push(0x01);
                decls.extend_from_slice(&f.comp_functype);
                decls.push(0x04);
                decls.extend_from_slice(&extern_name(
                    &crate::backend::common::export_name::kebab_extern_name(&f.op),
                ));
                decls.push(0x01);
                uleb128(i as u64, &mut decls);
            }
            let mut it = vec![0x42];
            it.extend_from_slice(&wasm_vec(2 * h, &decls));
            it
        };
        let rt_it = runtime_op_instance_type(imports);
        let mut items = host_it;
        items.extend_from_slice(&rt_it);
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });

    // sec 10: import the host effect interface (comp type 0 → comp instance 0), then the runtime (comp
    // type 1 → comp instance 1).
    out.extend_from_slice(&{
        let mut items = Vec::new();
        let mut eff = extern_name(&crate::backend::common::export_name::kebab_extern_name(
            iface,
        ));
        eff.push(0x05);
        uleb128(0, &mut eff);
        items.extend_from_slice(&eff);
        let mut rt = extern_name(import_name);
        rt.push(0x05);
        uleb128(1, &mut rt);
        items.extend_from_slice(&rt);
        section(sec::COMPONENT_IMPORT, &wasm_vec(2, &items))
    });

    // sec 6: alias host ops out of comp instance 0 (→ comp funcs 0..h), then runtime ops out of comp
    // instance 1 (→ comp funcs h..h+k).
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for f in host_fns {
            items.extend_from_slice(&comp_alias_item(
                0,
                &crate::backend::common::export_name::kebab_extern_name(&f.op),
            ));
        }
        for op in imports {
            items.extend_from_slice(&comp_alias_item(1, op.name));
        }
        section(sec::ALIAS, &wasm_vec(h + k, &items))
    });
    // sec 8: canon-lower each aliased op (comp funcs 0..h+k) → core funcs 0..h+k.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..(h + k) {
            items.extend_from_slice(&canon_lower_item(i as u32));
        }
        section(sec::CANON, &wasm_vec(h + k, &items))
    });

    // sec 2: host core instance (the lowered host ops under their op names → core instance 0).
    out.extend_from_slice(&{
        let host_exports: Vec<(&str, u32)> = host_fns
            .iter()
            .enumerate()
            .map(|(i, f)| (f.op.as_str(), i as u32))
            .collect();
        section(
            sec::CORE_INSTANCE,
            &wasm_vec(1, &core_export_instance_item(&host_exports)),
        )
    });

    // sec 2: `heap-dtor` core instance exporting the lowered `drop` (→ core instance 1). `drop`'s core func
    // index is `h + its runtime position`.
    let drop_core = h as u32
        + imports
            .iter()
            .position(|op| op.name == RUNTIME_DROP)
            .map(|i| i as u32)
            .expect("the closure-resource escape imports `drop` for the dtor");
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_export_instance_item(&[(RUNTIME_DROP, drop_core)])),
    ));
    // sec 1: dtor module. sec 2: instantiate it threading `heap-dtor` = core instance 1 → core instance 2.
    out.extend_from_slice(&core_module_section(dtor_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(1, &core_instantiate_item(0, &[(HEAP_DTOR_MODULE, 1)])),
    ));
    // sec 6: alias `t-dtor` off the dtor-module instance (core instance 2) → core func h+k.
    out.extend_from_slice(&section(
        sec::ALIAS,
        &wasm_vec(1, &core_alias_item(2, DTOR_CORE_EXPORT)),
    ));
    // sec 7: the resource type `t` (dtor = core func h+k) → component type 2.
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(1, &resource_type_item((h + k) as u32)),
    ));
    // sec 8: canon `resource.new` (→ core func h+k+1) AND `resource.rep` (→ core func h+k+2), on comp type 2.
    out.extend_from_slice(&{
        let mut items = resource_new_item(2);
        items.extend_from_slice(&resource_rep_item(2));
        section(sec::CANON, &wasm_vec(2, &items))
    });
    // sec 2: the `heap` core instance exporting the k lowered runtime ops (core funcs h..h+k) + resource-new
    // (h+k+1) + resource-rep (h+k+2) → core instance 3 (what `main_core` binds its `heap` import to).
    out.extend_from_slice(&{
        let mut ex: Vec<(&str, u32)> = imports
            .iter()
            .enumerate()
            .map(|(i, op)| (op.name, (h + i) as u32))
            .collect();
        ex.push((RESOURCE_NEW, (h + k + 1) as u32));
        ex.push((RESOURCE_REP, (h + k + 2) as u32));
        section(
            sec::CORE_INSTANCE,
            &wasm_vec(1, &core_export_instance_item(&ex)),
        )
    });
    // sec 1: the program core module (module 1). sec 2: instantiate threading BOTH `host` = core instance 0
    // AND `heap` = core instance 3 → core instance 4.
    out.extend_from_slice(&core_module_section(main_core));
    out.extend_from_slice(&section(
        sec::CORE_INSTANCE,
        &wasm_vec(
            1,
            &core_instantiate_item(1, &[(HOST_MODULE, 0), (HEAP_MODULE, 3)]),
        ),
    ));
    // sec 6: alias `make` + `call` off the program instance (core instance 4) → core funcs h+k+3, h+k+4.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        items.extend_from_slice(&core_alias_item(4, MAKE_CORE_EXPORT));
        items.extend_from_slice(&core_alias_item(4, CALL_CORE_EXPORT));
        section(sec::ALIAS, &wasm_vec(2, &items))
    });
    // sec 7: make `own<t>` (type 3) + make functype (type 4). Resource is comp type 2.
    out.extend_from_slice(&{
        let mut items = own_item(2);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(3)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 8: lift `make` (core func h+k+3) against functype type 4 → component func h+k.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((h + k + 3) as u32, 4)),
    ));
    // sec 7: call `own<t>` (type 5) + call functype (type 6).
    out.extend_from_slice(&{
        let mut items = own_item(2);
        items.extend_from_slice(&closure_call_functype(5, arg_bytes, result_byte));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 8: lift `call` (core func h+k+4) against functype type 6 → component func h+k+1.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(1, &canon_lift_item((h + k + 4) as u32, 6)),
    ));
    // sec 4: the nested re-export component (host-agnostic — re-exports make/call). sec 5: instantiate it
    // (comp type 2 + comp funcs h+k, h+k+1) → component instance 2 (host = comp inst 0, runtime = comp inst
    // 1). sec 11: export as the closure interface.
    out.extend_from_slice(&component_section(&resource_inner_component_closure(
        make_param_bytes,
        arg_bytes,
        result_byte,
    )));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_call_item(2, (h + k) as u32, (h + k + 1) as u32),
        ),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_instance_item(CLOSURE_INTERFACE, 2)),
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
    assemble_closure_bytes_resource_borrow(
        main_core,
        dtor_core,
        imports,
        import_name,
        make_param_bytes,
        arg_bytes,
        false,
    )
}

/// [`assemble_closure_bytes_resource`] with a `call_borrow` switch (C-HOST-6, `list<u8>`-result closures —
/// byte-rope / compound value-form / collection value-encode). When TRUE `call`'s `self` is `borrow<t>` (both
/// the outer lift and the nested re-export re-typing) so the host keeps the handle across calls; `make` stays
/// `own<t>`. Pairs with the `_borrow(…, true)` serializer cores (the `call` body uses the passed rep directly,
/// no `resource.rep`, no cell self-drop). `false` reproduces the shipped own component byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn assemble_closure_bytes_resource_borrow(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    call_borrow: bool,
) -> Vec<u8> {
    assemble_closure_bytes_resource_borrow_tuple(
        main_core,
        dtor_core,
        imports,
        import_name,
        make_param_bytes,
        arg_bytes,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`assemble_closure_bytes_resource_borrow`] with an optional fixed-shape scalar tuple ARGUMENT: when
/// `tuple_arg_bytes` is `Some(field_bytes)`, `call`'s single argument is a native `tuple<field_bytes…>`
/// (minted before the `list<u8>` result type on both the outer lift + the nested re-export, shifting the
/// `list<u8>` + call-functype indices up by 1) instead of `arg_bytes`'s inline scalars. Pairs with a bytes
/// core serialized with a matching `TupleArgRebuild`. `None` = the scalar-arg path (byte-identical).
#[allow(clippy::too_many_arguments)]
pub fn assemble_closure_bytes_resource_borrow_tuple(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    // A NESTED fixed-shape compound tuple arg's recursive field shape (mints inner `tuple<…>` types by index).
    // `None` = an all-scalar tuple (the flat `tuple_arg_bytes` path). Only the single-export path threads this.
    tuple_shape: Option<&[TupleFieldShape]>,
    // The N-COMPOUND-ARGS override (see [`assemble_closure_resource_borrow_tuple`]): when `Some(slots)`, the
    // ordered slot list drives type minting + the `call` functype (each tuple slot mints its own `tuple<…>`
    // group, in arg order, before the shared `list<u8>` result type), subsuming the single-tuple inputs.
    // `None` reproduces the single-tuple/scalar path byte-for-byte.
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    let k = imports.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: import instance-type (component type 0). — identical prologue to `assemble_closure_resource`.
    let instance_type = runtime_op_instance_type(imports);
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
    // sec 7: `own<t>`/`borrow<t>` (type 4), then — for a TUPLE arg — the `tuple<…>` type (type 5), then
    // `list<u8>`, then the `call` functype `(self, args…) -> list<u8>`. A tuple arg shifts `list<u8>` + the
    // call functype up by 1 (the tuple sits between); the scalar-arg path keeps `list<u8>`=5, call-ft=6.
    let call_ft_idx: u32;
    out.extend_from_slice(&{
        let mut items = if call_borrow {
            borrow_item(1)
        } else {
            own_item(1)
        };
        let n_items: usize;
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint every tuple slot's type(s) starting at type 5 (after the handle at 4), in
            // arg order; then `list<u8>`; then the slot-model `call` functype. Handle + tuple types + list + ft.
            let mut next_type = 5u32;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            let list_ty = next_type;
            items.extend_from_slice(&list_u8_defined_type());
            next_type += 1;
            items.extend_from_slice(&closure_call_list_functype_slots(
                4, slots, &tup_idxs, list_ty,
            ));
            call_ft_idx = next_type;
            n_items = 1 + call_arg_tuple_type_count(slots) as usize + 2;
        } else if let Some(shape) = tuple_shape {
            // NESTED tuple arg: mint the (multi-level) tuple types starting at type 5; the OUTERMOST tuple
            // index is the `call` arg, then `list<u8>`, then the functype.
            let mut next_type = 5u32;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            let list_ty = next_type;
            items.extend_from_slice(&list_u8_defined_type());
            next_type += 1;
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                4,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                list_ty,
            ));
            call_ft_idx = next_type;
            n_items = 1 + nested_tuple_type_count(shape) as usize + 2; // handle + tuple types + list + functype
        } else if let Some(fields) = tuple_arg_bytes {
            items.extend_from_slice(&tuple_defined_type(fields)); // type 5
            items.extend_from_slice(&list_u8_defined_type()); // type 6
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                4,
                tuple_prefix_bytes,
                5,
                tuple_suffix_bytes,
                6,
            )); // type 7
            call_ft_idx = 7;
            n_items = 4;
        } else {
            items.extend_from_slice(&list_u8_defined_type()); // type 5
            items.extend_from_slice(&closure_call_list_functype(4, arg_bytes, 5)); // type 6
            call_ft_idx = 6;
            n_items = 3;
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(n_items, &items))
    });
    // sec 8: lift `call` (core func k+4) against the call functype WITH Memory 0 + Realloc (core func k+5) →
    // component func k+1. The compound result crosses through linear memory by the canonical ABI.
    out.extend_from_slice(&section(
        sec::CANON,
        &wasm_vec(
            1,
            &canon_lift_list_item((k + 4) as u32, 0, (k + 5) as u32, call_ft_idx),
        ),
    ));
    // sec 4/5/11: nested re-export component; instantiate (comp type 1 + comp funcs k, k+1); export.
    out.extend_from_slice(&component_section(
        &resource_inner_component_closure_bytes_borrow_tuple(
            make_param_bytes,
            arg_bytes,
            call_borrow,
            tuple_arg_bytes,
            tuple_prefix_bytes,
            tuple_suffix_bytes,
            tuple_shape,
            call_arg_slots,
        ),
    ));
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
    assemble_mixed_closure_resource_borrow(
        main_core,
        dtor_core,
        imports,
        import_name,
        makes,
        arg_bytes,
        result_byte,
        &[],
        false,
    )
}

/// [`assemble_multi_closure_resource`] with a `call_borrow` switch (C-HOST-6, multi-export scalar). When
/// TRUE the ONE shared `call` is typed `borrow<t>` (repeatable — each make's handle survives across calls);
/// `make`s stay `own<t>`. Pairs with [`serialize::multi_closure_resource_core_module_borrow`]`(…, true)`.
#[allow(clippy::too_many_arguments)]
pub fn assemble_multi_closure_resource_borrow(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
    call_borrow: bool,
) -> Vec<u8> {
    assemble_mixed_closure_resource_borrow(
        main_core,
        dtor_core,
        imports,
        import_name,
        makes,
        arg_bytes,
        result_byte,
        &[],
        call_borrow,
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
    assemble_mixed_closure_resource_borrow(
        main_core,
        dtor_core,
        imports,
        import_name,
        makes,
        arg_bytes,
        result_byte,
        plain,
        false,
    )
}

/// [`assemble_mixed_closure_resource`] with a `call_borrow` switch (C-HOST-6). When TRUE the ONE shared
/// `call` is typed `borrow<t>` (repeatable) on both the outer lift and the nested re-export re-typing;
/// `make`s + plain exports are unaffected. `false` reproduces the shipped own component byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn assemble_mixed_closure_resource_borrow(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
    plain: &[PlainExportAbi],
    call_borrow: bool,
) -> Vec<u8> {
    assemble_mixed_closure_resource_borrow_tuple(
        main_core,
        dtor_core,
        imports,
        import_name,
        makes,
        arg_bytes,
        result_byte,
        plain,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`assemble_mixed_closure_resource_borrow`] with an optional FIXED-SHAPE SCALAR tuple ARGUMENT for the
/// shared `call` (the multi-export/mixed direct-call compound-arg path). When `tuple_arg_bytes` is
/// `Some(field_bytes)`, the shared `call`'s single argument is a native component `tuple<field_bytes…>`
/// DEFINED type — minted just before the `call` functype (shifting the call functype + every plain functype
/// past it by 1) and inside the nested re-export — instead of `arg_bytes`'s inline scalar params. `None`
/// reproduces the scalar path byte-for-byte. Pairs with a shared-`call` core serialized with a matching
/// `TupleArgRebuild`. SCOPE mirrors the single-export tuple path: one fixed-shape scalar tuple/record arg,
/// scalar result.
#[allow(clippy::too_many_arguments)]
pub fn assemble_mixed_closure_resource_borrow_tuple(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
    plain: &[PlainExportAbi],
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    // Scalar boundary bytes BEFORE / AFTER the tuple arg (closure arg order) when the tuple sits AMONG scalar
    // args; both empty for a sole tuple. Only the SCALAR-result shared `call` interleaves them this increment.
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    // A NESTED fixed-shape compound tuple arg's recursive field shape (mints inner `tuple<…>` types by index).
    // `None` = an all-scalar tuple (the flat `tuple_arg_bytes` path). Only the multi-export path threads this.
    tuple_shape: Option<&[TupleFieldShape]>,
    // The N-COMPOUND-ARGS override (see [`assemble_closure_resource_borrow_tuple`]): `Some(slots)` mints one
    // `tuple<…>` group per tuple slot (in arg order) before the shared `call` functype, subsuming the
    // single-tuple inputs. `None` reproduces the single-tuple/scalar path byte-for-byte.
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    let k = imports.len();
    let nmk = makes.len();
    let np = plain.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: the import instance-type declaring the k used runtime ops (component type 0).
    let instance_type = runtime_op_instance_type(imports);
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
        // the shared call's own/borrow<t> + functype. `borrow<t>` → the handle survives across calls
        // (repeatable — C-HOST-6); `make`s stay `own<t>`. For a TUPLE arg, the `tuple<…>` defined type is
        // minted (comp type 3+2*nmk) between the handle and the call functype, shifting the call functype to
        // 4+2*nmk and each plain functype by +1.
        items.extend_from_slice(&if call_borrow {
            borrow_item(1)
        } else {
            own_item(1)
        });
        let call_own_ty = (2 + 2 * nmk) as u32;
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint each tuple slot's type(s) starting at 3+2*nmk (after the handle), in arg
            // order; the slot-model `call` functype references each by index.
            let mut next_type = 3 + 2 * nmk as u32;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_functype_slots(
                call_own_ty,
                slots,
                &tup_idxs,
                result_byte,
            ));
        } else if let Some(shape) = tuple_shape {
            let mut next_type = 3 + 2 * nmk as u32;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                call_own_ty,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                result_byte,
            ));
        } else if let Some(fields) = tuple_arg_bytes {
            let tup_ty = (3 + 2 * nmk) as u32;
            items.extend_from_slice(&tuple_defined_type(fields));
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                call_own_ty,
                tuple_prefix_bytes,
                tup_ty,
                tuple_suffix_bytes,
                result_byte,
            ));
        } else {
            items.extend_from_slice(&closure_call_functype(call_own_ty, arg_bytes, result_byte));
        }
        // each plain export's functype (scalar result, inline primitive byte).
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
        }
        // scalar call = own + functype (2); flat tuple = own + tuple + functype (3); nested = own +
        // nested-count tuple types + functype; N-compound = own + all tuple-slot types + functype.
        let n_call_types = if let Some(slots) = call_arg_slots {
            1 + call_arg_tuple_type_count(slots) as usize + 1
        } else if let Some(shape) = tuple_shape {
            1 + nested_tuple_type_count(shape) as usize + 1
        } else if tuple_arg_bytes.is_some() {
            3
        } else {
            2
        };
        section(
            sec::COMPONENT_TYPE,
            &wasm_vec(2 * nmk + n_call_types + np, &items),
        )
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
        // A TUPLE arg mints extra defined type(s) before the call functype, so the call functype (and every
        // plain functype after it) shifts: a FLAT tuple by +1, a NESTED tuple by `nested_tuple_type_count`.
        let tuple_shift: usize = if let Some(slots) = call_arg_slots {
            call_arg_tuple_type_count(slots) as usize
        } else if let Some(shape) = tuple_shape {
            nested_tuple_type_count(shape) as usize
        } else if tuple_arg_bytes.is_some() {
            1
        } else {
            0
        };
        let call_core_fn = (k + 3 + nmk) as u32;
        let call_functype = (3 + 2 * nmk + tuple_shift) as u32;
        items.extend_from_slice(&canon_lift_item(call_core_fn, call_functype));
        for j in 0..np {
            let core_fn = (k + 3 + nmk + 1 + j) as u32;
            let functype = (4 + 2 * nmk + tuple_shift + j) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        section(sec::CANON, &wasm_vec(nmk + 1 + np, &items))
    });
    // sec 4/5/11: nested re-export component; instantiate it (resource type 1 + comp funcs k..k+N makes,
    // k+N call) → component instance 1; export as the closure interface.
    out.extend_from_slice(&component_section(
        &resource_inner_component_multi_closure_borrow_tuple(
            makes,
            arg_bytes,
            result_byte,
            call_borrow,
            tuple_arg_bytes,
            tuple_prefix_bytes,
            tuple_suffix_bytes,
            tuple_shape,
            call_arg_slots,
        ),
    ));
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

/// Assemble a MULTI-EXPORT BYTE-ROPE-result closure component with a `call_borrow` switch (C-HOST-6, multi-export
/// `list<u8>`-result — byte-rope/compound/collection). When TRUE the shared `call`'s self is `borrow<t>`
/// (repeatable — each make's handle survives across calls) on the outer lift + the nested re-export;
/// `make`s + plain exports unaffected. `false` reproduces the shipped own component byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn assemble_multi_closure_bytes_resource_borrow(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    plain: &[PlainExportAbi],
    call_borrow: bool,
) -> Vec<u8> {
    assemble_multi_closure_bytes_resource_borrow_tuple(
        main_core,
        dtor_core,
        imports,
        import_name,
        makes,
        arg_bytes,
        plain,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`assemble_multi_closure_bytes_resource_borrow`] with an optional fixed-shape scalar tuple ARGUMENT: when
/// `tuple_arg_bytes` is `Some(field_bytes)`, the shared `call`'s single argument is a native `tuple<…>` minted
/// just before the `list<u8>` result type (shifting the `list<u8>` + call functype + every plain functype up
/// by 1) on both the outer lift + the nested re-export. `None` = the scalar-arg path (byte-identical). Pairs
/// with a multi list-result core serialized with a matching `TupleArgRebuild`.
#[allow(clippy::too_many_arguments)]
pub fn assemble_multi_closure_bytes_resource_borrow_tuple(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    plain: &[PlainExportAbi],
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    // A NESTED fixed-shape compound tuple arg's recursive field shape (mints inner `tuple<…>` types by index).
    // `None` = an all-scalar tuple (the flat `tuple_arg_bytes` path). Only the multi-export path threads this.
    tuple_shape: Option<&[TupleFieldShape]>,
    // The N-COMPOUND-ARGS override: `Some(slots)` mints one `tuple<…>` group per tuple slot (in arg order)
    // before the shared `list<u8>` result on each side. `None` reproduces the single-tuple/scalar path.
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    let k = imports.len();
    let nmk = makes.len();
    let np = plain.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // Shared prologue with the scalar multi envelope: import instance-type, runtime import, op alias/lower,
    // dtor, resource type, resource.new/rep, heap instance, program module/instance.
    let instance_type = runtime_op_instance_type(imports);
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
        // then each PLAIN export's body (core funcs after realloc; memory is not a func index).
        for p in plain {
            items.extend_from_slice(&core_alias_item(3, &p.core_name));
        }
        // N makes + `call` + `memory` + `cabi_realloc` + P plain.
        section(sec::ALIAS, &wasm_vec(nmk + 3 + np, &items))
    });
    // sec 7: per make `own<t>` (2+2i) + make functype (3+2i); then call `own<t>` (2+2N) + `list<u8>` (3+2N) +
    // call functype `(self: own<t>, args…) -> list<u8>` (4+2N); then one PLAIN functype per plain export
    // (a scalar `(params…) -> R`, comp type 5+2N+j).
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
        items.extend_from_slice(&if call_borrow {
            borrow_item(1)
        } else {
            own_item(1)
        });
        let call_own_ty = (2 + 2 * nmk) as u32;
        // For a TUPLE arg, mint `tuple<…>` (3+2N) before `list<u8>` + the call functype. A NESTED tuple mints
        // its inner types first (bottom-up), so the block adds `nested_tuple_type_count` tuple types + list +
        // functype; a flat tuple adds 1 + list + functype (3); a scalar arg list + functype (2).
        let n_call_types = if let Some(slots) = call_arg_slots {
            call_arg_tuple_type_count(slots) as usize + 2
        } else if let Some(shape) = tuple_shape {
            nested_tuple_type_count(shape) as usize + 2
        } else if tuple_arg_bytes.is_some() {
            3
        } else {
            2
        };
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint each tuple slot's type(s) starting at 3+2*nmk (in arg order), then
            // `list<u8>`, then the slot-model list-result functype.
            let mut next_type = 3 + 2 * nmk as u32;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            let list_ty = next_type;
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_functype_slots(
                call_own_ty,
                slots,
                &tup_idxs,
                list_ty,
            ));
        } else if let Some(shape) = tuple_shape {
            let mut next_type = 3 + 2 * nmk as u32;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            let list_ty = next_type; // the `list<u8>` result type sits right after the tuple type(s)
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                call_own_ty,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                list_ty,
            ));
        } else if let Some(fields) = tuple_arg_bytes {
            let tup_ty = (3 + 2 * nmk) as u32;
            let list_ty = (4 + 2 * nmk) as u32;
            items.extend_from_slice(&tuple_defined_type(fields));
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                call_own_ty,
                tuple_prefix_bytes,
                tup_ty,
                tuple_suffix_bytes,
                list_ty,
            ));
        } else {
            let list_ty = (3 + 2 * nmk) as u32;
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_functype(call_own_ty, arg_bytes, list_ty));
        }
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
        }
        section(
            sec::COMPONENT_TYPE,
            &wasm_vec(2 * nmk + 1 + n_call_types + np, &items),
        )
    });
    // sec 8: lift make[i] (core func k+3+i) against functype 3+2i → comp func k+i; lift `call` (core func
    // k+3+N) against functype 4+2N WITH Memory 0 + Realloc (core func k+4+N) → comp func k+N; lift each PLAIN
    // export (core func k+5+N+j) against its functype (comp type 5+2N+j) → comp func k+N+1+j.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..nmk {
            let core_fn = (k + 3 + i) as u32;
            let functype = (3 + 2 * i) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        // A TUPLE arg minted extra defined type(s) before the call functype, so the call functype (and every
        // plain functype after it) shifts: a FLAT tuple by +1, a NESTED tuple by its `nested_tuple_type_count`.
        let tuple_shift: usize = if let Some(slots) = call_arg_slots {
            call_arg_tuple_type_count(slots) as usize
        } else if let Some(shape) = tuple_shape {
            nested_tuple_type_count(shape) as usize
        } else if tuple_arg_bytes.is_some() {
            1
        } else {
            0
        };
        let call_core_fn = (k + 3 + nmk) as u32;
        let call_functype = (4 + 2 * nmk + tuple_shift) as u32;
        let realloc_fn = (k + 4 + nmk) as u32;
        items.extend_from_slice(&canon_lift_list_item(
            call_core_fn,
            0,
            realloc_fn,
            call_functype,
        ));
        for j in 0..np {
            let core_fn = (k + 5 + nmk + j) as u32;
            let functype = (5 + 2 * nmk + tuple_shift + j) as u32;
            items.extend_from_slice(&canon_lift_item(core_fn, functype));
        }
        section(sec::CANON, &wasm_vec(nmk + 1 + np, &items))
    });
    // sec 4/5/11: nested re-export component (list-result `call`); instantiate; export the closure interface,
    // then each PLAIN export as an ordinary top-level comp func (comp func k+nmk+1+j).
    out.extend_from_slice(&component_section(
        &resource_inner_component_multi_closure_bytes_borrow_tuple(
            makes,
            arg_bytes,
            call_borrow,
            tuple_arg_bytes,
            tuple_prefix_bytes,
            tuple_suffix_bytes,
            tuple_shape,
            call_arg_slots,
        ),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_INSTANCE,
        &wasm_vec(
            1,
            &component_instantiate_multi_call_item(1, k as u32, nmk, makes),
        ),
    ));
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

/// The MULTI-EXPORT inner re-export component for a BYTE-ROPE-result closure: like
/// [`resource_inner_component_multi_closure`] but the shared `call`'s result is `list<u8>` (each side mints
/// its own `list<u8>` defined type, shifting the export-side type base by 2 vs the scalar version — a make
/// contributes own<t>+ft = 2 types, the call contributes own<t>+list<u8>+ft = 3). Uses a running type
/// counter for clarity. Imported funcs: make[i] → func i, `call` → func N.
#[allow(dead_code)]
fn resource_inner_component_multi_closure_bytes(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
) -> Vec<u8> {
    resource_inner_component_multi_closure_bytes_borrow(makes, arg_bytes, false)
}

/// [`resource_inner_component_multi_closure_bytes`] with a `call_borrow` switch. The shared `call`'s self
/// handle (the imported `own<0>`/`borrow<0>` + the re-exported `own<R>`/`borrow<R>`) is `borrow<t>` when TRUE
/// — matching the outer lift in [`assemble_multi_closure_bytes_resource_borrow`]. `make`s stay `own<t>`.
fn resource_inner_component_multi_closure_bytes_borrow(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    call_borrow: bool,
) -> Vec<u8> {
    resource_inner_component_multi_closure_bytes_borrow_tuple(
        makes,
        arg_bytes,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`resource_inner_component_multi_closure_bytes_borrow`] with an optional fixed-shape scalar tuple ARGUMENT:
/// when `tuple_arg_bytes` is `Some`, the shared `call`'s single arg is a native `tuple<…>` minted before the
/// `list<u8>` result type on both the import and export sides — each `call` type block then holds handle +
/// tuple + list + functype (4 types, vs the scalar-arg handle + list + functype = 3). A running type counter
/// keeps both shapes consistent (the exported-resource index R absorbs the extra import-side type). `None` =
/// the scalar-arg path (byte-identical).
///
/// `call_arg_slots` is the N-COMPOUND-ARGS override: `Some(slots)` mints one `tuple<…>` group per tuple slot
/// (in arg order) before the shared `list<u8>` result on each side.
#[allow(clippy::too_many_arguments)]
fn resource_inner_component_multi_closure_bytes_borrow_tuple(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    tuple_shape: Option<&[TupleFieldShape]>,
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    let call_handle = |idx: u32| -> Vec<u8> {
        if call_borrow {
            borrow_item(idx)
        } else {
            own_item(idx)
        }
    };
    // Emit the shared `call` type block (self handle wrapping `resource_ty`, [tuple type(s)], list<u8>,
    // functype) at `block_base`. Returns the emitted items + the CALL FUNCTYPE index + how many types the block
    // added (3 scalar / 4 flat-tuple / 3 + nested-count for a NESTED tuple; N-compound = handle + all tuple
    // types + list + functype). Prefix/suffix scalar bytes surround the tuple when it sits AMONG scalar args.
    let call_type_block = |resource_ty: u32, block_base: u32| -> (Vec<u8>, u32, u32) {
        let handle_ty = block_base;
        let mut items = call_handle(resource_ty);
        if let Some(slots) = call_arg_slots {
            let mut next_type = block_base + 1;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            let list_ty = next_type;
            items.extend_from_slice(&list_u8_defined_type());
            next_type += 1;
            items.extend_from_slice(&closure_call_list_functype_slots(
                handle_ty, slots, &tup_idxs, list_ty,
            ));
            let added = 1 + call_arg_tuple_type_count(slots) + 2;
            (items, next_type, added)
        } else if let Some(shape) = tuple_shape {
            let mut next_type = block_base + 1;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            let list_ty = next_type;
            items.extend_from_slice(&list_u8_defined_type());
            next_type += 1;
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                handle_ty,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                list_ty,
            ));
            let added = 1 + nested_tuple_type_count(shape) + 2; // handle + tuple types + list + functype
            (items, next_type, added)
        } else if let Some(fields) = tuple_arg_bytes {
            let tup_ty = block_base + 1;
            let list_ty = block_base + 2;
            let ft_ty = block_base + 3;
            items.extend_from_slice(&tuple_defined_type(fields));
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                handle_ty,
                tuple_prefix_bytes,
                tup_ty,
                tuple_suffix_bytes,
                list_ty,
            ));
            (items, ft_ty, 4)
        } else {
            let list_ty = block_base + 1;
            let ft_ty = block_base + 2;
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_functype(handle_ty, arg_bytes, list_ty));
            (items, ft_ty, 3)
        }
    };
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
    // Shared call (import side): self handle wrapping the imported resource (type 0); import func N.
    {
        let (items, ft_ty, added) = call_type_block(0, ty);
        out.extend_from_slice(&section(
            sec::COMPONENT_TYPE,
            &wasm_vec(added as usize, &items),
        ));
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_func_item(&import_wire_name(n), ft_ty)),
        ));
        ty += added;
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
    // Shared call (export side): self handle wrapping the re-exported resource (type R); export `call`.
    {
        let (items, ft_ty, _added) = call_type_block(r, ty);
        out.extend_from_slice(&section(
            sec::COMPONENT_TYPE,
            &wasm_vec(_added as usize, &items),
        ));
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
    /// True when this group's `call-<g>` crosses the boundary as `list<u8>` rather than the inline scalar
    /// `result_byte` — i.e. its closure result is a byte-rope (`Bytes`/`String`, raw payload) OR a fixed-
    /// shape COMPOUND (tuple/record/sum, the canonical value form). The envelope treats both identically
    /// here (a `list<u8>` result lifted with the Memory/Realloc canon options); the core serializer decides
    /// what bytes fill it. When any group crosses as `list<u8>`, the whole distinct-sig component gains a
    /// shared memory + `cabi_realloc` alias.
    pub ret_is_bytes: bool,
    /// `Some(field_bytes)` when this group's closure takes a single FIXED-SHAPE SCALAR tuple/record ARGUMENT
    /// (the direct-call compound-arg path, SOLE or among scalar args): its `call-<g>` functype's tuple argument
    /// is a native component `tuple<field_bytes…>` (minted just before the functype on both import + export
    /// sides). `None` = the scalar-arg path (byte-identical). Composes with EVERY result shape.
    pub tuple_arg_bytes: Option<Vec<u8>>,
    /// The PREFIX scalar boundary bytes before the tuple (when it sits among scalar args), interleaved into the
    /// `call-<g>` functype. Empty for a SOLE tuple (or a scalar-arg group).
    pub tuple_prefix_bytes: Vec<u8>,
    /// The SUFFIX scalar boundary bytes after the tuple, interleaved into the `call-<g>` functype. Empty for a
    /// SOLE tuple (or a scalar-arg group).
    pub tuple_suffix_bytes: Vec<u8>,
    /// `Some(shape)` when this group's tuple arg is a NESTED fixed-shape compound: the per-group `call-<g>`
    /// mint sites emit the inner `tuple<…>` types by index from it (recursively). `None` = a flat all-scalar
    /// tuple (uses `tuple_arg_bytes`) or a scalar-arg group.
    pub tuple_shape: Option<Vec<TupleFieldShape>>,
    /// The N-COMPOUND-ARGS override for this group: `Some(slots)` when its closure takes ≥2 fixed-shape
    /// tuple/record args — the ordered slot list drives the per-group `call-<g>` functype's arg minting (one
    /// `tuple<…>` group per tuple slot, in arg order, interleaved with scalars), subsuming the single-tuple
    /// `tuple_arg_bytes`/prefix/suffix/`tuple_shape`. `None` reproduces the ≤1-tuple path byte-for-byte.
    pub call_arg_slots: Option<Vec<ArgSlot>>,
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
    assemble_distinct_sig_resource_mixed_borrow(
        main_core,
        dtor_core,
        imports,
        import_name,
        groups,
        plain,
        false,
    )
}

/// [`assemble_distinct_sig_resource_mixed`] with a `call_borrow` switch (C-HOST-6, distinct-sig per-group
/// `call-g<n>`). When TRUE each group's `call-g<n>` self handle is `borrow<t_g>` (repeatable) on the outer
/// lift + the nested re-export; `make`s stay `own<t_g>`. `false` reproduces the shipped own component
/// byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn assemble_distinct_sig_resource_mixed_borrow(
    main_core: &[u8],
    dtor_core: &[u8],
    imports: &[&RtOp],
    import_name: &str,
    groups: &[SigGroupAbi],
    plain: &[PlainExportAbi],
    call_borrow: bool,
) -> Vec<u8> {
    let call_handle = |idx: u32| -> Vec<u8> {
        if call_borrow {
            borrow_item(idx)
        } else {
            own_item(idx)
        }
    };
    let k = imports.len();
    let g = groups.len();
    let np = plain.len();
    // Flat function count across all groups: each group contributes (its makes) + 1 call.
    let total_fns: usize = groups.iter().map(|gr| gr.makes.len() + 1).sum();
    // A byte-rope group's `call-<g>` crosses as `list<u8>` → the component needs a shared memory +
    // `cabi_realloc` (aliased from the program instance once) and that group's call is lifted with the
    // Memory/Realloc canon options against a `(…) -> list<u8>` functype (own<t> + list<u8> + functype = 3
    // component types, vs a scalar call's own<t> + functype = 2). `n_bytes` counts those extra `list<u8>`
    // types; `any_bytes` gates the shared memory/realloc plumbing.
    let n_bytes = groups.iter().filter(|gr| gr.ret_is_bytes).count();
    let any_bytes = n_bytes > 0;
    // A tuple-ARG group's `call-<g>` argument is a native component `tuple<…>` (a DEFINED type minted just
    // before the call functype), so it also adds extra component type(s) — a FLAT tuple adds ONE (own<t> +
    // tuple + functype = 3, vs a scalar-arg call's own<t> + functype = 2); a NESTED tuple adds
    // `nested_tuple_type_count` (its inner tuples too). `n_tuple` sums those extras across groups.
    let n_tuple: usize = groups
        .iter()
        .map(|gr| {
            if let Some(slots) = &gr.call_arg_slots {
                call_arg_tuple_type_count(slots) as usize
            } else if let Some(shape) = &gr.tuple_shape {
                nested_tuple_type_count(shape) as usize
            } else if gr.tuple_arg_bytes.is_some() {
                1
            } else {
                0
            }
        })
        .sum();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: import instance-type (component type 0).
    let instance_type = runtime_op_instance_type(imports);
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
    // lowered ops + 3g resource funcs). Record each fn's core-func index in flat order, and whether the fn
    // is a byte-rope `call` (its lift needs Memory/Realloc). When any group is byte-rope, also alias the
    // shared `memory` (not a func index) + `cabi_realloc` AFTER the closure fns (mirrors the multi-export
    // bytes envelope), before the plain exports.
    let mut fn_core: Vec<u32> = Vec::new();
    let mut fn_is_bytes_call: Vec<bool> = Vec::new();
    let mut plain_core: Vec<u32> = Vec::new();
    let mut realloc_core: u32 = 0;
    let mut next_fn = (k + 3 * g) as u32;
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for (gi, gr) in groups.iter().enumerate() {
            for mk in &gr.makes {
                items.extend_from_slice(&core_alias_item(prog_inst, &mk.name));
                fn_core.push(next_fn);
                fn_is_bytes_call.push(false);
                next_fn += 1;
            }
            items.extend_from_slice(&core_alias_item(prog_inst, &format!("call-g{gi}")));
            fn_core.push(next_fn);
            fn_is_bytes_call.push(gr.ret_is_bytes);
            next_fn += 1;
        }
        if any_bytes {
            items.extend_from_slice(&memory_alias_item(prog_inst, MEMORY_EXPORT));
            items.extend_from_slice(&core_alias_item(prog_inst, REALLOC_EXPORT));
            realloc_core = next_fn;
            next_fn += 1;
        }
        // each PLAIN export's body, aliased AFTER the closure fns (+ memory/realloc when byte-rope).
        for p in plain {
            items.extend_from_slice(&core_alias_item(prog_inst, &p.core_name));
            plain_core.push(next_fn);
            next_fn += 1;
        }
        section(
            sec::ALIAS,
            &wasm_vec(total_fns + np + if any_bytes { 2 } else { 0 }, &items),
        )
    });
    // sec 7: per fn, its `own<t>` + functype. Component types after the import-instance-type (0) + G
    // resource types (1..1+g): the next defined type index is `1 + g`. A make/scalar-call adds own<t> (1) +
    // functype (1); a BYTE-ROPE call adds own<t> (1) + list<u8> (1) + `(…) -> list<u8>` functype (1). Record
    // each fn's functype component-type index.
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
            if gr.ret_is_bytes {
                // call-<g> returns `list<u8>`. With a TUPLE arg: own/borrow<t_g> (ti) + tuple type(s) +
                // list<u8> + `(self, tuple) -> list<u8>` functype. Without: own/borrow (ti) + list<u8> (ti+1) +
                // `(self, args…) -> list<u8>` functype (ti+2) — 3 types.
                items.extend_from_slice(&call_handle(rty));
                let own_ty = ti;
                if let Some(slots) = &gr.call_arg_slots {
                    // N-COMPOUND-ARGS list result: mint each tuple slot's type(s) after the handle (in arg
                    // order), then `list<u8>`, then the slot-model list functype.
                    let mut next = ti + 1;
                    let tup_idxs = mint_call_arg_tuple_types(slots, &mut next, &mut items);
                    let list_ty = next;
                    items.extend_from_slice(&list_u8_defined_type());
                    next += 1;
                    items.extend_from_slice(&closure_call_list_functype_slots(
                        own_ty, slots, &tup_idxs, list_ty,
                    ));
                    fn_functype.push(next);
                    ti = next + 1;
                } else if let Some(shape) = &gr.tuple_shape {
                    let mut next = ti + 1;
                    let outer_tup = mint_tuple_type_nested(shape, &mut next, &mut items);
                    let list_ty = next;
                    items.extend_from_slice(&list_u8_defined_type());
                    next += 1;
                    items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                        own_ty,
                        &gr.tuple_prefix_bytes,
                        outer_tup,
                        &gr.tuple_suffix_bytes,
                        list_ty,
                    ));
                    fn_functype.push(next);
                    ti = next + 1;
                } else if let Some(fields) = &gr.tuple_arg_bytes {
                    let tup_ty = ti + 1;
                    let list_ty = ti + 2;
                    items.extend_from_slice(&tuple_defined_type(fields));
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                        own_ty,
                        &gr.tuple_prefix_bytes,
                        tup_ty,
                        &gr.tuple_suffix_bytes,
                        list_ty,
                    ));
                    fn_functype.push(ti + 3);
                    ti += 4;
                } else {
                    let list_ty = ti + 1;
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&closure_call_list_functype(
                        own_ty,
                        &gr.arg_bytes,
                        list_ty,
                    ));
                    fn_functype.push(ti + 2);
                    ti += 3;
                }
            } else if let Some(slots) = &gr.call_arg_slots {
                // N-COMPOUND-ARGS scalar result: own/borrow<t_g> (ti) + N tuple type(s) + slot-model functype.
                items.extend_from_slice(&call_handle(rty));
                let own_ty = ti;
                let mut next = ti + 1;
                let tup_idxs = mint_call_arg_tuple_types(slots, &mut next, &mut items);
                items.extend_from_slice(&closure_call_functype_slots(
                    own_ty,
                    slots,
                    &tup_idxs,
                    gr.result_byte,
                ));
                fn_functype.push(next);
                ti = next + 1;
            } else if let Some(shape) = &gr.tuple_shape {
                // call-<g>: own/borrow<t_g> (ti) + nested tuple type(s) + `(self, tuple) -> R` functype.
                items.extend_from_slice(&call_handle(rty));
                let own_ty = ti;
                let mut next = ti + 1;
                let outer_tup = mint_tuple_type_nested(shape, &mut next, &mut items);
                items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                    own_ty,
                    &gr.tuple_prefix_bytes,
                    outer_tup,
                    &gr.tuple_suffix_bytes,
                    gr.result_byte,
                ));
                fn_functype.push(next);
                ti = next + 1;
            } else if let Some(fields) = &gr.tuple_arg_bytes {
                // call-<g>: own/borrow<t_g> (ti) + tuple<…> (ti+1) + `(self, <prefix…>, tuple, <suffix…>) -> R`.
                items.extend_from_slice(&call_handle(rty));
                let own_ty = ti;
                let tup_ty = ti + 1;
                items.extend_from_slice(&tuple_defined_type(fields));
                items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                    own_ty,
                    &gr.tuple_prefix_bytes,
                    tup_ty,
                    &gr.tuple_suffix_bytes,
                    gr.result_byte,
                ));
                fn_functype.push(ti + 2);
                ti += 3;
            } else {
                // call-<g>: own/borrow<t_g> + scalar call functype.
                items.extend_from_slice(&call_handle(rty));
                items.extend_from_slice(&closure_call_functype(ti, &gr.arg_bytes, gr.result_byte));
                fn_functype.push(ti + 1);
                ti += 2;
            }
        }
        // each PLAIN export's functype (scalar result, inline primitive byte — NO own<t> wrapper).
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
            plain_functype.push(ti);
            ti += 1;
        }
        section(
            sec::COMPONENT_TYPE,
            &wasm_vec(2 * total_fns + n_bytes + n_tuple + np, &items),
        )
    });
    // sec 8: lift each fn (its core func) against its functype → comp funcs k..k+total_fns; a byte-rope call
    // is lifted WITH Memory 0 + Realloc (`canon_lift_list_item`) so its `list<u8>` result crosses. Then lift
    // each PLAIN export (core func after the closure fns + memory/realloc) against its functype.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..total_fns {
            if fn_is_bytes_call[i] {
                items.extend_from_slice(&canon_lift_list_item(
                    fn_core[i],
                    0,
                    realloc_core,
                    fn_functype[i],
                ));
            } else {
                items.extend_from_slice(&canon_lift_item(fn_core[i], fn_functype[i]));
            }
        }
        for j in 0..np {
            items.extend_from_slice(&canon_lift_item(plain_core[j], plain_functype[j]));
        }
        section(sec::CANON, &wasm_vec(total_fns + np, &items))
    });
    // sec 4/5/11: nested re-export component; instantiate (G resources + total_fns comp funcs); export.
    out.extend_from_slice(&component_section(
        &resource_inner_component_distinct_sig_borrow(groups, call_borrow),
    ));
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
///
/// This is the P PLAIN (non-closure) exports variant, riding alongside the G
/// resource groups (the P=0 case is just `plain = &[]`): each plain body is
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
    // A byte-rope consumer crosses as `(…) -> list<u8>` (own<t> + list<u8> + functype = 3 comp types, vs a
    // scalar consumer's 2) and needs a shared memory + `cabi_realloc` (lifted with Memory/Realloc). `n_bytes`
    // counts them; `any_bytes` gates the shared plumbing.
    let n_bytes: usize = groups
        .iter()
        .map(|gr| gr.consumers.iter().filter(|c| c.ret_is_bytes).count())
        .sum();
    let any_bytes = n_bytes > 0;
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7/10/6/8: import instance-type, import runtime, alias+lower the ops (identical to distinct-sig).
    let instance_type = runtime_op_instance_type(imports);
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
    // core module's export order: per group, makes then consumers); then (when byte-rope) the shared
    // `memory` + `cabi_realloc`; then each PLAIN export's body. Track which flat fn is a byte-rope consumer.
    let mut fn_core: Vec<u32> = Vec::new();
    let mut fn_is_bytes: Vec<bool> = Vec::new();
    let mut plain_core: Vec<u32> = Vec::new();
    let mut realloc_core: u32 = 0;
    let mut next_fn = (k + 3 * g) as u32;
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for gr in groups.iter() {
            for mk in &gr.makes {
                items.extend_from_slice(&core_alias_item(prog_inst, &mk.name));
                fn_core.push(next_fn);
                fn_is_bytes.push(false);
                next_fn += 1;
            }
            for c in &gr.consumers {
                items.extend_from_slice(&core_alias_item(prog_inst, &c.name));
                fn_core.push(next_fn);
                fn_is_bytes.push(c.ret_is_bytes);
                next_fn += 1;
            }
        }
        if any_bytes {
            items.extend_from_slice(&memory_alias_item(prog_inst, MEMORY_EXPORT));
            items.extend_from_slice(&core_alias_item(prog_inst, REALLOC_EXPORT));
            realloc_core = next_fn;
            next_fn += 1;
        }
        // each PLAIN export's body, aliased AFTER the closure fns (+ memory/realloc when byte-rope).
        for p in plain {
            items.extend_from_slice(&core_alias_item(prog_inst, &p.core_name));
            plain_core.push(next_fn);
            next_fn += 1;
        }
        section(
            sec::ALIAS,
            &wasm_vec(total_fns + np + if any_bytes { 2 } else { 0 }, &items),
        )
    });
    // sec 7: per fn its `own<t_g>` + functype (make: `(params…)->own<t>`; scalar consumer: source-ordered
    // params → R; byte-rope consumer: own<t> + list<u8> + `(…)->list<u8>` = 3 types); then one PLAIN functype
    // per plain export (scalar `(params…)->R`, NO own<t> wrapper). Record each fn's functype comp-type index.
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
                if c.ret_is_bytes {
                    items.extend_from_slice(&own_item(rty));
                    let own_ty = ti;
                    let list_ty = ti + 1;
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&consumer_list_functype(own_ty, &c.params, list_ty));
                    fn_functype.push(ti + 2);
                    ti += 3;
                } else {
                    items.extend_from_slice(&own_item(rty));
                    items.extend_from_slice(&consumer_functype(ti, &c.params, c.result_byte));
                    fn_functype.push(ti + 1);
                    ti += 2;
                }
            }
        }
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
            plain_functype.push(ti);
            ti += 1;
        }
        section(
            sec::COMPONENT_TYPE,
            &wasm_vec(2 * total_fns + n_bytes + np, &items),
        )
    });
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..total_fns {
            if fn_is_bytes[i] {
                items.extend_from_slice(&canon_lift_list_item(
                    fn_core[i],
                    0,
                    realloc_core,
                    fn_functype[i],
                ));
            } else {
                items.extend_from_slice(&canon_lift_item(fn_core[i], fn_functype[i]));
            }
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
    /// True when the consumer's result crosses as `list<u8>` rather than the inline scalar `result_byte` —
    /// i.e. a byte-rope (`Bytes`/`String`, raw payload) OR a fixed-shape COMPOUND (tuple/record/sum, the
    /// canonical value form). The envelope treats both identically (a `list<u8>` result via the shared
    /// memory + `cabi_realloc`, lifted with Memory/Realloc; the inner re-export component types the consumer
    /// as `(…) -> list<u8>`); the core serializer decides what bytes fill it.
    pub ret_is_bytes: bool,
}

/// Assemble a ROUND-TRIP closure-resource component (C-HOST-4): N producer `make-<name>` functions PLUS M
/// CONSUMER functions, published together under `cadenza:closure/exports`, with P PLAIN (non-closure)
/// exports riding alongside the producers + consumers (the P=0 case is just `plain = &[]`). A producer mints
/// a closure handle (`() / (params…) -> own<t>`); a consumer takes a handle back (`(g: own<t>, args…) -> R`)
/// and applies it. Structurally the multi-export envelope with the shared `call` generalized to M named
/// consumers (each a `call`-shaped functype). `main_core` is
/// [`serialize::roundtrip_resource_core_module`]'s output (exporting each `make-<name>` + each consumer).
///
/// Outer index spaces (k = imports.len(), N = makes, M = consumers): lowered ops → core funcs 0..k;
/// `t-dtor` → k; `resource.new` → k+1, `resource.rep` → k+2; aliased make[i] → k+3+i, consumer[j] →
/// k+3+N+j. Component funcs: aliased ops 0..k, lifted make[i] → comp func k+i, consumer[j] → k+N+j.
/// Component types: 0 = import instance-type, 1 = resource; then per make: `own<t>` + make-functype; then
/// per consumer: `own<t>` + consume-functype.
///
/// Each plain body is aliased off the SAME program
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
    // A byte-rope consumer result crosses as `list<u8>` → the component needs a shared memory +
    // `cabi_realloc` (aliased from the program instance once) and that consumer is lifted with the
    // Memory/Realloc canon options against a `(…) -> list<u8>` functype (own<t> + list<u8> + functype = 3
    // component types, vs a scalar consumer's own<t> + functype = 2). `n_bytes` counts the extra `list<u8>`
    // types; `any_bytes` gates the shared memory/realloc plumbing.
    let n_bytes = consumers.iter().filter(|c| c.ret_is_bytes).count();
    let any_bytes = n_bytes > 0;
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);

    // sec 7: the import instance-type declaring the k used runtime ops (component type 0).
    let instance_type = runtime_op_instance_type(imports);
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
    // sec 6: alias each make + each consumer off the program instance → core funcs k+3..k+3+nfns; then
    // (when any byte-rope consumer) the shared `memory` + `cabi_realloc`; then each PLAIN export's body. A
    // byte-rope consumer's lift needs Memory/Realloc, so track its flat fn index + the realloc core func.
    // Flat fn order is makes (never byte-rope) then consumers (byte-rope iff their result is a byte-rope).
    let mut fn_is_bytes: Vec<bool> = vec![false; nmk];
    fn_is_bytes.extend(consumers.iter().map(|c| c.ret_is_bytes));
    let realloc_core = (k + 3 + nfns) as u32; // valid only when any_bytes
    let plain_core_base = (k + 3 + nfns) as u32 + if any_bytes { 1 } else { 0 };
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for mk in makes {
            items.extend_from_slice(&core_alias_item(3, &mk.name));
        }
        for c in consumers {
            items.extend_from_slice(&core_alias_item(3, &c.name));
        }
        if any_bytes {
            items.extend_from_slice(&memory_alias_item(3, MEMORY_EXPORT));
            items.extend_from_slice(&core_alias_item(3, REALLOC_EXPORT));
        }
        for p in plain {
            items.extend_from_slice(&core_alias_item(3, &p.core_name));
        }
        section(
            sec::ALIAS,
            &wasm_vec(nfns + np + if any_bytes { 2 } else { 0 }, &items),
        )
    });
    // sec 7: per make, `own<t>` + make functype; per SCALAR consumer, `own<t>` + consume functype (`(g:
    // own<t>, args…) -> R`); per BYTE-ROPE consumer, `own<t>` + `list<u8>` + `(…)->list<u8>` functype (3
    // comp types). Resource is comp type 1. Record each fn's functype comp-type index (byte-rope shifts it).
    let mut fn_functype: Vec<u32> = Vec::with_capacity(nfns);
    let mut plain_functype: Vec<u32> = Vec::with_capacity(np);
    out.extend_from_slice(&{
        let mut items = Vec::new();
        let mut ti = 2u32; // next defined-type index (type 0 = import inst, 1 = resource)
        for mk in makes {
            items.extend_from_slice(&own_item(1));
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(ti),
            ));
            fn_functype.push(ti + 1);
            ti += 2;
        }
        for c in consumers {
            if c.ret_is_bytes {
                items.extend_from_slice(&own_item(1));
                let own_ty = ti;
                let list_ty = ti + 1;
                items.extend_from_slice(&list_u8_defined_type());
                items.extend_from_slice(&consumer_list_functype(own_ty, &c.params, list_ty));
                fn_functype.push(ti + 2);
                ti += 3;
            } else {
                items.extend_from_slice(&own_item(1));
                items.extend_from_slice(&consumer_functype(ti, &c.params, c.result_byte));
                fn_functype.push(ti + 1);
                ti += 2;
            }
        }
        // each PLAIN export's functype (scalar result, inline primitive byte — NO own<t> wrapper).
        for p in plain {
            items.extend_from_slice(&params_result_functype(&p.param_bytes, &[p.result_byte]));
            plain_functype.push(ti);
            ti += 1;
        }
        section(
            sec::COMPONENT_TYPE,
            &wasm_vec(2 * nfns + n_bytes + np, &items),
        )
    });
    // sec 8: lift each make + each consumer against its functype → comp funcs k..k+nfns; a byte-rope consumer
    // is lifted WITH Memory 0 + Realloc. Then lift each PLAIN export against its functype → comp func
    // k+nfns+j.
    out.extend_from_slice(&{
        let mut items = Vec::new();
        for i in 0..nfns {
            let core_fn = (k + 3 + i) as u32;
            if fn_is_bytes[i] {
                items.extend_from_slice(&canon_lift_list_item(
                    core_fn,
                    0,
                    realloc_core,
                    fn_functype[i],
                ));
            } else {
                items.extend_from_slice(&canon_lift_item(core_fn, fn_functype[i]));
            }
        }
        for (j, &functype) in plain_functype.iter().enumerate() {
            let core_fn = plain_core_base + j as u32;
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

/// A resource-`make` component functype `(params…) -> result` over a per-parameter [`ArgSlot`] list: a
/// SCALAR slot is an inline primitive byte, a TUPLE slot references its minted `tuple<…>` type index (from
/// `tuple_type_idxs`, positionally). No `self` receiver (unlike the closure `call` slot functype). An
/// empty slot list is the nullary `() -> result`, byte-identical to [`params_result_functype`] over `&[]`.
fn make_functype_slots(
    slots: &[ArgSlot],
    tuple_type_idxs: &[Option<u32>],
    result_valtype: &[u8],
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (pn, (slot, tup_idx)) in slots.iter().zip(tuple_type_idxs).enumerate() {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        match (slot, tup_idx) {
            (ArgSlot::Scalar(vt), _) => param_items.push(*vt),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                Some(idx),
            ) => param_items.extend_from_slice(&owned_valtype(*idx)),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                None,
            ) => {
                unreachable!("a Tuple/Option make param must carry a minted defined-type index")
            }
        }
    }
    item.extend_from_slice(&wasm_vec(slots.len(), &param_items));
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

/// A round-trip CONSUMER's component functype whose RESULT is a byte-rope `list<u8>` (the compound-result
/// consumer). Identical param handling to [`consumer_functype`] — params in SOURCE ORDER, each an `own<t>`
/// closure handle or a scalar byte — but the single result is the `list<u8>` defined type at `list_type_idx`
/// (referenced by index) rather than an inline scalar primitive byte.
fn consumer_list_functype(own_ty: u32, params: &[ConsumeParamAbi], list_type_idx: u32) -> Vec<u8> {
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
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
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
    let name = crate::backend::common::export_name::kebab_extern_name(name);
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

/// The runtime-import INSTANCE TYPE (component type form `0x42`) for a set of runtime ops: per op, a `ty`
/// decl (`0x01` + the op's component functype) then an `export` decl (`0x04` + the op's extern name +
/// sort `0x01` + the op's component-func index `i`), all `2*k` decls wrapped as one instance type. This is
/// the shape every `assemble_*` variant open-codes to build the imported runtime interface's component
/// type 0 — factored here (the runtime-op twin of [`host_effect_instance_type`]) so the ~20 assemblers
/// share one source of truth for the import shape. Byte-identical to the inlined loop; callers that
/// COMPOSE it with prepended defined types (a host-fused list/option type) build their decls directly
/// rather than calling this.
fn runtime_op_instance_type(imports: &[&RtOp]) -> Vec<u8> {
    let mut decls = Vec::new();
    for (i, op) in imports.iter().enumerate() {
        decls.push(0x01); // ty decl
        decls.extend_from_slice(&op_comp_functype(op));
        decls.push(0x04); // export decl
        decls.extend_from_slice(&extern_name(op.name));
        decls.push(0x01); // sort: component func
        uleb128(i as u64, &mut decls);
    }
    let mut it = vec![0x42]; // instance type form
    it.extend_from_slice(&wasm_vec(2 * imports.len(), &decls));
    it
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
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# A trap MUST be an out-of-band halt the embedder observes when it invokes the entry — the wasm-level failure a partial operation or an aborting host function raises — rather than a variant the entry's result type declares, so that the internal trap mechanism (core-semantics.md §A Trap Halts Execution At A Defined Point) stays a run's terminal behavior and is not duplicated as a redundant arm of the interface.
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# The host MUST NOT require the component to encode any resume state, so that whichever resumption strategy a host chooses is invisible to the emitted component and constrained only by the run's determinism (capabilities-and-effects.md §A Run Is A Deterministic Function Of Its Input And Responses).
//= spec/capabilities/capabilities-and-effects.md#how-a-host-resumes-is-host-policy-not-language
//# The mechanism by which a host resolves a call it cannot answer immediately — suspending an in-memory fiber and resuming in place, or discarding the run and re-deriving it from the ordered responses it has recorded — MUST be host runtime policy the language neither prescribes nor represents, so that portable re-derivation and local fiber suspension are both admissible and the emitted component is identical under either.
//= spec/capabilities/capabilities-and-effects.md#how-a-host-resumes-is-host-policy-not-language
//# Because a host MAY choose to re-derive a run from its input and recorded responses, a program that is a deterministic function of those (the requirement above) MUST remain resumable under that strategy without carrying any resume state itself; but a host that instead suspends the run in place MAY hold the run's live state, so the language requires determinism rather than statelessness and leaves the choice to the host.
// The plain functype — no resume parameter, no suspension arm — is also how the seed mandates NOTHING
// about the host's resume mechanism: determinism is the only boundary requirement, and every faithful
// resolution strategy (inline, suspend-in-place, tear-down-and-replay) sees these same emitted bytes.
//= spec/capabilities/capabilities-and-effects.md#a-run-is-a-deterministic-function-of-its-input-and-responses
//# This determinism MUST be the language's only requirement on the host boundary: the language MUST NOT mandate how a host suspends, resumes, or resolves a call, so that a host is free to answer synchronously, to suspend and resume a run in place, or to tear a run down and re-derive it, and every faithful strategy produces identical observable behavior.
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

/// A sec-8 canon-LOWER item WITH both Memory + Realloc options — needed to lower a host import that RETURNS
/// a heap value (`option<list<u8>>`, kv.get): the adapter uses `realloc` to allocate the returned list in
/// guest memory `mem_idx`. A host op with no list result carries the realloc option unused (harmless). Both
/// the memory and the realloc core func are aliased BEFORE `lower_sec` (the shared mem module; realloc =
/// core func 0 in the bytes-provider), breaking the lower↔realloc circularity.
fn canon_lower_item_mem_realloc(comp_func: u32, mem_idx: u32, realloc_func: u32) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(comp_func as u64, &mut item);
    item.push(0x02); // canon options: count 2
    item.push(0x03); // CanonicalOption::Memory
    uleb128(mem_idx as u64, &mut item);
    item.push(0x04); // CanonicalOption::Realloc
    uleb128(realloc_func as u64, &mut item);
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

/// Build the HOST-effect import instance-type (`0x42 <decls>`) from `host_fns`. Each op is a `ty` decl
/// (`01 <comp_functype>`) + an `export` decl (`04 <name> 01 <func-type-index>`). When ANY op has a
/// `list<u8>` (Bytes) parameter (`has_list_param`), PREPEND a `(list u8)` defined type as instance-type
/// type index 0 — a Bytes param's `comp_functype` references it by index 0 — and the per-op func types
/// then occupy indices `1..=h`, so each export decl references `base + i` where `base = 1`. A pure
/// scalar/string set (no Bytes param) takes `base = 0`, no prepend, `2*h` decls — byte-identical to the
/// pre-Bytes shape. Shared by every host-import assembly variant so the prepend/shift is defined once.
fn host_effect_instance_type(
    host_fns: &[HostFn],
    needs_list: bool,
    // Each spilled-RESULT defined type + whether it is NOMINAL (a `variant`/`enum`/`record` an import func's
    // result references, which MUST be exported — like a record param — else "instance not valid as import").
    result_defs: &[(Vec<u8>, bool)],
    record_defs: &[Vec<u8>],
) -> Vec<u8> {
    let h = host_fns.len();
    let mut decls = Vec::new();
    let mut prepended: u64 = 0;
    // idx 0: the shared `(list u8)` defined type — referenced by every `list<u8>` PARAM and every `list<u8>`
    // leaf of a spilled result. `needs_list` is computed by the caller (`build_host_result_types`) over both
    // params and results (every admitted spilled result bottoms out at `list<u8>`, so it forces this type).
    if needs_list {
        decls.push(0x01);
        decls.extend_from_slice(&list_u8_defined_type()); // type index 0
        prepended += 1;
    }
    // The spilled-RESULT defined types, built GENERALLY (Ty → WitType → CDef via `wit_ctype`) by the caller
    // and emitted here at instance-type indices 1.. (right after `(list u8)`), children-first + deduped. Each
    // op's `comp_functype` references its own result type by the `CRef` index the caller computed. This ONE
    // list REPLACES the former per-shape option / tuple / list<tuple> blocks; `list<list<u8>>`
    // (graph.neighbors) is just another entry here, no new branch.
    for (i, (rd, is_nominal)) in result_defs.iter().enumerate() {
        let defined_idx = prepended;
        decls.push(0x01);
        decls.extend_from_slice(rd);
        prepended += 1;
        // A NOMINAL result-def (`variant`/`enum`/`record` — e.g. run.run's `result<list<u8>, enum>` err arm)
        // must be EXPORTED (component-model rule, like a record param); a structural `list`/`option`/`result`/
        // `tuple` stays an anonymous bare define. The caller (`build_host_result_types`) already remapped every
        // reference to the EXPORT index (`defined_idx + 1`) via its export-aware indexing.
        if *is_nominal {
            decls.push(0x04); // export decl
            decls.extend_from_slice(&extern_name(&format!("host-result-t{i}")));
            decls.push(0x03); // externdesc: type
            decls.push(0x00); // typebound: eq
            uleb128(defined_idx, &mut decls);
            prepended += 1;
        }
    }
    // RECORD-param types (shape d): a NOMINAL type (record) that a func in an IMPORT instance-type uses
    // must be EXPORTED from the instance (a component-model rule — a structural `list<u8>` may be anonymous,
    // a record may NOT; verified against `wasm-tools component wit`'s own encoding of a record-param import).
    // So per record param lay TWO decls: (1) DEFINE the record (`0x01 <record-bytes>`, type index = current)
    // then (2) EXPORT it as a named type (`0x04 <name> 0x03 0x00 <defined-idx>` — export decl, a `type`
    // externdesc with an `eq` bound to the defined index), which introduces the EXPORTED type at the NEXT
    // index. The op's `comp_functype` references that EXPORTED index (defined+1), NOT the raw defined index.
    // In the supported shape (a single record param + NO Bytes/option/pairs op — enforced by the caller) no
    // shared type is prepended, so the record is DEFINED at index 0 and EXPORTED at index 1 (the index the
    // Record arm of `host_op_comp_functype` references), and the func types follow at index 2+.
    for (i, rd) in record_defs.iter().enumerate() {
        let defined_idx = prepended;
        decls.push(0x01);
        decls.extend_from_slice(rd);
        prepended += 1;
        decls.push(0x04); // export decl
        decls.extend_from_slice(&extern_name(&format!("host-record-p{i}")));
        decls.push(0x03); // externdesc: type
        decls.push(0x00); // typebound: eq
        uleb128(defined_idx, &mut decls);
        prepended += 1;
    }
    let base: u64 = prepended;
    for (i, f) in host_fns.iter().enumerate() {
        decls.push(0x01);
        decls.extend_from_slice(&f.comp_functype);
        decls.push(0x04);
        decls.extend_from_slice(&extern_name(
            &crate::backend::common::export_name::kebab_extern_name(&f.op),
        ));
        decls.push(0x01); // sort: component func
        uleb128(base + i as u64, &mut decls);
    }
    let decl_count = prepended as usize + 2 * h;
    let mut it = vec![0x42]; // instance type form
    it.extend_from_slice(&wasm_vec(decl_count, &decls));
    it
}

/// The sec-7 defined-type item for a component `tuple<vt0, vt1, …>`: `6f <count> <vt>*` — the component-
/// model `tuple` defined-type tag `0x6f`, then the field-count vec of primitive valtype bytes. A FIXED-SHAPE
/// SCALAR tuple closure argument crosses the DIRECT-CALL boundary as this native type; the canonical ABI
/// FLATTENS it (≤16 scalar fields) into scalar core params, which the guest `call` rebuilds into a cell
/// (`serialize::TupleArgRebuild`). The `0x6f` tuple tag is a component-model structural encoding
/// `wasm-encoder` writes via `ComponentDefinedType::tuple`; the `a_fixed_shape_tuple_closure_arg_crosses_by_
/// native_flattening` oracle pins that a `tuple<s64,s64>` param lifts + runs (matching `type_defined().tuple`).
fn tuple_defined_type(field_bytes: &[u8]) -> Vec<u8> {
    let mut item = vec![0x6f];
    item.extend_from_slice(&wasm_vec(field_bytes.len(), field_bytes));
    item
}

/// A component `option<T>` DEFINED TYPE: `0x6b <T-valtype>` — the `option` former tag then the payload's
/// primitive valtype byte. The direct-call SUM-arg path: an `(Option scalar)` closure argument crosses as
/// this native type, which the canonical ABI FLATTENS into `(disc: i32, payload: <T>)` core params — the
/// guest `call` rebuilds the sum cell from them (`serialize::SumArgRebuild`). Pinned runnable by the
/// `an_option_scalar_closure_arg_crosses_by_native_flattening` oracle (`wasm_encoder`'s `.option(...)`).
fn option_defined_type(payload_byte: u8) -> Vec<u8> {
    vec![0x6b, payload_byte]
}

/// A component `result<ok, err>` DEFINED TYPE: `0x6a <ok-valtype-opt> <err-valtype-opt>` — the `result`
/// former tag then each side's OPTIONAL valtype (`0x01 <byte>` for a present scalar payload, `0x00` for a
/// nullary side). A general `variant` must be NAMED, but `result`/`option` are anonymous-allowed, so a
/// `(Result scalar scalar)` arg crosses as this native type. Flattened by the canonical ABI to `(disc: i32,
/// payload)` (Ok=0, Err=1). Pinned runnable by the `a_result_scalar_closure_arg_crosses_by_native_flattening`
/// oracle (`wasm_encoder`'s `.result(Some, Some)`).
fn result_defined_type(ok_byte: u8, err_byte: u8) -> Vec<u8> {
    // `0x01 <byte>` is the `Some(primitive)` valtype encoding (an inline primitive, not a type-index ref).
    vec![0x6a, 0x01, ok_byte, 0x01, err_byte]
}

/// The boundary component-TYPE shape of a fixed-shape compound closure argument, recursively: each field is
/// either a PRIMITIVE valtype byte (an aliased-width scalar leaf) or a NESTED tuple (its own field shapes).
/// A `Scalar` field is one flattened core param; a `Nested` field is its own `tuple<…>` DEFINED type the
/// canonical ABI flattens recursively. `TupleFieldShape::Scalar`-only tuples reduce to the flat
/// `tuple_defined_type(field_bytes)` case; a `Nested` field forces the recursive minting below.
#[derive(Clone)]
pub enum TupleFieldShape {
    /// A scalar leaf field carrying its component primitive valtype byte (`COMP_S64`, …).
    Scalar(u8),
    /// A nested fixed-shape tuple/record field, its own fields in cell order.
    Nested(Vec<TupleFieldShape>),
}

/// Mint the `tuple<…>` DEFINED TYPE for a (possibly NESTED) fixed-shape compound argument into `items`,
/// emitting every INNER nested tuple type FIRST (bottom-up) so the outer tuple can reference them by index.
/// `next_type` is the component-type index the FIRST minted type will occupy; it is advanced past every type
/// this mints. Returns the type index of the OUTERMOST tuple (the one a `call` functype references as the
/// argument). A field shape of all `Scalar`s mints exactly ONE type (byte-identical to `tuple_defined_type`);
/// each `Nested` field mints its own sub-tuple first (recursively). Used by the single-export scalar-result
/// `call` path; other paths (flat `tuple_defined_type`) still assume all-scalar fields.
fn mint_tuple_type_nested(
    fields: &[TupleFieldShape],
    next_type: &mut u32,
    items: &mut Vec<u8>,
) -> u32 {
    // Mint each nested field's sub-tuple first, recording the valtype byte(s) each field contributes to the
    // outer tuple (a scalar → its primitive byte; a nested → an sleb128 type-index reference).
    let mut field_valtypes: Vec<Vec<u8>> = Vec::with_capacity(fields.len());
    for f in fields {
        match f {
            TupleFieldShape::Scalar(b) => field_valtypes.push(vec![*b]),
            TupleFieldShape::Nested(sub) => {
                let sub_idx = mint_tuple_type_nested(sub, next_type, items);
                // A defined-type reference in a `tuple` field is the type index as a SIGNED LEB128 (matching
                // `wasm_encoder`'s `ComponentValType::Type(i) => (i as i64).encode`); a small positive index
                // is one byte ≥ 0 and < 0x64, distinct from the primitive-byte range (0x64..=0x7f).
                let mut enc = Vec::new();
                crate::backend::wasm::encode::sleb128(sub_idx as i64, &mut enc);
                field_valtypes.push(enc);
            }
        }
    }
    // Now emit the OUTER tuple type: `0x6f <count> <field-valtype-encoding>*`.
    let mut tup = vec![0x6f];
    let mut body = Vec::new();
    for fv in &field_valtypes {
        body.extend_from_slice(fv);
    }
    tup.extend_from_slice(&wasm_vec(field_valtypes.len(), &body));
    items.extend_from_slice(&tup);
    let outer_idx = *next_type;
    *next_type += 1;
    outer_idx
}

/// The number of component TYPES [`mint_tuple_type_nested`] emits for `fields`: 1 for the outer tuple + the
/// recursive count for every nested field. A flat all-scalar shape is 1 (byte-identical to the flat path).
fn nested_tuple_type_count(fields: &[TupleFieldShape]) -> u32 {
    1 + fields
        .iter()
        .map(|f| match f {
            TupleFieldShape::Scalar(_) => 0,
            TupleFieldShape::Nested(sub) => nested_tuple_type_count(sub),
        })
        .sum::<u32>()
}

/// ONE `call`-argument slot in the closure's original arg order: either an aliased-width SCALAR (crossing as
/// its component primitive valtype byte) or a fixed-shape TUPLE/record (crossing as a native `tuple<…>` the
/// canonical ABI flattens — possibly nested, so its own `TupleFieldShape` tree). This is the N-arg
/// generalization of the single-tuple `(prefix_bytes, tuple_shape, suffix_bytes)` interleave: a slot list with
/// exactly ONE `Tuple` and the rest `Scalar` reproduces that shape byte-for-byte, and TWO+ `Tuple` slots are
/// the N-compound-args case (each tuple mints its own `tuple<…>` defined type, referenced by index in order).
#[derive(Clone)]
pub enum ArgSlot {
    /// A scalar leaf arg carrying its component primitive valtype byte.
    Scalar(u8),
    /// A fixed-shape tuple/record arg, its (possibly nested) field shape.
    Tuple(Vec<TupleFieldShape>),
    /// An `(Option scalar)` arg carrying its payload's component primitive valtype byte — crosses as a native
    /// `option<payload>` DEFINED type (minted by [`mint_call_arg_tuple_types`], flattened by the canonical ABI
    /// to `(disc: i32, payload)`; the guest rebuilds the sum cell via `serialize::SumArgRebuild`).
    OptionScalar(u8),
    /// A `(Result ok-scalar err-scalar)` arg carrying its ok + err payload component primitive valtype bytes —
    /// crosses as a native `result<ok, err>` DEFINED type (the `0x6a` former; anonymous-allowed, unlike a
    /// general `variant`). Flattened by the canonical ABI to `(disc: i32, payload)`; the guest rebuilds the
    /// sum cell via `serialize::SumArgRebuild`.
    Result(u8, u8),
    /// An `(Option compound)` arg whose payload is a fixed-shape TUPLE/record — crosses as a native
    /// `option<tuple<…>>` DEFINED type (the inner `tuple<…>` minted first — possibly nested — then `option`
    /// referencing it; both formers anonymous-allowed, unlike a general `variant`). Flattened by the canonical
    /// ABI to `(disc: i32, <payload tuple's leaves…>)`; the guest rebuilds the payload cell + `sum-new`s the
    /// Some over it via `serialize::SumArgRebuild` (a `SumArmPayload::Compound` arm). Carries the payload's
    /// (possibly nested) `TupleFieldShape` tree, exactly like [`ArgSlot::Tuple`].
    OptionCompound(Vec<TupleFieldShape>),
    /// A `(Result ok err)` arg where AT LEAST ONE side's payload is a fixed-shape TUPLE/record (a compound) —
    /// crosses as a native `result<ok, err>` DEFINED type whose ok/err valtypes are each a primitive byte
    /// (scalar side) OR a minted `tuple<…>` (compound side). The canonical ABI flattens it to `(disc: i32,
    /// <joined payload leaves…>)`, the two arms' payloads joined position-by-position (the wider arm sets each
    /// slot's width). The guest rebuilds the selected arm's cell over a PREFIX of the joined slots via
    /// `serialize::SumArgRebuild`. Each side carries its [`ResultSide`] (scalar byte or tuple shape).
    ResultCompound(ResultSide, ResultSide),
}

/// One side (ok or err) of a [`ArgSlot::ResultCompound`]: a scalar leaf (its component primitive byte) OR a
/// fixed-shape TUPLE/record payload (its own, possibly nested, `TupleFieldShape` tree — minted as a `tuple<…>`
/// the `result` former references by index). A nullary side (`Result … ()`) is not modeled here (both sides of
/// a Cadenza `(Result a b)` carry a payload).
#[derive(Clone)]
pub enum ResultSide {
    Scalar(u8),
    Compound(Vec<TupleFieldShape>),
}

/// The number of component TYPES [`mint_call_arg_tuple_types`] emits for `slots`: the sum of
/// [`nested_tuple_type_count`] over every `Tuple` slot, plus ONE `option<…>` type per `OptionScalar` slot (a
/// `Scalar` slot mints none). Zero when every slot is a plain scalar (byte-identical to the all-scalar path).
fn call_arg_tuple_type_count(slots: &[ArgSlot]) -> u32 {
    slots
        .iter()
        .map(|s| match s {
            ArgSlot::Scalar(_) => 0,
            ArgSlot::Tuple(shape) => nested_tuple_type_count(shape),
            ArgSlot::OptionScalar(_) | ArgSlot::Result(_, _) => 1,
            // the payload tuple's types (possibly nested) + the outer `option<…>` referencing it.
            ArgSlot::OptionCompound(shape) => nested_tuple_type_count(shape) + 1,
            // each compound side's tuple types + the outer `result<…>` referencing them (a scalar side = 0).
            ArgSlot::ResultCompound(ok, err) => {
                result_side_type_count(ok) + result_side_type_count(err) + 1
            }
        })
        .sum()
}

/// The number of component types a [`ResultSide`] mints: 0 for a scalar (an inline primitive byte), the nested
/// tuple type count for a compound.
fn result_side_type_count(side: &ResultSide) -> u32 {
    match side {
        ResultSide::Scalar(_) => 0,
        ResultSide::Compound(shape) => nested_tuple_type_count(shape),
    }
}

/// Mint the aggregate DEFINED TYPES for every non-scalar slot into `items`, in arg order, advancing
/// `next_type` past each. Returns, per slot, `Some(defined_type_idx)` for a `Tuple`/`OptionScalar` slot (the
/// `tuple<…>`/`option<…>` type the `call` functype references by index) and `None` for a `Scalar` slot. A
/// single `Tuple` slot mints byte-identically to `mint_tuple_type_nested`; an `OptionScalar` mints one
/// `option<payload>`.
fn mint_call_arg_tuple_types(
    slots: &[ArgSlot],
    next_type: &mut u32,
    items: &mut Vec<u8>,
) -> Vec<Option<u32>> {
    slots
        .iter()
        .map(|s| match s {
            ArgSlot::Scalar(_) => None,
            ArgSlot::Tuple(shape) => Some(mint_tuple_type_nested(shape, next_type, items)),
            ArgSlot::OptionScalar(payload_byte) => {
                items.extend_from_slice(&option_defined_type(*payload_byte));
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
            ArgSlot::Result(ok_byte, err_byte) => {
                items.extend_from_slice(&result_defined_type(*ok_byte, *err_byte));
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
            ArgSlot::OptionCompound(shape) => {
                // Mint the payload `tuple<…>` (possibly nested) FIRST, then the `option<…>` referencing it by
                // index. The `option` former is `0x6b <valtype>`; a defined-type reference is the type index as
                // a SIGNED LEB128 (matching `ComponentValType::Type(i) => (i as i64).encode`).
                let tup_idx = mint_tuple_type_nested(shape, next_type, items);
                let mut opt = vec![0x6b];
                crate::backend::wasm::encode::sleb128(tup_idx as i64, &mut opt);
                items.extend_from_slice(&opt);
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
            ArgSlot::ResultCompound(ok, err) => {
                // Mint each COMPOUND side's `tuple<…>` FIRST (in order ok, err), then the `result<…>` (former
                // `0x6a <ok-valtype-opt> <err-valtype-opt>`, each `0x01 <valtype>`) referencing them. A scalar
                // side's valtype is its inline primitive byte; a compound side's is its minted tuple index
                // (SIGNED LEB128). `mint_result_side_valtype` mints the tuple (if any) + returns the encoding.
                let ok_vt = mint_result_side_valtype(ok, next_type, items);
                let err_vt = mint_result_side_valtype(err, next_type, items);
                let mut res = vec![0x6a, 0x01];
                res.extend_from_slice(&ok_vt);
                res.push(0x01);
                res.extend_from_slice(&err_vt);
                items.extend_from_slice(&res);
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
        })
        .collect()
}

/// Mint a [`ResultSide`]'s tuple type (if compound) into `items`, advancing `next_type`, and return the side's
/// component valtype ENCODING for the enclosing `result<…>` former: an inline primitive byte for a scalar, or
/// the minted tuple type index as a SIGNED LEB128 for a compound.
fn mint_result_side_valtype(
    side: &ResultSide,
    next_type: &mut u32,
    items: &mut Vec<u8>,
) -> Vec<u8> {
    match side {
        ResultSide::Scalar(byte) => vec![*byte],
        ResultSide::Compound(shape) => {
            let tup_idx = mint_tuple_type_nested(shape, next_type, items);
            let mut enc = Vec::new();
            crate::backend::wasm::encode::sleb128(tup_idx as i64, &mut enc);
            enc
        }
    }
}

/// A `call` functype for a closure whose args are the given ordered `slots` (scalars + fixed-shape tuples
/// interleaved): `(self: <handle<t>>, p0: <slot0>, …) -> R`. `tuple_type_idxs[i]` is `Some(idx)` for a `Tuple`
/// slot (the `tuple<…>` defined type minted by [`mint_call_arg_tuple_types`], referenced by index) and `None`
/// for a `Scalar` slot (its primitive byte is taken from the slot). This is the N-tuple generalization of
/// [`closure_call_tuple_arg_functype_interleaved`]: a slot list of `[Scalar…, Tuple, Scalar…]` produces the
/// exact same bytes (one tuple among scalars); TWO+ `Tuple` slots interleave their type-index references.
fn closure_call_functype_slots(
    self_handle_type_idx: u32,
    slots: &[ArgSlot],
    tuple_type_idxs: &[Option<u32>],
    result_byte: u8,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    // `self` — the receiver handle (own/borrow<t>), a defined type referenced by index.
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    for (pn, (slot, tup_idx)) in slots.iter().zip(tuple_type_idxs).enumerate() {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        match (slot, tup_idx) {
            (ArgSlot::Scalar(vt), _) => param_items.push(*vt),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                Some(idx),
            ) => param_items.extend_from_slice(&owned_valtype(*idx)),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                None,
            ) => {
                unreachable!("a Tuple/Option slot must carry a minted defined-type index")
            }
        }
    }
    item.extend_from_slice(&wasm_vec(1 + slots.len(), &param_items));
    // One result — the closure's return valtype (a scalar boundary byte).
    item.extend_from_slice(&[0x00, result_byte]);
    item
}

/// The `list<u8>`-result counterpart of [`closure_call_functype_slots`]: `(self: <handle<t>>, p0: <slot0>, …)
/// -> list<u8>`. The param list is identical (scalars + fixed-shape tuples interleaved by the `ArgSlot`
/// model); only the result references the `list<u8>` DEFINED type by index instead of an inline scalar byte.
/// Its lift carries Memory/Realloc (the caller uses `canon_lift_list_item`). The N-tuple generalization of
/// [`closure_call_list_tuple_arg_functype_interleaved`].
fn closure_call_list_functype_slots(
    self_handle_type_idx: u32,
    slots: &[ArgSlot],
    tuple_type_idxs: &[Option<u32>],
    list_type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    for (pn, (slot, tup_idx)) in slots.iter().zip(tuple_type_idxs).enumerate() {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        match (slot, tup_idx) {
            (ArgSlot::Scalar(vt), _) => param_items.push(*vt),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                Some(idx),
            ) => param_items.extend_from_slice(&owned_valtype(*idx)),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                None,
            ) => {
                unreachable!("a Tuple/Option slot must carry a minted defined-type index")
            }
        }
    }
    item.extend_from_slice(&wasm_vec(1 + slots.len(), &param_items));
    // One result — the `list<u8>` defined type, referenced by index.
    item.push(0x00);
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A `call` functype for a closure taking ONE fixed-shape scalar tuple arg AMONG scalar args: `(self:
/// <handle<t>>, <prefix scalars…>, p: tuple<…>, <suffix scalars…>) -> R`. `prefix_bytes`/`suffix_bytes` are
/// the scalar boundary bytes BEFORE/AFTER the tuple (in the closure's original arg order); `tuple_type_idx`
/// the `tuple<…>` defined type. The scalar-result compound-arg path with the tuple at any position.
fn closure_call_tuple_arg_functype_interleaved(
    self_handle_type_idx: u32,
    prefix_bytes: &[u8],
    tuple_type_idx: u32,
    suffix_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    // `self` — the receiver handle (own/borrow<t>), a defined type referenced by index.
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    let mut pn = 0usize; // positional param name counter (cosmetic)
    for &vt in prefix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    // the tuple argument, a defined type referenced by index.
    {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.extend_from_slice(&owned_valtype(tuple_type_idx));
        pn += 1;
    }
    for &vt in suffix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    item.extend_from_slice(&wasm_vec(
        1 + prefix_bytes.len() + 1 + suffix_bytes.len(),
        &param_items,
    ));
    // One result — the closure's return valtype (a scalar boundary byte).
    item.extend_from_slice(&[0x00, result_byte]);
    item
}

/// A `call` functype for a closure taking ONE fixed-shape scalar tuple arg AMONG scalar args AND returning a
/// `list<u8>` (byte-rope / compound / collection result): `(self: <handle<t>>, <prefix scalars…>, p:
/// tuple<…>, <suffix scalars…>) -> list<u8>`. Combines the interleaved-arg shape of
/// [`closure_call_tuple_arg_functype_interleaved`] with the `list<u8>` result. Its lift carries Memory/Realloc.
fn closure_call_list_tuple_arg_functype_interleaved(
    self_handle_type_idx: u32,
    prefix_bytes: &[u8],
    tuple_type_idx: u32,
    suffix_bytes: &[u8],
    list_type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    let mut pn = 0usize;
    for &vt in prefix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.extend_from_slice(&owned_valtype(tuple_type_idx));
        pn += 1;
    }
    for &vt in suffix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    item.extend_from_slice(&wasm_vec(
        1 + prefix_bytes.len() + 1 + suffix_bytes.len(),
        &param_items,
    ));
    // One result — the `list<u8>` defined type, referenced by index.
    item.push(0x00);
    uleb128(list_type_idx as u64, &mut item);
    item
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
    let extern_name = crate::backend::common::export_name::kebab_extern_name(name);
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

    /// N-compound-args (byte-neutral): the `ArgSlot` model reproduces the single-tuple-among-scalars
    /// interleaved `call` functype BYTE-FOR-BYTE, and extends cleanly to TWO tuple args. Pins that
    /// `mint_call_arg_tuple_types` + `closure_call_functype_slots` are a pure generalization of
    /// `mint_tuple_type_nested` + `closure_call_tuple_arg_functype_interleaved` before they are wired into the
    /// assembled component (the same de-risking `closure_call_functype_encodes_the_call_method_shape` gave the
    /// scalar `call`).
    #[test]
    fn arg_slots_reproduce_the_single_tuple_interleave_and_extend_to_n_tuples() {
        use crate::backend::wasm::runtime_abi::AbiValType;
        let s64 = AbiValType::S64.comp_byte(); // 0x78

        // (a) ONE flat `tuple<s64,s64>` among a leading + trailing scalar, self handle = type 4, tuple type
        //     minted at index 5. The slot model must match the interleaved builder exactly.
        let shape = vec![TupleFieldShape::Scalar(s64), TupleFieldShape::Scalar(s64)];
        let slots = vec![
            ArgSlot::Scalar(s64),
            ArgSlot::Tuple(shape.clone()),
            ArgSlot::Scalar(s64),
        ];
        let mut items_ref = Vec::new();
        let mut next_ref = 5u32;
        let outer = mint_tuple_type_nested(&shape, &mut next_ref, &mut items_ref);
        let want_ft = closure_call_tuple_arg_functype_interleaved(4, &[s64], outer, &[s64], s64);

        let mut items_slots = Vec::new();
        let mut next_slots = 5u32;
        let tup_idxs = mint_call_arg_tuple_types(&slots, &mut next_slots, &mut items_slots);
        let got_ft = closure_call_functype_slots(4, &slots, &tup_idxs, s64);
        assert_eq!(items_ref, items_slots, "one-tuple mint bytes match");
        assert_eq!(next_ref, next_slots, "one-tuple type counter matches");
        assert_eq!(got_ft, want_ft, "one-tuple-among-scalars functype matches");
        assert_eq!(
            call_arg_tuple_type_count(&slots),
            nested_tuple_type_count(&shape),
            "one-tuple type count matches"
        );

        // (b) TWO flat `tuple<s64,s64>` args (the N-compound-args case): each mints its own tuple type
        //     (indices 5, 6), and the functype references them positionally as p0, p1.
        let two = vec![ArgSlot::Tuple(shape.clone()), ArgSlot::Tuple(shape.clone())];
        let mut items2 = Vec::new();
        let mut next2 = 5u32;
        let idxs2 = mint_call_arg_tuple_types(&two, &mut next2, &mut items2);
        assert_eq!(idxs2, vec![Some(5), Some(6)], "two tuples minted at 5,6");
        assert_eq!(next2, 7, "counter advanced past both tuple types");
        assert_eq!(
            call_arg_tuple_type_count(&two),
            2,
            "two flat tuples = 2 types"
        );
        let ft2 = closure_call_functype_slots(4, &two, &idxs2, s64);
        let want2: Vec<u8> = vec![
            wasm_abi::COMP_FUNCTYPE_FORM,
            0x03, // self + p0 + p1
            0x04,
            b's',
            b'e',
            b'l',
            b'f',
            0x04, // self : own<t> index 4
            0x02,
            b'p',
            b'0',
            0x05, // p0 : tuple type index 5
            0x02,
            b'p',
            b'1',
            0x06, // p1 : tuple type index 6
            0x00,
            s64, // result s64
        ];
        assert_eq!(ft2, want2, "two-tuple-arg call functype byte shape");
    }
}
