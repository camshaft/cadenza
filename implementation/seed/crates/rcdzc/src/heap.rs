//! The runtime-compound component — the envelope for a program whose result is a value-heap value
//! (a tuple, later a record/sum/list/string). Such a program imports `cadenza:runtime/heap`,
//! constructs its result on the value heap, and RENDERS it to a string in-program (the tag-free
//! runtime cannot render — the compiler emits a type-directed renderer). It crosses the boundary as
//! an ordinary `string` the `run` export returns.
//!
//! This mirrors the old compiler's `runtime_compound_component` (codegen.rs) but is driven by the
//! solved `Ty` (via `render`), not the old `Shape` re-derivation. It REUSES the generated envelope
//! byte constants from `heap_envelope` (RT_HEAD/RT_TAIL/RT_IMPORT_CONTENT/RT_MEM/RT_GLOBAL/
//! rt_import_types/RT_N_IMPORTS) — one derivation, no drift. The three fixed helper bodies
//! (cabi_realloc / putu / itoa) are fixed hand-authored wasm, reproduced here verbatim (the
//! runtime-compound analog of the `frame` constants); everything else is derived.
//!
//! Core-module function index layout (what `render` and the call sites assume):
//!   `[0, RT_N_IMPORTS)`      the heap imports (42), by their himport index
//!   `RT_N_IMPORTS + 0/1/2`   cabi_realloc / putu / itoa   (the 3 fixed helpers)
//!   `RT_FUNC_BASE + i`       user function i  (the entry/main = user func 0 = RT_FUNC_BASE)
//!   after the user funcs      the per-type render fns
//!   last                      `run : () -> i32`

// The runtime-compound (heap-returning) path is DRAFTED but not yet wired into the pipeline (it
// lands with the tuple slice, on top of the multi-export foundation). Everything here is staged for
// that slice; suppress the interim dead-code noise wholesale rather than scatter per-item allows.
#![allow(dead_code)]

use crate::heap_envelope::{
    himport, rt_import_types, RT_GLOBAL, RT_HEAD, RT_IMPORT_CONTENT, RT_MEM, RT_N_IMPORTS, RT_TAIL,
};
use crate::op;
use crate::select::SelectedFunc;
use crate::wasm::{section, sleb128, uleb128, uleb_bytes, wasm_vec};

/// The number of fixed helper funcs (cabi_realloc, putu, itoa, utf8-valid) that precede the user
/// functions.
pub const RT_FIXED_FUNCS: u32 = 4;
/// The wasm index of the first user function (main = user 0 = `RT_FUNC_BASE`).
pub const RT_FUNC_BASE: u32 = RT_N_IMPORTS + RT_FIXED_FUNCS;
/// The fixed helpers' indices.
pub const RT_REALLOC: u32 = RT_N_IMPORTS;
pub const RT_PUTU: u32 = RT_N_IMPORTS + 1;
pub const RT_ITOA: u32 = RT_N_IMPORTS + 2;
/// `utf8-valid(buf: i32) -> i32` — 1 if the Bytes leaf in `buf` is well-formed UTF-8, else 0. The
/// fixed helper the `String.from-bytes` decode calls (a byte loop the flat `Lir` cannot express — the
/// `putu`/`itoa` precedent for control flow in raw wasm).
pub const RT_UTF8_VALID: u32 = RT_N_IMPORTS + 3;

/// `cabi_realloc(orig, old_size, ALIGN=2, new_size=3) -> ptr`: a bump allocator over global 0 that
/// honours alignment (rounds the bump pointer up to `align`, a power of two, then advances by
/// `new_size`). The align argument is param index **2** (the canonical component-ABI order). Body
/// bytes reproduced verbatim from the old compiler's `rt_realloc_body`.
fn realloc_body() -> Vec<u8> {
    const I32_AND: u8 = 0x71;
    // 1 extra i32 local (index 4 = the ret).
    let mut inner = vec![0x01, 0x01, 0x7F]; // local decls: 1 group of 1 i32
    inner.extend_from_slice(&[
        // ret(4) = (global0 + align - 1) & -align
        op::GLOBAL_GET, 0, op::LOCAL_GET, 2, op::I32_ADD, op::I32_CONST, 1, op::I32_SUB,
        op::I32_CONST, 0, op::LOCAL_GET, 2, op::I32_SUB,
        I32_AND,
        op::LOCAL_SET, 4,
        // global0 = ret + new_size
        op::LOCAL_GET, 4, op::LOCAL_GET, 3, op::I32_ADD, op::GLOBAL_SET, 0,
        op::LOCAL_GET, 4,
        op::END,
    ]);
    sized(inner)
}

/// `putu(v: u64, cursor: i32) -> i32`: write `v` as unsigned decimal at `cursor`, return the cursor
/// past the last digit. Recursive (high digits first). Verbatim from `rt_putu_body`.
fn putu_body() -> Vec<u8> {
    const I64_GT_U: u8 = 0x56;
    const I64_DIV_U: u8 = 0x80;
    const I64_REM_U: u8 = 0x82;
    const I32_WRAP_I64: u8 = 0xA7;
    let mut inner = vec![0x00]; // no extra locals
    // if v > 9 { cursor = putu(v/10, cursor) }
    inner.extend_from_slice(&[op::LOCAL_GET, 0, op::I64_CONST, 9, I64_GT_U, op::IF, 0x40]);
    inner.extend_from_slice(&[op::LOCAL_GET, 0, op::I64_CONST, 10, I64_DIV_U, op::LOCAL_GET, 1, op::CALL]);
    uleb128(RT_PUTU as u64, &mut inner);
    inner.extend_from_slice(&[op::LOCAL_SET, 1, op::END]);
    // cursor[0] = '0' + (v % 10) ; return cursor + 1
    inner.extend_from_slice(&[
        op::LOCAL_GET, 1, op::I32_CONST, 48, op::LOCAL_GET, 0, op::I64_CONST, 10, I64_REM_U,
        I32_WRAP_I64, op::I32_ADD, op::I32_STORE8, 0, 0,
    ]);
    inner.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST, 1, op::I32_ADD, op::END]);
    sized(inner)
}

/// `itoa(v: i64, cursor: i32) -> i32`: signed decimal (leading `-`, then magnitude via putu).
/// One i64 local (index 2). Verbatim from `rt_itoa_body`.
fn itoa_body() -> Vec<u8> {
    let mut inner = vec![0x01, 0x01, 0x7E]; // 1 i64 local (index 2)
    inner.extend_from_slice(&[op::LOCAL_GET, 0, op::I64_CONST, 0, op::I64_LT_S, op::IF, 0x40]);
    inner.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST, 45, op::I32_STORE8, 0, 0]);
    inner.extend_from_slice(&[op::LOCAL_GET, 1, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, 1]);
    inner.extend_from_slice(&[op::I64_CONST, 0, op::LOCAL_GET, 0, op::I64_SUB, op::LOCAL_SET, 2]);
    inner.extend_from_slice(&[op::ELSE, op::LOCAL_GET, 0, op::LOCAL_SET, 2, op::END]);
    inner.extend_from_slice(&[op::LOCAL_GET, 2, op::LOCAL_GET, 1, op::CALL]);
    uleb128(RT_PUTU as u64, &mut inner);
    inner.push(op::END);
    sized(inner)
}

/// `utf8-valid(buf: i32) -> i32`: 1 if the Bytes leaf `buf` is well-formed UTF-8, else 0. A faithful
/// port of the old compiler's inline `emit_utf8_valid` (codegen.rs) into a standalone fixed helper —
/// the `putu`/`itoa` precedent for control flow (a byte loop) the flat `Lir` cannot express. Rejects
/// invalid leads, OVERLONG encodings, SURROGATES (U+D800..=U+DFFF), and code points > U+10FFFF, per
/// the Unicode UTF-8 definition (matching `str::from_utf8`, so the runtime decode agrees with the
/// const-fold path). Emitted over the frozen `bytes-len`/`bytes-get` imports — no runtime/envelope change.
///
/// Locals: `buf`=param 0; the scratch (all i32) are `n`=1 (scan index), `len`=2 (byte length),
/// `lead`=3 (current lead byte), `seq`=4 (continuation count 1/2/3), `k`=5 (continuation counter),
/// `cb`=6 (current continuation byte), `lo`=7/`hi`=8 (legal FIRST-continuation range), `valid`=9
/// (running validity). A failure just sets `valid=0`; both loops are GUARDED by `valid` so they run to
/// a clean finish (no multi-level `br` — the depth bookkeeping that makes a hand-emitted validator wrong).
fn utf8_valid_body() -> Vec<u8> {
    const AND: u8 = 0x71; // i32.and
    const OR: u8 = 0x72; // i32.or
    const EQ: u8 = 0x46; // i32.eq
    const NE: u8 = 0x47; // i32.ne (unused here but kept for parity)
    const GE_U: u8 = 0x4F;
    const LE_U: u8 = 0x4D;
    const GT_U: u8 = 0x4B;
    const LT_U: u8 = 0x49;
    const EQZ: u8 = 0x45;
    let _ = NE;
    // Local indices (buf is param 0).
    let (buf, n, len, lead, seq, k, cb, lo, hi, valid) = (0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8);
    // 9 i32 locals (indices 1..=9).
    let mut c = vec![0x01, 0x09, 0x7F];
    let konst = |c: &mut Vec<u8>, val: i64| {
        c.push(op::I32_CONST);
        sleb128(val, c);
    };
    let get = |c: &mut Vec<u8>, idx_body: &dyn Fn(&mut Vec<u8>)| {
        c.extend_from_slice(&[op::LOCAL_GET, buf]);
        idx_body(c);
        c.push(op::CALL);
        uleb128(himport::BYTES_GET as u64, c);
    };
    // A range test `(x >= lo) & (x <= hi)`, x from `xget`.
    let in_range = |c: &mut Vec<u8>, xget: &dyn Fn(&mut Vec<u8>), lo_v: i64, hi_v: i64| {
        xget(c);
        konst(c, lo_v);
        c.push(GE_U);
        xget(c);
        konst(c, hi_v);
        c.push(LE_U);
        c.push(AND);
    };
    let lead_get = |c: &mut Vec<u8>| c.extend_from_slice(&[op::LOCAL_GET, lead]);
    let cb_get = |c: &mut Vec<u8>| c.extend_from_slice(&[op::LOCAL_GET, cb]);

    // valid = 1 ; n = 0 ; len = bytes-len(buf)
    c.extend_from_slice(&[op::I32_CONST, 1, op::LOCAL_SET, valid]);
    c.extend_from_slice(&[op::I32_CONST, 0, op::LOCAL_SET, n]);
    c.extend_from_slice(&[op::LOCAL_GET, buf, op::CALL]);
    uleb128(himport::BYTES_LEN as u64, &mut c);
    c.extend_from_slice(&[op::LOCAL_SET, len]);

    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    // continue only while n < len AND valid; else exit (br 1).
    c.extend_from_slice(&[op::LOCAL_GET, n, op::LOCAL_GET, len, LT_U]);
    c.extend_from_slice(&[op::LOCAL_GET, valid, AND, EQZ, op::BR_IF, 1]);
    // lead = get(n)
    get(&mut c, &|c| c.extend_from_slice(&[op::LOCAL_GET, n]));
    c.extend_from_slice(&[op::LOCAL_SET, lead]);
    // seq = lead<=0x7F ? 0 : lead>=0xF0 ? 3 : lead>=0xE0 ? 2 : 1
    lead_get(&mut c);
    konst(&mut c, 0x7f);
    c.push(LE_U);
    c.extend_from_slice(&[op::IF, 0x7F, op::I32_CONST, 0, op::ELSE]);
    lead_get(&mut c);
    konst(&mut c, 0xf0);
    c.push(GE_U);
    c.extend_from_slice(&[op::IF, 0x7F, op::I32_CONST, 3, op::ELSE]);
    lead_get(&mut c);
    konst(&mut c, 0xe0);
    c.push(GE_U);
    c.extend_from_slice(&[op::IF, 0x7F, op::I32_CONST, 2, op::ELSE, op::I32_CONST, 1, op::END, op::END, op::END]);
    c.extend_from_slice(&[op::LOCAL_SET, seq]);
    // Default first-continuation range 0x80..0xBF; narrow for special leads.
    konst(&mut c, 0x80);
    c.extend_from_slice(&[op::LOCAL_SET, lo]);
    konst(&mut c, 0xbf);
    c.extend_from_slice(&[op::LOCAL_SET, hi]);
    let mut narrow = |c: &mut Vec<u8>, leadval: i64, lo_v: i64, hi_v: i64| {
        lead_get(c);
        konst(c, leadval);
        c.extend_from_slice(&[EQ, op::IF, 0x40]);
        konst(c, lo_v);
        c.extend_from_slice(&[op::LOCAL_SET, lo]);
        konst(c, hi_v);
        c.extend_from_slice(&[op::LOCAL_SET, hi]);
        c.push(op::END);
    };
    narrow(&mut c, 0xe0, 0xa0, 0xbf);
    narrow(&mut c, 0xed, 0x80, 0x9f);
    narrow(&mut c, 0xf0, 0x90, 0xbf);
    narrow(&mut c, 0xf4, 0x80, 0x8f);
    // valid = valid & lead-not-invalid & enough-bytes
    //   lead-not-invalid = !((0x80<=lead<=0xC1) | (0xF5<=lead<=0xFF))
    //   enough-bytes     = (n + seq) < len
    c.extend_from_slice(&[op::LOCAL_GET, valid]);
    in_range(&mut c, &lead_get, 0x80, 0xc1);
    in_range(&mut c, &lead_get, 0xf5, 0xff);
    c.push(OR);
    c.push(EQZ); // not-invalid
    c.push(AND);
    c.extend_from_slice(&[op::LOCAL_GET, n, op::LOCAL_GET, seq, op::I32_ADD]);
    c.extend_from_slice(&[op::LOCAL_GET, len, LT_U, AND]);
    c.extend_from_slice(&[op::LOCAL_SET, valid]);
    // Check continuations only if still valid.
    c.extend_from_slice(&[op::LOCAL_GET, valid, op::IF, 0x40]);
    c.extend_from_slice(&[op::I32_CONST, 1, op::LOCAL_SET, k]);
    c.extend_from_slice(&[op::BLOCK, 0x40, op::LOOP, 0x40]);
    // continue inner while k <= seq AND valid. (k > seq | !valid) → exit.
    c.extend_from_slice(&[op::LOCAL_GET, k, op::LOCAL_GET, seq, GT_U]);
    c.extend_from_slice(&[op::LOCAL_GET, valid, EQZ, OR, op::BR_IF, 1]);
    // cb = get(n + k)
    get(&mut c, &|c| c.extend_from_slice(&[op::LOCAL_GET, n, op::LOCAL_GET, k, op::I32_ADD]));
    c.extend_from_slice(&[op::LOCAL_SET, cb]);
    // cbok = (k==1) ? cb in [lo,hi] : cb in [0x80,0xBF]
    c.extend_from_slice(&[op::LOCAL_GET, k, op::I32_CONST, 1, EQ, op::IF, 0x7F]);
    // cb in [lo,hi] — lo/hi are locals, not constants.
    cb_get(&mut c);
    c.extend_from_slice(&[op::LOCAL_GET, lo, GE_U]);
    cb_get(&mut c);
    c.extend_from_slice(&[op::LOCAL_GET, hi, LE_U, AND]);
    c.push(op::ELSE);
    in_range(&mut c, &cb_get, 0x80, 0xbf);
    c.push(op::END);
    // valid = valid & cbok
    c.extend_from_slice(&[op::LOCAL_GET, valid, AND, op::LOCAL_SET, valid]);
    // k += 1 ; loop
    c.extend_from_slice(&[op::LOCAL_GET, k, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_SET, k, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // inner loop, inner block
    c.push(op::END); // if valid (continuation check)
    // n += 1 + seq ; loop
    c.extend_from_slice(&[op::LOCAL_GET, n, op::I32_CONST, 1, op::I32_ADD, op::LOCAL_GET, seq, op::I32_ADD, op::LOCAL_SET, n, op::BR, 0]);
    c.extend_from_slice(&[op::END, op::END]); // outer loop, outer block
    // return valid
    c.extend_from_slice(&[op::LOCAL_GET, valid, op::END]);
    sized(c)
}

/// Prefix a code body (`local-decls … end`) with its byte length — a code-section entry.
fn sized(inner: Vec<u8>) -> Vec<u8> {
    let mut out = uleb_bytes(inner.len() as u64);
    out.extend_from_slice(&inner);
    out
}

/// A component functype byte-vec `0x60 <params> <results>` for a `(i32…)->i32`-shaped helper/fn.
fn functype_i32(n_params: usize) -> Vec<u8> {
    let mut out = vec![0x60];
    out.extend_from_slice(&wasm_vec(n_params, &vec![0x7F; n_params]));
    out.extend_from_slice(&wasm_vec(1, &[0x7F]));
    out
}

/// An export-section entry `<name-len> name <kind> <index>`.
fn export_entry(name: &str, kind: u8, index: u32) -> Vec<u8> {
    let mut out = uleb_bytes(name.len() as u64);
    out.extend_from_slice(name.as_bytes());
    out.push(kind);
    uleb128(index as u64, &mut out);
    out
}

/// Build the runtime-compound component for a module whose ENTRY returns a heap value. `funcs` are
/// the selected user functions (entry first, index 0); `render_bodies` are the per-type render fn
/// code-section entries; `run_body` is the `run : () -> i32` code-section entry (both from `render`).
/// Reuses the `RT_*` envelope constants; assembles the core module and wraps it in RT_HEAD/RT_TAIL.
pub fn component(
    funcs: &[&SelectedFunc],
    user_code: &[Vec<u8>],
    render_bodies: &[Vec<u8>],
    run_body: &[u8],
) -> Vec<u8> {
    let n_user = funcs.len();
    let n_render = render_bodies.len();

    // ── Type section: import types, then realloc, putu(=itoa), then one per user func, then a
    //    shared render type `(i32,i32)->i32`, then run `()->i32`. ──
    let mut type_items = Vec::new();
    let mut n_types = 0usize;
    for t in rt_import_types() {
        type_items.extend_from_slice(&t);
        n_types += 1;
    }
    let ty_realloc = n_types as u32;
    type_items.extend_from_slice(&functype_i32(4)); // cabi_realloc (i32×4)->i32
    n_types += 1;
    let ty_putu = n_types as u32; // (i64,i32)->i32 ; itoa shares
    type_items.extend_from_slice(&{
        let mut t = vec![0x60];
        t.extend_from_slice(&wasm_vec(2, &[0x7E, 0x7F]));
        t.extend_from_slice(&wasm_vec(1, &[0x7F]));
        t
    });
    n_types += 1;
    let ty_utf8 = n_types as u32; // utf8-valid (i32)->i32
    type_items.extend_from_slice(&functype_i32(1));
    n_types += 1;
    let ty_user_base = n_types as u32;
    for f in funcs {
        type_items.extend_from_slice(&user_functype(f));
        n_types += 1;
    }
    let ty_render = n_types as u32;
    if n_render > 0 {
        type_items.extend_from_slice(&functype_i32(2)); // (handle, cursor)->cursor
        n_types += 1;
    }
    let ty_run = n_types as u32;
    type_items.extend_from_slice(&{
        let mut t = vec![0x60];
        t.extend_from_slice(&wasm_vec(0, &[]));
        t.extend_from_slice(&wasm_vec(1, &[0x7F]));
        t
    });
    n_types += 1;
    let type_sec = section(1, &wasm_vec(n_types, &type_items));

    // ── Import section (fixed 42 heap imports). ──
    let import_sec = section(2, RT_IMPORT_CONTENT);

    // ── Function section: realloc, putu, itoa, utf8-valid, user funcs, render fns, run. ──
    let mut func_items = Vec::new();
    uleb128(ty_realloc as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items);
    uleb128(ty_putu as u64, &mut func_items); // itoa shares putu's shape
    uleb128(ty_utf8 as u64, &mut func_items);
    for u in 0..n_user {
        uleb128((ty_user_base + u as u32) as u64, &mut func_items);
    }
    for _ in 0..n_render {
        uleb128(ty_render as u64, &mut func_items);
    }
    uleb128(ty_run as u64, &mut func_items);
    let n_funcs = RT_FIXED_FUNCS as usize + n_user + n_render + 1;
    let func_sec = section(3, &wasm_vec(n_funcs, &func_items));

    // ── Memory + global (fixed). ──
    let mem_sec = section(5, RT_MEM);
    let glob_sec = section(6, RT_GLOBAL);

    // ── Export section: memory, cabi_realloc, run. ──
    let run_idx = RT_FUNC_BASE + (n_user + n_render) as u32;
    let mut exports = Vec::new();
    exports.extend_from_slice(&export_entry("memory", 0x02, 0));
    exports.extend_from_slice(&export_entry("cabi_realloc", 0x00, RT_REALLOC));
    exports.extend_from_slice(&export_entry("run", 0x00, run_idx));
    let export_sec = section(7, &wasm_vec(3, &exports));

    // ── Code section: realloc/putu/itoa/utf8-valid, user bodies, render bodies, run. ──
    let mut code_items = Vec::new();
    code_items.extend_from_slice(&realloc_body());
    code_items.extend_from_slice(&putu_body());
    code_items.extend_from_slice(&itoa_body());
    code_items.extend_from_slice(&utf8_valid_body());
    for b in user_code {
        code_items.extend_from_slice(b);
    }
    for b in render_bodies {
        code_items.extend_from_slice(b);
    }
    code_items.extend_from_slice(run_body);
    let code_sec = section(10, &wasm_vec(n_funcs, &code_items));

    // ── Core module. ──
    let mut core = Vec::new();
    core.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]); // \0asm v1
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&glob_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&code_sec);

    // ── Component: RT_HEAD + embedded core module + RT_TAIL. ──
    let mut out = Vec::new();
    out.extend_from_slice(RT_HEAD);
    out.push(1); // core-module section id
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(&core);
    out.extend_from_slice(RT_TAIL);
    out
}

/// A user function's core functype `0x60 <params> <result>`. In the heap component every scalar is
/// its core valtype and every compound (heap) value is an i32 handle.
fn user_functype(f: &SelectedFunc) -> Vec<u8> {
    let mut out = vec![0x60];
    let params: Vec<u8> = f.params.iter().map(|vt| vt.byte()).collect();
    out.extend_from_slice(&wasm_vec(params.len(), &params));
    // A heap-returning function's result is the i32 handle; a scalar's is its valtype; Unit → none.
    match f.ret.core_valtype() {
        Some(vt) => out.extend_from_slice(&wasm_vec(1, &[vt.byte()])),
        None => out.extend_from_slice(&wasm_vec(0, &[])),
    }
    out
}
