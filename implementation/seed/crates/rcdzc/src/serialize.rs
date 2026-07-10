//! `serialize : Lir → bytes` + framing — the exhaustive backend serializer, module-wide.
//!
//! Mirrors `cdzc/40-frame.cdz` + `50-compile.cdz`. The only place that produces bytes; it decides
//! NOTHING. `serialize_instr` maps each `Lir` to bytes by an EXHAUSTIVE match. A module of N functions
//! becomes a core module (type + func + export + code sections) whose functions are emitted in the
//! layout's emission order, then the N-export component envelope is assembled around it by
//! [`crate::component`].
//!
//! Two things the layout fixes upstream, so this pass is pure byte-laying:
//!  - CALLS ARE ABSOLUTE. `select` already resolved every `Lir::Call` to its final wasm index via the
//!    layout, so there is NO call remap here.
//!  - THE BOUNDARY IS MULTI-EXPORT. The core module exports EVERY boundary function (by name), and the
//!    component envelope lifts each — no single hard-coded `run`. At N=1 with a nullary-scalar entry
//!    the bytes are identical to the old compiler (validated by the `frame`-anchored tests).

use crate::component::{self, ExportForCore};
use crate::ir::{Lir, ValType};
use crate::layout::Layout;
use crate::op;
use crate::select::{SelectedFunc, SelectedModule};
use crate::ty::Ty;
use crate::wasm::{section, sleb128, uleb128, uleb_bytes, wasm_vec};

use crate::frame::FRAME_CORE_MAGIC;

/// Serialize one flat `Lir` instruction. Exhaustive over `Lir`.
fn serialize_instr(instr: &Lir, out: &mut Vec<u8>) {
    match instr {
        Lir::ConstI64(n) => {
            out.push(op::I64_CONST);
            sleb128(*n, out);
        }
        Lir::ConstI32(n) => {
            out.push(op::I32_CONST);
            sleb128(*n as i64, out);
        }
        Lir::LocalGet(i) => {
            out.push(op::LOCAL_GET);
            uleb128(*i as u64, out);
        }
        Lir::LocalSet(i) => {
            out.push(op::LOCAL_SET);
            uleb128(*i as u64, out);
        }
        Lir::LocalTee(i) => {
            out.push(op::LOCAL_TEE);
            uleb128(*i as u64, out);
        }
        Lir::Call(i) => {
            out.push(op::CALL);
            uleb128(*i as u64, out);
        }
        Lir::Drop => out.push(op::DROP),
        Lir::I32Add => out.push(op::I32_ADD),
        Lir::I32And => out.push(0x71), // i32.and — not in the curated `op` table; the raw opcode byte.
        Lir::I32Sub => out.push(op::I32_SUB),
        Lir::I32Store => out.extend_from_slice(&[op::I32_STORE, 0x02, 0x00]), // align=2, offset=0
        Lir::I32Store8 => out.extend_from_slice(&[op::I32_STORE8, 0x00, 0x00]), // align=0, offset=0
        Lir::I64Add => out.push(op::I64_ADD),
        Lir::I64Sub => out.push(op::I64_SUB),
        Lir::I64Mul => out.push(op::I64_MUL),
        Lir::I64DivS => out.push(op::I64_DIV_S),
        Lir::I64Xor => out.push(op::I64_XOR),
        Lir::I64And => out.push(op::I64_AND),
        Lir::I64Or => out.push(op::I64_OR),
        Lir::I64RemS => out.push(op::I64_REM_S),
        Lir::I64Shl => out.push(op::I64_SHL),
        Lir::I64ShrS => out.push(op::I64_SHR_S),
        Lir::I64GeU => out.push(op::I64_GE_U),
        Lir::I64Eqz => out.push(op::I64_EQZ),
        Lir::I64Eq => out.push(op::I64_EQ),
        Lir::I64Ne => out.push(op::I64_NE),
        Lir::I64LtS => out.push(op::I64_LT_S),
        Lir::I64GtS => out.push(op::I64_GT_S),
        Lir::I64LeS => out.push(op::I64_LE_S),
        Lir::I64GeS => out.push(op::I64_GE_S),
        Lir::I32Eqz => out.push(op::I32_EQZ),
        // `i32.wrap_i64` = 0xA7, `i64.extend_i32_u` = 0xAD — not in the curated `op` table (the Bytes
        // path narrows a range-checked byte to i32 for `bytes-set`, and widens a `bytes-len` count to
        // the Int64 result); the raw opcode bytes, like render.rs's local `i32.ge_s`.
        Lir::I32WrapI64 => out.push(0xA7),
        Lir::I64ExtendI32U => out.push(0xAD),
        Lir::I32Eq => out.push(op::I32_EQ),
        Lir::I32LtU => out.push(op::I32_LT_U),
        Lir::I32GtU => out.push(op::I32_GT_U),
        Lir::I32LeU => out.push(op::I32_LE_U),
        Lir::I32GeU => out.push(op::I32_GE_U),
        Lir::If(blocktype) => {
            out.push(op::IF);
            out.push(blocktype.byte()); // block-type encoding lives here, not in the IR
        }
        Lir::Else => out.push(op::ELSE),
        Lir::End => out.push(op::END),
        Lir::Block(blocktype) => {
            out.push(op::BLOCK);
            out.push(blocktype.byte());
        }
        Lir::Loop(blocktype) => {
            out.push(op::LOOP);
            out.push(blocktype.byte());
        }
        Lir::Br(depth) => {
            out.push(op::BR);
            uleb128(*depth as u64, out);
        }
        Lir::BrIf(depth) => {
            out.push(op::BR_IF);
            uleb128(*depth as u64, out);
        }
        Lir::Unreachable => out.push(op::UNREACHABLE),
    }
}

/// One function's code-section entry: `<size> <local-decls> <instrs> end`. Calls are already absolute
/// (the layout resolved them in `select`), so there is no remap. Public so the runtime-compound
/// (heap) assembler reuses the SAME body serializer — one serializer, no drift.
pub fn serialize_body(f: &SelectedFunc) -> Vec<u8> {
    let mut inner = Vec::new();
    // Local declarations (RLE by value type), each group's type encoded to its byte here.
    let groups = rle(&f.declared);
    uleb128(groups.len() as u64, &mut inner);
    for (count, valtype) in groups {
        uleb128(count as u64, &mut inner);
        inner.push(valtype.byte());
    }
    for instr in &f.code {
        serialize_instr(instr, &mut inner);
    }
    inner.push(op::END);
    let mut out = uleb_bytes(inner.len() as u64);
    out.extend_from_slice(&inner);
    out
}

/// One function's functype: `0x60 <params> <results>`.
fn functype(f: &SelectedFunc) -> Result<Vec<u8>, String> {
    let mut out = vec![0x60];
    let param_bytes: Vec<u8> = f.params.iter().map(|vt| vt.byte()).collect();
    out.extend_from_slice(&wasm_vec(param_bytes.len(), &param_bytes));
    // Results: a Unit return has no value slot; otherwise one result of the return's value type.
    match f.ret.core_valtype() {
        Some(vt) => out.extend_from_slice(&wasm_vec(1, &[vt.byte()])),
        None if matches!(f.ret, Ty::Unit) => out.extend_from_slice(&wasm_vec(0, &[])),
        None => return Err("function return type unresolved".to_string()),
    }
    Ok(out)
}

/// Assemble the complete SCALAR component for a selected module: build the embedded core module
/// (functions in the layout's emission order, every boundary function exported by name), then wrap it
/// in the N-export component envelope. Every export is presented by its signature ABI — no single
/// hard-coded `run`.
pub fn component(module: &SelectedModule, layout: &Layout) -> Result<Vec<u8>, String> {
    let core = core_module(module, layout)?;
    let exports: Vec<ExportForCore> = layout
        .exports
        .iter()
        .map(ExportForCore::from_plan)
        .collect::<Result<Vec<_>, String>>()?;
    component::assemble(&core, &exports)
}

/// Assemble the RUNTIME-COMPOUND component for a module that touches the value heap — either its entry
/// returns a compound (`ret`), or it merely constructs/projects one internally while returning a
/// scalar. The program imports the value-heap runtime, builds on the heap, and RENDERS its result to a
/// string in-program (the tag-free runtime cannot render — the compiler emits a type-directed
/// renderer). It crosses the boundary as a `string` the `run` export returns.
///
/// User functions are emitted in `layout.order` starting at `RT_FUNC_BASE` (past the 42 heap imports +
/// 3 fixed helpers), ENTRY FIRST — the renderer's `run` calls user func 0 (= `RT_FUNC_BASE`) and walks
/// its result. `ret` is the ENTRY's return type (scalar or compound); the renderer handles both. Phase
/// 3a renders a single entry (`order[0]`).
pub fn runtime_compound_component(
    module: &SelectedModule,
    layout: &Layout,
    ret: &Ty,
) -> Result<Vec<u8>, String> {
    // Serialize each user function body (entry first) with the SAME serializer the scalar path uses.
    let user_code: Vec<Vec<u8>> =
        layout.order.iter().map(|&i| serialize_body(&module.funcs[i])).collect();
    let funcs: Vec<&SelectedFunc> = layout.order.iter().map(|&i| &module.funcs[i]).collect();
    // Build the type-directed renderer for the entry's return type: the per-type render fn bodies plus
    // the `run : () -> i32` body (render fns are indexed after the user funcs).
    let (render_bodies, run_body) = crate::render::build(ret, funcs.len())?;
    Ok(crate::heap::component(&funcs, &user_code, &render_bodies, &run_body))
}

/// The embedded core module: `\0asm v1` + type + func + export + code sections. Functions are emitted
/// in `layout.order` (boundary functions first), so a body at emission position `k` is core func `k`
/// — exactly the absolute indices `select` resolved and the component aliases reference. The export
/// section names every boundary function so the component's core aliases resolve.
fn core_module(module: &SelectedModule, layout: &Layout) -> Result<Vec<u8>, String> {
    let n = layout.order.len();

    // Type section: one functype per function, in emission order.
    let mut type_items = Vec::new();
    for &orig in &layout.order {
        type_items.extend_from_slice(&functype(&module.funcs[orig])?);
    }
    let type_sec = section(1, &wasm_vec(n, &type_items));

    // Function section: func i uses type i (1:1).
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128(i as u64, &mut func_items);
    }
    let func_sec = section(3, &wasm_vec(n, &func_items));

    // Export section: export every boundary function under its boundary name, by its absolute core
    // index (`layout.abs[func]`). Presented in declaration order.
    let mut export_items = Vec::new();
    for e in &layout.exports {
        let mut item = uleb_bytes(e.name.len() as u64);
        item.extend_from_slice(e.name.as_bytes());
        item.push(0x00); // export kind: func
        uleb128(layout.abs[e.func] as u64, &mut item);
        export_items.extend_from_slice(&item);
    }
    let export_sec = section(7, &wasm_vec(layout.exports.len(), &export_items));

    // Code section: bodies in emission order.
    let mut code_items = Vec::new();
    for &orig in &layout.order {
        code_items.extend_from_slice(&serialize_body(&module.funcs[orig]));
    }
    let code_sec = section(10, &wasm_vec(n, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(FRAME_CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

// ─── local helpers ───────────────────────────────────────────────────────────────────

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
