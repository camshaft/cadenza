//! `serialize` — the flat Lir of a module's functions laid into a core wasm module's bytes.
//!
//! The ONLY place that produces core-module bytes; it decides nothing (calls are already absolute,
//! the boundary is already fixed by the layout). A module of N functions becomes a core module with
//! four sections — type, function, export, code — its functions emitted in the layout's emission
//! order, so a body at emission position `k` is core func `k` (`reference-compiler.md` §Emission
//! Serializes A Lowered Representation). The export section names EVERY boundary function by its
//! verbatim source name (multi-export from the start — no single hard-coded `run`).

use crate::backend::wasm::encode::{op, section, uleb128, uleb_bytes, wasm_vec};
use crate::backend::wasm::lir::{comp_valtype_of, valtype_of, Lir, ValType};
use crate::backend::wasm::select::SelectedFunc;
use crate::layout::Layout;
use crate::ty::Ty;

/// The `\0asm` version-1 core-module preamble.
const CORE_MAGIC: &[u8] = &[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];

/// Serialize one flat instruction, appending its bytes to `out`. Exhaustive over `Lir`.
fn instr(i: &Lir, out: &mut Vec<u8>) {
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
        Lir::If(bt) => {
            out.push(op::IF);
            out.push(bt.byte()); // block-type byte lives here, not in the IR
        }
        Lir::Else => out.push(op::ELSE),
        Lir::End => out.push(op::END),
    }
}

/// One function's code-section entry: `<size> <local-decls> <instrs> end`.
fn code_entry(f: &SelectedFunc) -> Vec<u8> {
    let mut inner = Vec::new();
    // Local declarations, run-length-encoded by value type (Stage 0 bodies declare none → count 0).
    let groups = rle(&f.declared);
    uleb128(groups.len() as u64, &mut inner);
    for (count, vt) in groups {
        uleb128(count as u64, &mut inner);
        inner.push(vt.byte());
    }
    for i in &f.code {
        instr(i, &mut inner);
    }
    inner.push(op::END);
    let mut out = uleb_bytes(inner.len() as u64);
    out.extend_from_slice(&inner);
    out
}

/// One function's core functype: `0x60 <params-vec> <results-vec>`. A unit return is a zero-result
/// function; any other return is one result of its value type.
fn functype(f: &SelectedFunc) -> Result<Vec<u8>, String> {
    let mut out = vec![0x60];
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
/// emission position `k` (already in the layout's order), exported under `export_names[k]` when it is
/// a boundary function. The export section names every boundary function by its absolute core index.
pub fn core_module(
    funcs: &[SelectedFunc],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    let n = funcs.len();

    // Type section: one functype per function, in emission order.
    let mut type_items = Vec::new();
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let type_sec = section(1, &wasm_vec(n, &type_items));

    // Function section: func i uses type i (1:1).
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128(i as u64, &mut func_items);
    }
    let func_sec = section(3, &wasm_vec(n, &func_items));

    // Export section: export every boundary function under its verbatim name, by its absolute core
    // index (its position in the layout's emission order).
    let mut export_items = Vec::new();
    for e in &layout.exports {
        let abs = layout.abs(e.def).ok_or_else(|| {
            format!("exported definition `{}` is not in the emission order", e.name)
        })?;
        let mut item = uleb_bytes(e.name.len() as u64);
        item.extend_from_slice(e.name.as_bytes());
        item.push(0x00); // export kind: func
        uleb128(abs as u64, &mut item);
        export_items.extend_from_slice(&item);
    }
    let export_sec = section(7, &wasm_vec(layout.exports.len(), &export_items));

    // Code section: bodies in emission order.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f));
    }
    let code_sec = section(10, &wasm_vec(n, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// The component-boundary valtype of an export's result (`None` = unit / no result) — read by the
/// envelope assembler for the component functype. A signature the boundary cannot represent is an
/// error here (Stage 0 exports are scalar).
pub fn export_result_valtype(ret: &Ty) -> Result<Option<u8>, String> {
    match ret {
        Ty::Unit => Ok(None),
        other => match comp_valtype_of(other) {
            Some(b) => Ok(Some(b)),
            None => Err("export result type has no component valtype".to_string()),
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
