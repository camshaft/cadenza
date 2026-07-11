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
        Lir::Call(func) => {
            out.push(op::CALL);
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
        Lir::Else => out.push(op::ELSE),
        Lir::End => out.push(op::END),
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
    // Local declarations, run-length-encoded by value type (Stage 0 bodies declare none → count 0).
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

/// Assemble the embedded core module for a module's selected functions. `funcs[k]` is the function at
/// emission position `k` (already in the layout's order). `imports` is the program's per-program set of
/// runtime ops (ordered — the same order `layout` numbered them), imported from module `"heap"` at core
/// func indices `0..imports.len()`; the program's own DEFINED functions therefore start at core index
/// `imports.len()`, and the export section (and every `Lir::Call`, via `layout.abs`) account for that
/// shift. An empty `imports` emits no import section and no shift — byte-identical to a runtime-free
/// program (`component-abi.md` v3 migration: a program importing nothing crosses as under v2).
pub fn core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    layout: &Layout,
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
    Ok(core)
}

/// The component-boundary valtype of an export's result (`None` = unit / no result) — read by the
/// envelope assembler for the component functype. A type with no boundary representation is an error
/// here: a NON-ALIASED integer width (`(UInt 48)`, …) is internal-only, so an export whose result or
/// parameter is one DECLINES (naming the width) rather than crossing as a misreported wider primitive.
pub fn export_result_valtype(ret: &Ty) -> Result<Option<u8>, String> {
    match ret {
        Ty::Unit => Ok(None),
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
