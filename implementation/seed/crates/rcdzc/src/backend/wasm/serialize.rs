//! `serialize` — the flat Lir of a module's functions laid into a core wasm module's bytes.
//!
//! The ONLY place that produces core-module bytes; it decides nothing (calls are already absolute,
//! the boundary is already fixed by the layout). A module of N functions becomes a core module with
//! four sections — type, function, export, code — its functions emitted in the layout's emission
//! order, so a body at emission position `k` is core func `k` (`reference-compiler.md` §Emission
//! Serializes A Lowered Representation). The export section names EVERY boundary function by its
//! verbatim source name (multi-export from the start — no single hard-coded `run`).

use crate::backend::wasm::encode::{op, section, uleb_bytes, uleb128, wasm_vec};
use crate::backend::wasm::lir::{Lir, ValType, comp_valtype_of, valtype_of};
use crate::backend::wasm::runtime_abi::RtOp;
use crate::backend::wasm::select::SelectedFunc;
use crate::backend::wasm::wasm_abi;
use crate::layout::Layout;
use crate::ty::Ty;

/// The `\0asm` version-1 core-module preamble — from the generated `wasm_abi` table (`Module::HEADER`
/// as `wasm-encoder` writes it), not a hand-typed byte string.
const CORE_MAGIC: &[u8] = wasm_abi::CORE_MAGIC;

/// The core functype `0x60 <params-vec> <results-vec>` of a runtime import op (from its generated
/// signature). Each ABI type projects to its CORE valtype byte (`AbiValType::core_byte` — a `u32`
/// handle lowers to i32, an `s64` to i64, …); the component-boundary bytes are the envelope's concern.
/// A runtime op returns at most one core value; `dup`/`drop` return none.
fn import_functype(o: &RtOp) -> Vec<u8> {
    let mut out = vec![wasm_abi::CORE_FUNCTYPE_FORM];
    let params: Vec<u8> = o.params.iter().map(|c| c.core_byte()).collect();
    out.extend_from_slice(&wasm_vec(params.len(), &params));
    match o.result {
        Some(c) => out.extend_from_slice(&wasm_vec(1, &[c.core_byte()])),
        None => out.extend_from_slice(&wasm_vec(0, &[])),
    }
    out
}

/// One core import item: `<mod-len><mod> <name-len><name> 00 <typeidx>` — importing a func (desc kind
/// `0x00`) of the given type index from module `"heap"` (the module name the component's threaded
/// core-instance is bound under). The runtime resolves the op by its `name`.
fn import_item(op_name: &str, type_idx: u32) -> Vec<u8> {
    const HEAP_MODULE: &str = "heap";
    let mut item = uleb_bytes(HEAP_MODULE.len() as u64);
    item.extend_from_slice(HEAP_MODULE.as_bytes());
    item.extend_from_slice(&uleb_bytes(op_name.len() as u64));
    item.extend_from_slice(op_name.as_bytes());
    item.push(0x00); // import desc: func
    uleb128(type_idx as u64, &mut item);
    item
}

/// Serialize one flat instruction, appending its bytes to `out`. `import_index` maps a runtime op's
/// name to its core function index (its position `0..k` in the import section), so a `CallImport`
/// resolves by name to the same index the import section assigned. Exhaustive over `Lir`.
fn instr(i: &Lir, import_index: &std::collections::HashMap<&str, u32>, out: &mut Vec<u8>) {
    match i {
        Lir::ConstI64(n) => {
            out.push(op::I64_CONST);
            crate::backend::wasm::encode::sleb128(*n, out);
        }
        Lir::ConstI32(n) => {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(*n as i64, out);
        }
        Lir::LocalGet(idx) => {
            out.push(op::LOCAL_GET);
            uleb128(*idx as u64, out);
        }
        Lir::LocalSet(idx) => {
            out.push(op::LOCAL_SET);
            uleb128(*idx as u64, out);
        }
        Lir::LocalTee(idx) => {
            out.push(op::LOCAL_TEE);
            uleb128(*idx as u64, out);
        }
        Lir::GlobalGet(idx) => {
            out.push(op::GLOBAL_GET);
            uleb128(*idx as u64, out);
        }
        Lir::GlobalSet(idx) => {
            out.push(op::GLOBAL_SET);
            uleb128(*idx as u64, out);
        }
        Lir::Call(func) => {
            out.push(op::CALL);
            uleb128(*func as u64, out);
        }
        Lir::ReturnCall(func) => {
            out.push(op::RETURN_CALL);
            uleb128(*func as u64, out);
        }
        Lir::CallImport(name) => {
            // Resolve the op name to its import function index (its position in the import section).
            // A `CallImport` for an op not in the import set is a compiler bug — `collect_used_ops`
            // computes the set from the same emit, so every emitted op is present. Fall back to a
            // no-op index of 0 is WRONG (would call the first import); instead index the map and, if
            // absent (a bug), emit an obviously-invalid high index so validation catches it loudly.
            let idx = import_index.get(name).copied().unwrap_or(u32::MAX);
            out.push(op::CALL);
            uleb128(idx as u64, out);
        }
        Lir::If(bt) => {
            out.push(op::IF);
            out.push(bt.byte()); // block-type byte lives here, not in the IR
        }
        Lir::Block(bt) => {
            out.push(op::BLOCK);
            out.push(bt.byte());
        }
        Lir::Loop(bt) => {
            out.push(op::LOOP);
            out.push(bt.byte());
        }
        Lir::Br(depth) => {
            out.push(op::BR);
            uleb128(*depth as u64, out);
        }
        Lir::BrIf(depth) => {
            out.push(op::BR_IF);
            uleb128(*depth as u64, out);
        }
        // `br_table`: the target vector (count-prefixed uleb128 entries) then the default target.
        Lir::BrTable(targets, default) => {
            out.push(op::BR_TABLE);
            uleb128(targets.len() as u64, out);
            for t in targets {
                uleb128(*t as u64, out);
            }
            uleb128(*default as u64, out);
        }
        Lir::Else => out.push(op::ELSE),
        Lir::End => out.push(op::END),
        Lir::Select => out.push(op::SELECT),
        Lir::Unreachable => out.push(op::UNREACHABLE),
        // `if (empty) unreachable end` — trap when the i32 condition is nonzero, leaving nothing.
        Lir::IfUnreachableEnd => {
            out.push(op::IF);
            out.push(wasm_abi::BLOCK_EMPTY); // empty block type
            out.push(op::UNREACHABLE);
            out.push(op::END);
        }
        Lir::I64Add => out.push(op::I64_ADD),
        Lir::I64Sub => out.push(op::I64_SUB),
        Lir::I64Mul => out.push(op::I64_MUL),
        Lir::I32Add => out.push(op::I32_ADD),
        Lir::I32Sub => out.push(op::I32_SUB),
        Lir::I32Mul => out.push(op::I32_MUL),
        Lir::I32DivS => out.push(op::I32_DIV_S),
        Lir::I32DivU => out.push(op::I32_DIV_U),
        Lir::I32RemS => out.push(op::I32_REM_S),
        Lir::I32RemU => out.push(op::I32_REM_U),
        Lir::I32And => out.push(op::I32_AND),
        Lir::I32Or => out.push(op::I32_OR),
        Lir::I32Xor => out.push(op::I32_XOR),
        Lir::I32Ne => out.push(op::I32_NE),
        Lir::I32Shl => out.push(op::I32_SHL),
        Lir::I32ShrS => out.push(op::I32_SHR_S),
        Lir::I32ShrU => out.push(op::I32_SHR_U),
        Lir::I64Eq => out.push(op::I64_EQ),
        Lir::I64LtS => out.push(op::I64_LT_S),
        Lir::I64GtS => out.push(op::I64_GT_S),
        Lir::I64LeS => out.push(op::I64_LE_S),
        Lir::I64GeS => out.push(op::I64_GE_S),
        Lir::I64LtU => out.push(op::I64_LT_U),
        Lir::I64GtU => out.push(op::I64_GT_U),
        Lir::I64LeU => out.push(op::I64_LE_U),
        Lir::I64GeU => out.push(op::I64_GE_U),
        Lir::I64Eqz => out.push(op::I64_EQZ),
        Lir::I32Eqz => out.push(op::I32_EQZ),
        Lir::I32Eq => out.push(op::I32_EQ),
        Lir::I32LtS => out.push(op::I32_LT_S),
        Lir::I32GtS => out.push(op::I32_GT_S),
        Lir::I32LeS => out.push(op::I32_LE_S),
        Lir::I32GeS => out.push(op::I32_GE_S),
        Lir::I32LtU => out.push(op::I32_LT_U),
        Lir::I32GtU => out.push(op::I32_GT_U),
        Lir::I32LeU => out.push(op::I32_LE_U),
        Lir::I32GeU => out.push(op::I32_GE_U),
        Lir::I64Ne => out.push(op::I64_NE),
        Lir::I64And => out.push(op::I64_AND),
        Lir::I64Or => out.push(op::I64_OR),
        Lir::I64Xor => out.push(op::I64_XOR),
        Lir::I64Shl => out.push(op::I64_SHL),
        Lir::I64ShrS => out.push(op::I64_SHR_S),
        Lir::I64ShrU => out.push(op::I64_SHR_U),
        Lir::I64DivS => out.push(op::I64_DIV_S),
        Lir::I64DivU => out.push(op::I64_DIV_U),
        Lir::I64RemS => out.push(op::I64_REM_S),
        Lir::I64RemU => out.push(op::I64_REM_U),
        Lir::I32WrapI64 => out.push(op::I32_WRAP_I64),
        Lir::I64ExtendI32S => out.push(op::I64_EXTEND_I32_S),
        Lir::I64ExtendI32U => out.push(op::I64_EXTEND_I32_U),
    }
}

/// One function's code-section entry: `<size> <local-decls> <instrs> end`. `import_index` resolves a
/// `CallImport` op name to its import function index.
fn code_entry(f: &SelectedFunc, import_index: &std::collections::HashMap<&str, u32>) -> Vec<u8> {
    let mut inner = Vec::new();
    // Local declarations, run-length-encoded by value type (a body with no locals → count 0).
    let groups = rle(&f.declared);
    uleb128(groups.len() as u64, &mut inner);
    for (count, vt) in groups {
        uleb128(count as u64, &mut inner);
        inner.push(vt.byte());
    }
    for i in &f.code {
        instr(i, import_index, &mut inner);
    }
    inner.push(op::END);
    let mut out = uleb_bytes(inner.len() as u64);
    out.extend_from_slice(&inner);
    out
}

/// One function's core functype: `0x60 <params-vec> <results-vec>`. A unit return is a zero-result
/// function; any other return is one result of its value type. The form tag is the generated
/// `wasm_abi::CORE_FUNCTYPE_FORM` (from `wasm-encoder`), not a hand-typed `0x60`.
fn functype(f: &SelectedFunc) -> Result<Vec<u8>, String> {
    let mut out = vec![wasm_abi::CORE_FUNCTYPE_FORM];
    let param_bytes: Vec<u8> = f.params.iter().map(|vt| vt.byte()).collect();
    out.extend_from_slice(&wasm_vec(param_bytes.len(), &param_bytes));
    match valtype_of(&f.ret) {
        Some(vt) => out.extend_from_slice(&wasm_vec(1, &[vt.byte()])),
        None if matches!(f.ret, Ty::Unit) => out.extend_from_slice(&wasm_vec(0, &[])),
        None => return Err("function return type has no machine representation".to_string()),
    }
    Ok(out)
}

/// Debug metadata the backend injects into the embedded core module — the `name` custom section's
/// inputs (D0 of `DESIGN-debug-info-rcdzc.md`). `module_name` is the program's name (its first export);
/// `func_names` are `(defined-function index, source name)` pairs for the program's OWN functions, in
/// ascending index order (a nameless def is omitted). The imported runtime ops are named from the
/// `imports` slice inside `core_module`, so they are not repeated here. Present only when a debug `Emit`
/// request drives the compile (Mode E); absent → byte-identical to today (the reproducibility anchor).
pub struct DebugInfo<'a> {
    pub module_name: &'a str,
    pub func_names: &'a [(u32, String)],
}

/// A length-prefixed name string, the wasm `name` section's string form: `<uleb len> <utf8 bytes>`.
fn name_string(s: &str) -> Vec<u8> {
    let mut out = uleb_bytes(s.len() as u64);
    out.extend_from_slice(s.as_bytes());
    out
}

/// One `name` subsection: `<id byte> <uleb payload-len> <payload>` (the same framing as a section, but
/// the id is one byte and it lives inside the custom section's contents).
fn name_subsection(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![id];
    uleb128(payload.len() as u64, &mut out);
    out.extend_from_slice(payload);
    out
}

/// The wasm `name` CUSTOM section (id 0, name `"name"`) — a module-name subsection (id 0) + a
/// function-name subsection (id 1) mapping a function index to its source name. Inert by construction:
/// a custom section moves no byte of the executed (type/func/export/code) sections, so appending it
/// changes no observable behavior and `wasm-tools strip` recovers the undecorated bytes (the D0/§5
/// guarantees). `func_names` must be ASCENDING by index (the name-map's wire form is ordered); the
/// caller concatenates imports-then-defined, which is already ascending. Turns `func[N]` into a source
/// name in every trace, profile, and debugger frame — DWARF-independent, highest value-per-line.
fn name_section(module_name: &str, func_names: &[(u32, String)]) -> Vec<u8> {
    // Contents: the section-name string "name", then the subsections in id order.
    let mut contents = name_string("name");
    // Subsection 0 — module name.
    contents.extend_from_slice(&name_subsection(0, &name_string(module_name)));
    // Subsection 1 — function names: a name map `<count> <(idx, name)…>` in ascending index order.
    let mut map = uleb_bytes(func_names.len() as u64);
    for (idx, name) in func_names {
        uleb128(*idx as u64, &mut map);
        map.extend_from_slice(&name_string(name));
    }
    contents.extend_from_slice(&name_subsection(1, &map));
    // Custom section: id 0.
    section(0, &contents)
}

/// Assemble the embedded core module for a module's selected functions. `funcs[k]` is the function at
/// emission position `k` (already in the layout's order). `imports` is the program's per-program set of
/// runtime ops (ordered — the same order `layout` numbered them), imported from module `"heap"` at core
/// func indices `0..imports.len()`; the program's own DEFINED functions therefore start at core index
/// `imports.len()`, and the export section (and every `Lir::Call`, via `layout.abs`) account for that
/// shift. An empty `imports` emits no import section and no shift — byte-identical to a runtime-free
/// program (`component-abi.md` v3 migration: a program importing nothing crosses as under v2).
///
/// `debug` carries the `name`-section inputs when a debug `Emit` request drives the compile (Mode E);
/// `None` (the common case) appends nothing, so the bytes are exactly today's. The `name` section is
/// appended LAST — after the code section — so it is inert by construction (`DESIGN-debug-info-rcdzc.md`
/// §1: appending a custom section moves no executed byte, so the byte-oracle tests over the executed
/// sections still hold and `wasm-tools strip` recovers the undecorated module).
pub fn core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    layout: &Layout,
    debug: Option<&DebugInfo>,
) -> Result<Vec<u8>, String> {
    let n = funcs.len();
    let import_count = imports.len();

    // Type section: the IMPORT functypes first (type indices `0..import_count`), then one functype per
    // defined function (type indices `import_count..import_count+n`). The type index space is separate
    // from the function index space, but numbering imports' types first keeps a defined func's type
    // index equal to `import_count + its emission position`, which the function section references.
    let mut type_items = Vec::new();
    for o in imports {
        type_items.extend_from_slice(&import_functype(o));
    }
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let type_sec = section(
        wasm_abi::CORE_SEC_TYPE,
        &wasm_vec(import_count + n, &type_items),
    );

    // Import section (id 2) — one func import per runtime op, in order, from module `"heap"`. Occupies
    // core FUNCTION indices `0..import_count`. Omitted entirely when there are no imports. The same
    // order fixes the `import_index` map a `CallImport` resolves against, so an op's call index equals
    // its position here.
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let import_sec = if import_count == 0 {
        Vec::new()
    } else {
        let mut import_items = Vec::new();
        for (i, o) in imports.iter().enumerate() {
            import_items.extend_from_slice(&import_item(o.name, i as u32));
            import_index.insert(o.name, i as u32);
        }
        section(2, &wasm_vec(import_count, &import_items))
    };

    // Function section: defined func `i` (function index `import_count + i`) uses type index
    // `import_count + i` (the import functypes came first).
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((import_count + i) as u64, &mut func_items);
    }
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(n, &func_items));

    // Export section: export every boundary function under its verbatim name, by its absolute core
    // function index (`layout.abs`, which already includes the import shift).
    let mut export_items = Vec::new();
    for e in &layout.exports {
        let abs = layout.abs(e.def).ok_or_else(|| {
            format!(
                "exported definition `{}` is not in the emission order",
                e.name
            )
        })?;
        let mut item = uleb_bytes(e.name.len() as u64);
        item.extend_from_slice(e.name.as_bytes());
        item.push(wasm_abi::EXPORT_KIND_FUNC); // export kind: func
        uleb128(abs as u64, &mut item);
        export_items.extend_from_slice(&item);
    }
    let export_sec = section(
        wasm_abi::CORE_SEC_EXPORT,
        &wasm_vec(layout.exports.len(), &export_items),
    );

    // Code section: bodies in emission order.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    let code_sec = section(wasm_abi::CORE_SEC_CODE, &wasm_vec(n, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    // DEBUG (Mode E): append the `name` custom section LAST, after every executed section — so it is
    // inert (moves no executed byte) and strippable. The function-name map names the imported runtime
    // ops (indices `0..import_count`, from `imports`) then the program's own defined functions (indices
    // `import_count..`, from `debug.func_names`), concatenated so the whole map is ascending by index.
    if let Some(dbg) = debug {
        let mut func_names: Vec<(u32, String)> = imports
            .iter()
            .enumerate()
            .map(|(i, o)| (i as u32, o.name.to_string()))
            .collect();
        func_names.extend(dbg.func_names.iter().cloned());
        core.extend_from_slice(&name_section(dbg.module_name, &func_names));
    }
    Ok(core)
}

/// The standalone STUB DTOR core module for the CONSTANT resource escape (R1): exports `t-dtor : (i32
/// rep) -> ()`, imports nothing, empty body. A constant compound carries no live runtime handle (its
/// bytes are baked), so the dtor has nothing to release. Kept in its OWN module (importing nothing) so it
/// instantiates FIRST — the resource type gets a real dtor core-func before `resource.new` needs the
/// resource type, dissolving the resource↔dtor↔`resource.new` cycle with no shim/fixup
/// ([[rcdzc-r1-resource-encode-linking-findings]]). Byte-identical to the R1 `oracle`'s `dtor_module`.
/// The RUNTIME escape (R2) uses [`resource_dtor_module_with_drop`] instead — its handle is live and must
/// be released.
pub fn resource_dtor_module() -> Vec<u8> {
    // sec 1 type: one functype `(i32) -> ()`.
    let ty = {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t.extend_from_slice(&wasm_vec(0, &[]));
        t
    };
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(1, &ty));
    // sec 3 function: one func of type 0.
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(1, &uleb_bytes(0)));
    // sec 7 export: `t-dtor` = func 0.
    let export_sec = {
        let mut item = uleb_bytes("t-dtor".len() as u64);
        item.extend_from_slice(b"t-dtor");
        item.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128(0, &mut item);
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(1, &item))
    };
    // sec 10 code: one body — no locals, just `end` (the stub release).
    let code_sec = {
        let mut body = uleb_bytes(0); // zero local-decl groups
        body.push(op::END);
        let entry = {
            let mut e = uleb_bytes(body.len() as u64);
            e.extend_from_slice(&body);
            e
        };
        section(wasm_abi::CORE_SEC_CODE, &wasm_vec(1, &entry))
    };
    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    core
}

/// The DTOR core module for the RUNTIME resource escape (R2): `t-dtor : (i32 rep) -> ()` that RELEASES
/// the compound's rc handle by calling the runtime `drop` (imported as `heap-dtor.drop`). Fires when the
/// host drops the resource — or when `encode` consumes its `own<t>` — and `drop` cascades the release
/// into the compound's boxed children (a complete reclamation, since the value heap is acyclic). It
/// imports `drop` from a SEPARATE small core instance (`heap-dtor`) the envelope builds from the LOWERED
/// `drop` op (a core func that exists BEFORE the resource type), NOT from the full `heap` instance (which
/// needs `resource-new`/`resource-rep`, and hence the resource type). That is what lets this module still
/// instantiate before the resource type, keeping the resource↔dtor↔`resource.new` cycle dissolved
/// ([[rcdzc-r1-resource-encode-linking-findings]] R2). Byte-identical to the R2 oracle's `dtor_module`.
pub fn resource_dtor_module_with_drop() -> Vec<u8> {
    // sec 1 type: one functype `(i32) -> ()` — shared by the `drop` import and `t-dtor`.
    let ty = {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t.extend_from_slice(&wasm_vec(0, &[]));
        t
    };
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(1, &ty));
    // sec 2 import: `heap-dtor.drop : (i32) -> ()` → core func 0.
    let import_sec = {
        let mut item = uleb_bytes("heap-dtor".len() as u64);
        item.extend_from_slice(b"heap-dtor");
        item.extend_from_slice(&uleb_bytes("drop".len() as u64));
        item.extend_from_slice(b"drop");
        item.push(0x00); // import desc: func
        uleb128(0, &mut item); // type 0
        section(2, &wasm_vec(1, &item))
    };
    // sec 3 function: t-dtor is defined func 1 (the import is func 0), of type 0.
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(1, &uleb_bytes(0)));
    // sec 7 export: `t-dtor` = func 1.
    let export_sec = {
        let mut item = uleb_bytes("t-dtor".len() as u64);
        item.extend_from_slice(b"t-dtor");
        item.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128(1, &mut item);
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(1, &item))
    };
    // sec 10 code: `local.get 0 ; call 0 (drop) ; end`.
    let code_sec = {
        let mut body = uleb_bytes(0); // no locals
        body.push(op::LOCAL_GET);
        uleb128(0, &mut body);
        body.push(op::CALL);
        uleb128(0, &mut body); // the imported drop
        body.push(op::END);
        let entry = {
            let mut e = uleb_bytes(body.len() as u64);
            e.extend_from_slice(&body);
            e
        };
        section(wasm_abi::CORE_SEC_CODE, &wasm_vec(1, &entry))
    };
    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    core
}

/// The program core module for a CONSTANT-compound resource escape (R1). It IMPORTS
/// `heap.resource-new : (i32 rep) -> i32 handle` (the `resource.new` intrinsic the component threads in;
/// a raw rep is NOT auto-wrapped by the lift), and exports `memory`, `make : () -> i32 handle`,
/// `t-encode : (i32 rep) -> i32 retptr`, and `cabi_realloc`. `make` registers a dummy rep (`0`) as a
/// resource handle; `t-encode` ignores the rep and returns a pointer to `value_bytes` laid in linear
/// memory as the canonical `(ptr, len)` return area (the R0 `list<u8>` ABI). `value_bytes` is the
/// constant canonical value form ([`crate::lower::constant_value_form`]). Byte-shaped like the oracle's
/// `resource_core`, but with the program's real value bytes. R2 replaces the constant `encode` with the
/// real handle-walking serializer.
pub fn resource_core_module(value_bytes: &[u8]) -> Vec<u8> {
    // Memory layout: the value bytes at offset 0; the (ptr,len) return area immediately after, aligned
    // to 4. ptr = 0, len = value_bytes.len().
    let payload_len = value_bytes.len();
    let retarea = (payload_len + 3) & !3; // 4-byte-align the return area after the payload
    let mut data_bytes = value_bytes.to_vec();
    data_bytes.resize(retarea, 0);
    // return area: ptr (i32-le) = 0, len (i32-le) = payload_len.
    data_bytes.extend_from_slice(&0u32.to_le_bytes());
    data_bytes.extend_from_slice(&(payload_len as u32).to_le_bytes());
    let retarea_ptr = retarea as i64;

    // sec 1 types: 0 = (i32)->i32 (resource-new / encode), 1 = ()->i32 (make), 2 = cabi_realloc.
    let types = {
        let mut items = Vec::new();
        let mut t0 = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t0.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t0.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        items.extend_from_slice(&t0);
        let mut t1 = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t1.extend_from_slice(&wasm_vec(0, &[]));
        t1.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        items.extend_from_slice(&t1);
        let mut t2 = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t2.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t2.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        items.extend_from_slice(&t2);
        section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(3, &items))
    };
    // sec 2 import: `heap.resource-new` of type 0 → func 0.
    let import_sec = section(2, &wasm_vec(1, &import_item("resource-new", 0)));
    // sec 3 functions: make (type 1), encode (type 0), cabi_realloc (type 2) → funcs 1,2,3.
    let func_sec = {
        let mut items = Vec::new();
        uleb128(1, &mut items);
        uleb128(0, &mut items);
        uleb128(2, &mut items);
        section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(3, &items))
    };
    // sec 5 memory: one memory, min 1 page.
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));
    // sec 7 exports: memory=mem0, make=func1, t-encode=func2, cabi_realloc=func3.
    let export_sec = {
        let export = |name: &str, kind: u8, idx: u32| {
            let mut item = uleb_bytes(name.len() as u64);
            item.extend_from_slice(name.as_bytes());
            item.push(kind);
            uleb128(idx as u64, &mut item);
            item
        };
        let mut items = Vec::new();
        items.extend_from_slice(&export("memory", wasm_abi::EXPORT_KIND_MEMORY, 0));
        items.extend_from_slice(&export("make", wasm_abi::EXPORT_KIND_FUNC, 1));
        items.extend_from_slice(&export("t-encode", wasm_abi::EXPORT_KIND_FUNC, 2));
        items.extend_from_slice(&export("cabi_realloc", wasm_abi::EXPORT_KIND_FUNC, 3));
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(4, &items))
    };
    // sec 10 code: make (i32.const 0; call 0 resource-new), encode (i32.const retarea_ptr), realloc
    // (i32.const 0 stub).
    let code_sec = {
        let body = |instrs: &[Lir]| {
            let mut inner = uleb_bytes(0); // no locals
            for i in instrs {
                instr(i, &std::collections::HashMap::new(), &mut inner);
            }
            inner.push(op::END);
            let mut e = uleb_bytes(inner.len() as u64);
            e.extend_from_slice(&inner);
            e
        };
        let mut items = Vec::new();
        items.extend_from_slice(&body(&[Lir::ConstI32(0), Lir::Call(0)])); // make: rep 0 → resource-new
        items.extend_from_slice(&body(&[Lir::ConstI32(retarea_ptr as i32)])); // encode: return retptr
        items.extend_from_slice(&body(&[Lir::ConstI32(0)])); // cabi_realloc: stub
        section(wasm_abi::CORE_SEC_CODE, &wasm_vec(3, &items))
    };
    // sec 11 data: active segment at offset 0.
    let data_sec = {
        let mut item = vec![0x00]; // active, memory 0
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut item);
        item.push(op::END);
        item.extend_from_slice(&uleb_bytes(data_bytes.len() as u64));
        item.extend_from_slice(&data_bytes);
        section(wasm_abi::CORE_SEC_DATA, &wasm_vec(1, &item))
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&types);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    core
}

/// The program core module for a RUNTIME-compound resource escape (R2) — a compound BUILT ON THE HEAP
/// (not a compile-time constant) crossing to the host, whose `encode()` walks the live handle. Unlike
/// the constant [`resource_core_module`] (which bakes the value bytes), this emits the REAL program:
///
///  * It imports the `k` runtime ops (`imports`, at core func indices `0..k`) plus the two resource
///    intrinsics `resource-new` (core func `k`) and `resource-rep` (core func `k+1`), all from `"heap"`.
///  * It emits every reachable definition body (`funcs`, selected with `import_base = k+2` so their
///    `Lir::Call` indices land past the imports), at core funcs `k+2 .. k+2+n`.
///  * It synthesizes `make : () -> i32` = call the escaping export (which builds the compound on the
///    heap and returns its handle) then `resource.new(handle)` to register it; `t-encode : (i32) -> i32`
///    = `resource.rep(handle)` to recover the heap rep, then WALK the `template`'s holes (each an
///    `arr-get` path + a leaf read), writing bytes into the template-as-output-buffer, returning the
///    `(ptr=0, len)` area; and a stub `cabi_realloc`.
///  * It exports `memory`, `make`, `t-encode`, `cabi_realloc` — the four the resource envelope aliases.
///
/// `export_abs` is the absolute core-func index of the escaping export (its selected body, which returns
/// the compound handle) — `make` calls it. `template` is the value-form byte template
/// ([`crate::lower::runtime_value_form_template`]); its bytes lie at memory offset 0 and double as the
/// output buffer. Mirrors the proven `r2_runtime_resource::walker_core` oracle, but with the program's
/// real bodies + the compiler's actual op set / export.
pub fn runtime_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    template: &crate::lower::ValueFormTemplate,
) -> Result<Vec<u8>, String> {
    runtime_resource_core_module_form(funcs, imports, export_abs, EscapeForm::Flat(template))
}

/// The value-form the escape `t-encode` walker renders — a single flat template (a tuple/record, ONE
/// shape) or a per-variant sum template (a disc-switch over N shapes). Both share the identical
/// type/import/func/memory/export envelope; only the data layout + the `t-encode` body differ.
pub enum EscapeForm<'a> {
    Flat(&'a crate::lower::ValueFormTemplate),
    Sum(&'a crate::lower::SumFormTemplate),
}

/// The escape core module for either a flat compound or a sum result (see [`EscapeForm`]). Everything
/// but the data section and the `t-encode` body is common — the resource shape (`make`/`t-encode`/
/// `cabi_realloc` + memory), the imports (`k` ops + `resource-new`/`resource-rep`), and the defined
/// bodies. The data section lays the value-form bytes (one template, or every variant's template
/// consecutively) as the output buffer; `t-encode` recovers the heap rep and either walks the one
/// template's holes or switches on `sum-disc` and walks the matching variant's.
pub fn runtime_resource_core_module_form(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    form: EscapeForm,
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();

    // ── Type section ──
    // Import functypes 0..k (the runtime ops), then resource-new/resource-rep (both `(i32)->i32`), then
    // one functype per defined body, then the three synthesized-func types (make `()->i32`, encode
    // `(i32)->i32`, cabi_realloc `(i32×4)->i32`).
    let mut type_items = Vec::new();
    for o in imports {
        type_items.extend_from_slice(&import_functype(o));
    }
    let i32_to_i32 = {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t
    };
    type_items.extend_from_slice(&i32_to_i32); // resource-new type (index k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep type (index k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    // make `()->i32`.
    let make_type_idx = defined_type_base + n;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(0, &[]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // t-encode `(i32)->i32` (reuse the shape but a distinct index for clarity).
    let encode_type_idx = make_type_idx + 1;
    type_items.extend_from_slice(&i32_to_i32);
    // cabi_realloc `(i32×4)->i32`.
    let realloc_type_idx = encode_type_idx + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + 3;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── k ops + resource-new + resource-rep, all from "heap". Also builds the
    // `import_index` a `CallImport` resolves against (op name → its `0..k` index).
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut import_items = Vec::new();
    for (i, o) in imports.iter().enumerate() {
        import_items.extend_from_slice(&import_item(o.name, i as u32));
        import_index.insert(o.name, i as u32);
    }
    import_items.extend_from_slice(&import_item("resource-new", k as u32));
    import_items.extend_from_slice(&import_item("resource-rep", (k + 1) as u32));
    let import_sec = section(2, &wasm_vec(k + 2, &import_items));
    let f_rnew = k as u32;
    let f_rrep = (k + 1) as u32;

    // ── Function section ── defined bodies use their functype (`defined_type_base + i`), then the three
    // synthesized funcs. Defined func indices: `k+2 .. k+2+n`; make = `k+2+n`, encode = `k+3+n`,
    // realloc = `k+4+n`.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    uleb128(make_type_idx as u64, &mut func_items);
    uleb128(encode_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(n + 3, &func_items));
    let make_abs = (defined_type_base + n) as u32;
    let encode_abs = make_abs + 1;
    let realloc_abs = encode_abs + 1;

    // ── Memory section ── one memory, min 1 page.
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Export section ── memory, make, t-encode, cabi_realloc.
    let export_sec = {
        let export = |name: &str, kind: u8, idx: u32| {
            let mut item = uleb_bytes(name.len() as u64);
            item.extend_from_slice(name.as_bytes());
            item.push(kind);
            uleb128(idx as u64, &mut item);
            item
        };
        let mut items = Vec::new();
        items.extend_from_slice(&export("memory", wasm_abi::EXPORT_KIND_MEMORY, 0));
        items.extend_from_slice(&export("make", wasm_abi::EXPORT_KIND_FUNC, make_abs));
        items.extend_from_slice(&export("t-encode", wasm_abi::EXPORT_KIND_FUNC, encode_abs));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(4, &items))
    };

    // ── Data section ── lay the value-form bytes as the output buffer, then each template's (ptr,len)
    // return area 4-aligned after it. A FLAT compound is one template at offset 0 + one ret area. A SUM
    // lays every variant's template CONSECUTIVELY (each 4-aligned), each with its own (ptr,len) area —
    // the walker switches on `sum-disc` to pick which region to fill + return. `placed` records each
    // template's `(byte_off, ret_off)` for the encode body.
    let mut data_bytes: Vec<u8> = Vec::new();
    let mut placed: Vec<Placed> = Vec::new();
    let templates: Vec<&crate::lower::ValueFormTemplate> = match &form {
        EscapeForm::Flat(t) => vec![*t],
        EscapeForm::Sum(s) => s.variants.iter().collect(),
    };
    for t in &templates {
        // 4-align the start of this template's bytes.
        let byte_off = (data_bytes.len() + 3) & !3;
        data_bytes.resize(byte_off, 0);
        data_bytes.extend_from_slice(&t.bytes);
        // Its (ptr,len) return area, 4-aligned after the bytes: ptr = byte_off, len = template length.
        let ret_off = (data_bytes.len() + 3) & !3;
        data_bytes.resize(ret_off, 0);
        data_bytes.extend_from_slice(&(byte_off as u32).to_le_bytes());
        data_bytes.extend_from_slice(&(t.bytes.len() as u32).to_le_bytes());
        placed.push(Placed { byte_off, ret_off });
    }
    let data_sec = {
        let mut item = vec![0x00]; // active, memory 0
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut item);
        item.push(op::END);
        item.extend_from_slice(&uleb_bytes(data_bytes.len() as u64));
        item.extend_from_slice(&data_bytes);
        section(wasm_abi::CORE_SEC_DATA, &wasm_vec(1, &item))
    };

    // ── Code section ── the defined bodies (in emission order), then make / t-encode / cabi_realloc.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // make: `call <export>` (builds the compound → its heap handle on the stack) then
    // `call resource-new` (register the handle → a resource handle).
    {
        let mut inner = uleb_bytes(0); // no locals
        inner.push(op::CALL);
        uleb128(export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // t-encode(handle): recover the heap rep, then either walk the one template's holes (flat) or
    // switch on `sum-disc` and walk the matching variant's (sum).
    let encode_body = match &form {
        EscapeForm::Flat(t) => encode_walk_body(
            t,
            placed[0].byte_off,
            placed[0].ret_off,
            f_rrep,
            &import_index,
        ),
        EscapeForm::Sum(s) => {
            encode_sum_walk_body(&s.variants, &placed_pairs(&placed), f_rrep, &import_index)
        }
    };
    code_items.extend_from_slice(&encode_body);
    // cabi_realloc: stub (never called for a nullary-input list result).
    {
        let mut inner = uleb_bytes(0);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(wasm_abi::CORE_SEC_CODE, &wasm_vec(n + 3, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    Ok(core)
}

/// The `t-encode(handle) -> i32` code-section entry (the R2 walker). Locals: 0 = the resource-table
/// handle param, 1 = the recovered i32 heap rep, 2 = i64 scratch. Recovers the rep via
/// `resource.rep(handle)` (core func `f_rrep`), then for each template hole walks its `arr-get` path
/// from the rep, reads the leaf (`get-int`/`get-bool`), and writes its bytes into the template (at mem
/// offset 0, doubling as the output buffer); returns the `(ptr=0, len)` return area at `ret_off`. The
/// walk ops resolve by name through `import_index` (the same map the defined bodies use).
fn encode_walk_body(
    template: &crate::lower::ValueFormTemplate,
    byte_off: usize,
    ret_off: usize,
    f_rrep: u32,
    import_index: &std::collections::HashMap<&str, u32>,
) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let call_op = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(import_index[name] as u64, out);
    };
    let mut body = Vec::new();
    // Locals: 1 group of i32 (the rep), 1 group of i64 (scratch).
    uleb128(2, &mut body); // 2 local-decl groups
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I32); // local 1: rep
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I64); // local 2: scratch
    let rep = 1u32;
    let scratch = 2u32;
    // rep = resource.rep(handle=local 0).
    body.push(op::LOCAL_GET);
    uleb128(0, &mut body);
    body.push(op::CALL);
    uleb128(f_rrep as u64, &mut body);
    body.push(op::LOCAL_SET);
    uleb128(rep as u64, &mut body);

    for hole in &template.leaves {
        emit_hole_fill(hole, byte_off, rep, scratch, import_index, &mut body);
    }
    // RELEASE the compound's heap handle: `encode` takes `self: own<t>` (the canonical ABI transfers
    // ownership INTO encode), so encode owns the last reference — call `heap.drop(rep)` to reclaim it
    // (cascades to the boxed children; the value heap is acyclic). This is what balances `make`'s
    // `arr-alloc`, so a runtime compound escape leaves NO live heap cell. (We do NOT rely on the resource
    // DTOR to release it: `encode`'s `own` self is consumed here, and — decisive finding — `resource.rep`
    // on a BORROWED self traps in wasmtime 37, so we cannot switch encode to `borrow` and let the host
    // drop. Owning + dropping in encode is the sound release point for the nullary escape, which always
    // calls encode. [[rcdzc-r1-resource-encode-linking-findings]].)
    body.push(op::LOCAL_GET);
    uleb128(rep as u64, &mut body);
    call_op("drop", &mut body);
    // return the (ptr,len) area.
    body.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(ret_off as i64, &mut body);
    body.push(op::END);
    let mut e = uleb_bytes(body.len() as u64);
    e.extend_from_slice(&body);
    e
}

/// Emit the instructions that WALK to one hole's leaf and WRITE its bytes into the output buffer (at the
/// hole's absolute `offset`). Shared by the flat tuple/record walker and the per-variant sum walker.
/// `rep` is the local holding the root heap handle; `scratch` an i64 scratch local. The walk starts at
/// `rep`, calls `sum-payload` first if the hole is `via_sum_payload` (a sum variant payload leaf), then
/// applies the `arr-get` path; the leaf read + byte writes match `LeafFill`. Ops resolve by name.
fn emit_hole_fill(
    hole: &crate::lower::RuntimeLeaf,
    byte_off: usize,
    rep: u32,
    scratch: u32,
    import_index: &std::collections::HashMap<&str, u32>,
    body: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let call_op = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(import_index[name] as u64, out);
    };
    let store8 = |out: &mut Vec<u8>| {
        out.push(op::I32_STORE8);
        out.push(0x00); // align 0
        out.push(0x00); // offset 0
    };
    // Push `rep`, then descend to the leaf's boxed handle: a sum variant payload is recovered by
    // `sum-payload(rep)` first, then any `arr-get` path (a multi-payload tuple index); a plain
    // tuple/record leaf just walks the `arr-get` path from `rep`.
    let push_walk = |body: &mut Vec<u8>| {
        body.push(op::LOCAL_GET);
        uleb128(rep as u64, body);
        if hole.via_sum_payload {
            call_op("sum-payload", body);
        }
        for &idx in &hole.path {
            body.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(idx as i64, body);
            call_op("arr-get", body);
        }
    };
    // The hole's offset is relative to its template's start; add the template's placement `byte_off`
    // (0 for a flat compound, the variant's data-section offset for a sum).
    let out_off = (hole.offset + byte_off) as u64;
    match hole.kind {
        crate::lower::LeafFill::Int => {
            // scratch = get-int(walk(rep, path)).
            push_walk(body);
            call_op("get-int", body);
            body.push(op::LOCAL_SET);
            uleb128(scratch as u64, body);
            // if scratch < 0 { store NEG_DEC kind at out_off-2; scratch = 0 - scratch }.
            body.push(op::LOCAL_GET);
            uleb128(scratch as u64, body);
            body.push(op::I64_CONST);
            crate::backend::wasm::encode::sleb128(0, body);
            body.push(op::I64_LT_S);
            body.push(op::IF);
            body.push(wasm_abi::BLOCK_EMPTY);
            body.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128((out_off as i64) - 2, body);
            body.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(3, body); // KIND_INT_NEG_DEC
            store8(body);
            body.push(op::I64_CONST);
            crate::backend::wasm::encode::sleb128(0, body);
            body.push(op::LOCAL_GET);
            uleb128(scratch as u64, body);
            body.push(op::I64_SUB);
            body.push(op::LOCAL_SET);
            uleb128(scratch as u64, body);
            body.push(op::END);
            // write 8 big-endian magnitude bytes at out_off.
            for byte in 0..8u64 {
                body.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128((out_off + byte) as i64, body);
                body.push(op::LOCAL_GET);
                uleb128(scratch as u64, body);
                body.push(op::I64_CONST);
                crate::backend::wasm::encode::sleb128((8 * (7 - byte)) as i64, body);
                body.push(op::I64_SHR_U);
                body.push(op::I32_WRAP_I64);
                store8(body);
            }
        }
        crate::lower::LeafFill::Bool => {
            // write kind byte (8 + get-bool) at out_off.
            body.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(out_off as i64, body);
            push_walk(body);
            call_op("get-bool", body);
            body.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(8, body);
            body.push(op::I32_ADD);
            store8(body);
        }
    }
}

/// Where one value-form template sits in the escape core's data section: the offset of its bytes
/// (which double as the output buffer the walker fills) and the offset of its 8-byte `(ptr,len)` return
/// area. A flat compound has one; a sum has one per variant, in discriminant order.
struct Placed {
    byte_off: usize,
    ret_off: usize,
}

/// Flatten the placed templates to `(byte_off, ret_off)` pairs in variant/discriminant order — the
/// layout the sum walker needs (where each variant's bytes + return area sit in the data section).
fn placed_pairs(placed: &[Placed]) -> Vec<(usize, usize)> {
    placed.iter().map(|p| (p.byte_off, p.ret_off)).collect()
}

/// The SUM `t-encode(handle) -> i32` walker. Locals: 0 = resource handle, 1 = i32 rep, 2 = i64 scratch,
/// 3 = i32 discriminant. Recovers the rep, reads `sum-disc(rep)` into `disc`, then an if-chain: for each
/// variant `k`, `if disc == k` fill variant `k`'s holes (each reached through `sum-payload`), `drop` the
/// rep, and return variant `k`'s `(ptr,len)` area (`ret_off`). A trailing `unreachable` closes the chain
/// (the discriminant is always one of the closed variant set). Each variant's holes are written at its
/// own `byte_off` region (its template bytes double as that region's output buffer).
fn encode_sum_walk_body(
    variants: &[crate::lower::ValueFormTemplate],
    placed: &[(usize, usize)],
    f_rrep: u32,
    import_index: &std::collections::HashMap<&str, u32>,
) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let call_op = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(import_index[name] as u64, out);
    };
    let mut body = Vec::new();
    // Locals: i32 rep, i64 scratch, i32 disc — 3 groups (i32, i64, i32).
    uleb128(3, &mut body);
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I32); // local 1: rep
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I64); // local 2: scratch
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I32); // local 3: disc
    let rep = 1u32;
    let scratch = 2u32;
    let disc = 3u32;
    // rep = resource.rep(handle).
    body.push(op::LOCAL_GET);
    uleb128(0, &mut body);
    body.push(op::CALL);
    uleb128(f_rrep as u64, &mut body);
    body.push(op::LOCAL_SET);
    uleb128(rep as u64, &mut body);
    // disc = sum-disc(rep).
    body.push(op::LOCAL_GET);
    uleb128(rep as u64, &mut body);
    call_op("sum-disc", &mut body);
    body.push(op::LOCAL_SET);
    uleb128(disc as u64, &mut body);
    // For each variant: if disc == k { fill; drop; return ret_off }.
    for (k, (tpl, (byte_off, ret_off))) in variants.iter().zip(placed).enumerate() {
        // disc == k ?
        body.push(op::LOCAL_GET);
        uleb128(disc as u64, &mut body);
        body.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(k as i64, &mut body);
        body.push(op::I32_EQ);
        // if (empty) { fill; drop; return ptr } — an EMPTY-result block: the true path RETURNs from the
        // function (so it yields nothing to the block), and the false path falls through to the next
        // variant's test. (A typed `(result i32)` if would require the false path to also yield an i32,
        // which it does not — control flows to the next arm.)
        body.push(op::IF);
        body.push(wasm_abi::BLOCK_EMPTY);
        for hole in &tpl.leaves {
            emit_hole_fill(hole, *byte_off, rep, scratch, import_index, &mut body);
        }
        // drop the rep (encode owns it), then return this variant's ret area pointer.
        body.push(op::LOCAL_GET);
        uleb128(rep as u64, &mut body);
        call_op("drop", &mut body);
        body.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(*ret_off as i64, &mut body);
        body.push(op::RETURN);
        body.push(op::END); // end if
    }
    // The discriminant is always one of the closed variant set, so the chain is total; a fall-through is
    // impossible. Emit `unreachable` to satisfy the validator (the function must yield an i32).
    body.push(op::UNREACHABLE);
    body.push(op::END);
    let mut e = uleb_bytes(body.len() as u64);
    e.extend_from_slice(&body);
    e
}

/// The component-boundary valtype of an export's result (`None` = unit / no result) — read by the
/// envelope assembler for the component functype. A type with no boundary representation is an error
/// here: a NON-ALIASED integer width (`(UInt 48)`, …) is internal-only, so an export whose result or
/// parameter is one DECLINES (naming the width) rather than crossing as a misreported wider primitive.
pub fn export_result_valtype(ret: &Ty) -> Result<Option<u8>, String> {
    match ret {
        Ty::Unit => Ok(None),
        // A COMPOUND returned across the HOST boundary crosses as the canonical binary value form via
        // the RESOURCE-ESCAPE path (a single nullary compound export → a resource whose `encode()`
        // yields the value form; see `wasm::emit`). That path is detected before selection and does not
        // come through this function; a compound reaching HERE is a multi-export or parameterized
        // return, which the resource shape does not cover, so it DECLINES — the internal handle
        // representation (`comp_valtype_of` → u32) is right for a compound CONSUMED internally but
        // handing the host a raw handle would misreport the value. Reject-don't-miscompile, not a leak.
        Ty::Tuple(_) | Ty::Record(_) => Err(format!(
            "returning a {} on the multi-export boundary is not supported (use a single compound export, which escapes as a resource)",
            ret.render_name()
        )),
        // A sum crosses ONLY via the single-nullary-export escape path (handled in `emit` before this is
        // reached); a sum in any OTHER boundary position — a multi-export result, a parameter — has no
        // scalar boundary form and declines here (the escape's disc-switch renderer applies only to the
        // lone-export case).
        Ty::Sum { .. } => Err(format!(
            "a `{}` sum crosses the host boundary only as a single nullary export's result",
            ret.render_name()
        )),
        other => match comp_valtype_of(other) {
            Some(b) => Ok(Some(b)),
            None => Err(format!(
                "type `{}` has no component boundary representation (only the aliased integer widths \
                 8/16/32/64 cross the boundary)",
                other.render_name()
            )),
        },
    }
}

/// An export's RESULT as the envelope needs it — the same mapping as [`export_result_valtype`] lifted
/// into the [`BoundaryResult`] the assembler consumes. Unit → `None`; a scalar → its primitive byte. A
/// COMPOUND crosses as the canonical binary value form (`BoundaryResult::Bytes`) only on the
/// resource-escape path (a single nullary compound export, handled in `wasm::emit`), which does not go
/// through this function; a compound reaching HERE is a multi-export/parameterized return, which
/// declines (see [`export_result_valtype`]). The `Bytes` variant is produced by that escape path and
/// exercised by the R0 envelope oracle + wasmtime tests, which hand-build a `list<u8>`-returning core.
pub fn export_result(ret: &Ty) -> Result<crate::backend::wasm::envelope::BoundaryResult, String> {
    use crate::backend::wasm::envelope::BoundaryResult;
    match export_result_valtype(ret) {
        Ok(None) => Ok(BoundaryResult::None),
        Ok(Some(b)) => Ok(BoundaryResult::Primitive(b)),
        Err(e) => Err(e),
    }
}

/// Run-length encode a slot-valtype vector into `(count, valtype)` groups (wasm local-decl form).
fn rle(valtypes: &[ValType]) -> Vec<(u32, ValType)> {
    let mut groups: Vec<(u32, ValType)> = Vec::new();
    for &vt in valtypes {
        match groups.last_mut() {
            Some((count, prev)) if *prev == vt => *count += 1,
            _ => groups.push((1, vt)),
        }
    }
    groups
}
