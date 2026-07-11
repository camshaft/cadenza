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

use crate::backend::wasm::encode::{section, uleb_bytes, uleb128, wasm_vec};
use crate::backend::wasm::runtime_abi::RtOp;

/// The component-model preamble (`\0asm` + component-layer version).
const COMPONENT_MAGIC: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x0D, 0x00, 0x01, 0x00];

/// Component section ids used by the envelope (component-model numbering, distinct from core wasm).
mod sec {
    pub const CORE_MODULE: u8 = 1;
    pub const CORE_INSTANCE: u8 = 2;
    pub const ALIAS: u8 = 6;
    pub const COMPONENT_TYPE: u8 = 7;
    pub const CANON: u8 = 8;
    pub const COMPONENT_IMPORT: u8 = 10;
    pub const COMPONENT_EXPORT: u8 = 11;
}

/// The core wasm module name the program's core module imports the runtime funcs from, and the name
/// the threaded core-instance of lowered ops is bound under (they must match).
const HEAP_MODULE: &str = "heap";

/// One export as the envelope assembler needs it: its verbatim boundary name, its parameter component
/// valtype bytes (in order; empty for a nullary export), and its result's component valtype byte
/// (`None` for a unit / no-result export).
pub struct BoundaryExport {
    pub name: String,
    pub params: Vec<u8>,
    pub result: Option<u8>,
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

/// The BARE shape (no runtime import) — unchanged from before the value heap, byte-identical to the
/// oracle for a runtime-free program.
fn assemble_bare(core: &[u8], exports: &[BoundaryExport]) -> Vec<u8> {
    let n = exports.len();

    // sec 7: one component functype per export (nullary → its result form).
    let mut type_items = Vec::new();
    for e in exports {
        type_items.extend_from_slice(&comp_functype(e));
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

    // sec 7 (second): one component functype per boundary export → component types `1..=m`.
    let boundary_type_sec = {
        let mut items = Vec::new();
        for e in exports {
            items.extend_from_slice(&comp_functype(e));
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
    let mut item = vec![0x40];
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
/// result form is `00 <valtype>` for one result, `01 00` for none.
fn comp_functype(e: &BoundaryExport) -> Vec<u8> {
    let mut item = vec![0x40]; // function type form
    let mut param_items = Vec::new();
    for (i, &vt) in e.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(e.params.len(), &param_items));
    match e.result {
        Some(vt) => item.extend_from_slice(&[0x00, vt]),
        None => item.extend_from_slice(&[0x01, 0x00]),
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

/// A sec-8 canon-lift item: `00 00 <core-func> 00 <type>` — `00 00` canon lift core func, `00` empty
/// canon-options, then the component type index.
fn canon_lift_item(core_func: u32, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00, 0x00];
    uleb128(core_func as u64, &mut item);
    item.push(0x00); // canon options: none
    uleb128(type_idx as u64, &mut item);
    item
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
