//! `serialize::encode_walk` — the R2 value-form ENCODE walkers + the bytes-roundtrip core modules.
//!
//! Split out of `serialize` (the file crossed the 512 KiB lint-mandates limit): the runtime
//! `value-encode` walk bodies (`encode_walk_body`/`encode_bytes_walk_body`/`encode_recursive_sum_walk_body`/
//! `encode_sum_walk_body`), the `t-encode` REP-recovery (`RepSource`), the bytes-roundtrip assemblers
//! (`bytes_roundtrip_core_module`/`_host_`), and the bump-realloc / hole-fill emit helpers. It shares the
//! parent `serialize` module's private emit helpers via `use super::*` and the same crate-level imports.

use super::*;
use crate::backend::wasm::encode::{section, uleb_bytes, uleb128, wasm_vec};
use crate::backend::wasm::lir::{Lir, ValType};
use crate::backend::wasm::runtime_abi::RtOp;
use crate::backend::wasm::wasm_abi;

/// The `t-encode(handle) -> i32` code-section entry (the R2 walker). Locals: 0 = the resource-table
/// handle param, 1 = the recovered i32 heap rep, 2 = i64 scratch. Recovers the rep via
/// `resource.rep(handle)` (core func `f_rrep`), then for each template hole walks its `arr-get` path
/// from the rep, reads the leaf (`get-int`/`get-bool`), and writes its bytes into the template (at mem
/// offset 0, doubling as the output buffer); returns the `(ptr=0, len)` return area at `ret_off`. The
/// How the `t-encode`/walk body recovers the heap REP from its `self` param, and whether it reclaims the
/// handle. Two receiver shapes:
///  * `Own(f_rrep)` — `encode` takes `own<t>`: `self` is a resource-table INDEX, so the rep is
///    `resource.rep(self)` (core func `f_rrep`), and encode OWNS the handle so it must `heap.drop(rep)` to
///    reclaim it (the constant-escape shape, whose resource carries no live heap handle, still uses this —
///    a drop of a baked rep is harmless).
///  * `Borrow` — `encode` takes `borrow<t>`: the canonical ABI's `lift_borrow` hands the guest the REP
///    DIRECTLY as the param (NOT a table index; wasmtime `resource_lift_borrow` returns `rep`), so the rep
///    IS `self` and there is NO `resource.rep`. Encode does NOT own the handle — the host keeps it and
///    drops it after the call (firing the dtor) — so encode must NOT drop. The value survives → the method
///    is repeatable. ([[rcdzc-r1-resource-encode-linking-findings]], the 2026-07-13 borrow correction.)
#[derive(Clone, Copy)]
pub(super) enum RepSource {
    /// `own<t>` self: recover the rep via `resource.rep(self)` (the core func index) and drop it after
    /// the walk. The RUNTIME resource path now lifts every method as `borrow` (see below), so this variant
    /// is the documented reference for the own shape — kept for the constant/closure paths that may adopt
    /// it and to make `emit_bind_rep`/`emit_drop_if_owned` total over both receiver modes.
    #[allow(dead_code)]
    Own(u32),
    Borrow,
}

impl RepSource {
    /// Emit the prologue binding local `rep` to the heap rep: `resource.rep(local 0)` (own) or a plain
    /// copy of the `self` param (borrow — the param IS the rep).
    fn emit_bind_rep(
        self,
        rep: u32,
        body: &mut Vec<u8>,
        _import_index: &std::collections::HashMap<&str, u32>,
    ) {
        use crate::backend::wasm::wasm_abi::op;
        body.push(op::LOCAL_GET);
        uleb128(0, body); // the self param
        if let RepSource::Own(f_rrep) = self {
            body.push(op::CALL);
            uleb128(f_rrep as u64, body); // resource.rep(handle) → rep
        }
        body.push(op::LOCAL_SET);
        uleb128(rep as u64, body);
    }

    /// Emit the epilogue reclaim: `heap.drop(rep)` for an OWNED self (encode holds the last reference); a
    /// borrow self reclaims NOTHING here (the host/dtor owns the release).
    fn emit_drop_if_owned(
        self,
        rep: u32,
        body: &mut Vec<u8>,
        import_index: &std::collections::HashMap<&str, u32>,
    ) {
        use crate::backend::wasm::wasm_abi::op;
        if let RepSource::Own(_) = self {
            body.push(op::LOCAL_GET);
            uleb128(rep as u64, body);
            body.push(op::CALL);
            uleb128(import_index["drop"] as u64, body);
        }
    }
}

/// walk ops resolve by name through `import_index` (the same map the defined bodies use).
pub(super) fn encode_walk_body(
    template: &crate::lower::ValueFormTemplate,
    byte_off: usize,
    ret_off: usize,
    rep_src: RepSource,
    import_index: &std::collections::HashMap<&str, u32>,
) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let mut body = Vec::new();
    // Locals: 1 group of i32 (the rep), 1 group of i64 (scratch).
    uleb128(2, &mut body); // 2 local-decl groups
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I32); // local 1: rep
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I64); // local 2: scratch
    let rep = 1u32;
    let scratch = 2u32;
    // Recover the heap rep into local `rep` — from `resource.rep(self)` (own self) or DIRECTLY from the
    // self param (borrow self: the canonical ABI passes the rep, not a table index).
    rep_src.emit_bind_rep(rep, &mut body, import_index);

    for hole in &template.leaves {
        emit_hole_fill(hole, byte_off, rep, scratch, import_index, &mut body);
    }
    // Reclaim the heap handle IFF `encode` owns it (own self). For a BORROW self the host keeps ownership
    // and drops the handle after the call (firing the dtor, which reclaims the rep) — so `encode` must NOT
    // drop, and the value survives, making the method repeatable.
    rep_src.emit_drop_if_owned(rep, &mut body, import_index);
    // return the (ptr,len) area.
    body.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(ret_off as i64, &mut body);
    body.push(op::END);
    let mut e = uleb_bytes(body.len() as u64);
    e.extend_from_slice(&body);
    e
}

/// The `t-encode(handle) -> i32` walker for a RUNTIME `Bytes` result — the FIRST looping `encode()`
/// (`DESIGN-runtime-bytes-escape-walker.md`). It writes the VARIABLE-length value form
/// `PREFIX · LEB(n) · <n bytes> · SUFFIX` into linear memory and returns a `(ptr,len)` retarea:
///  * retarea at `[0..8]` (ptr,len); the value form is written starting at `OUT = 8`.
///  * write the static `prefix` (a store8 run), the runtime `bytes-len` as a LEB (a bounded loop over
///    the count), a `bytes-get` copy loop `for i in 0..n`, then the static `suffix`.
///  * `heap.drop(rep)` (encode owns `own<t>`), then store `(ptr=OUT, len = w-OUT)` and return `0`.
///
/// `n` (bytes-len) fits u32; the LEB of a u32 is ≤ 5 bytes. Byte constants ≥ 0x80 (the LEB continuation
/// bit) are emitted with `sleb128` (the raw-`i32.const`-is-signed-LEB rule).
pub(super) fn encode_bytes_walk_body(
    form: &crate::lower::RuntimeBytesForm,
    rep_src: RepSource,
    import_index: &std::collections::HashMap<&str, u32>,
) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let call_op = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(import_index[name] as u64, out);
    };
    // A signed-LEB `i32.const v` (v may be ≥ 0x80 — the LEB continuation bit — so NEVER a raw byte).
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    // `[addr, value]` on the stack → store the low byte at `addr` (align 0, offset 0).
    let store8 = |out: &mut Vec<u8>| {
        out.push(op::I32_STORE8);
        out.push(0x00);
        out.push(0x00);
    };
    // Output region begins after the 8-byte retarea.
    const OUT: i64 = 8;

    // Locals (after param 0 = handle): rep(i32), n(i32), w(i32 write cursor), i(i32 loop counter),
    // t(i32 LEB temp). One i32 group of 5.
    let mut body = Vec::new();
    uleb128(1, &mut body); // 1 local-decl group
    uleb128(5, &mut body);
    body.push(wasm_abi::CORE_I32);
    let (rep, n, w, i, t) = (1u32, 2u32, 3u32, 4u32, 5u32);
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };
    let tee = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_TEE);
        uleb128(l as u64, out);
    };

    // Recover the heap rep (own: resource.rep; borrow: the param IS the rep).
    rep_src.emit_bind_rep(rep, &mut body, import_index);
    // n = bytes-len(rep)  (BORROWS rep — drop happens once, at the end, only if we own it).
    get(rep, &mut body);
    call_op("bytes-len", &mut body);
    set(n, &mut body);
    // GROW linear memory to cover the whole value-form write BEFORE writing: memory is `(memory 1)` = one
    // 64-KiB page, but this walker writes `prefix + LEB128(n) + n payload bytes + suffix` starting at OUT,
    // so a >64-KiB payload (e.g. a String result larger than a page) writes OOB past 65536 — the same
    // page-boundary trap class as #7793 (rcw4), on the value-ESCAPE copy-out (breaker sibling repro: a
    // >64-KiB String/Bytes result faults `@0x10000 in a size-0x10000 memory`). Grow to cover the final
    // write cursor: an upper bound is OUT + prefix.len() + 5 (max LEB128 for a u32 length) + suffix.len()
    // + n. `emit_grow_to_cover_out(n, out_offset)` grows ceil((out_offset + n) / 65536) pages iff short
    // (memory.grow never traps, a no-op when memory already suffices); over-approximating the fixed
    // overhead only ever grows one extra page, never too little.
    emit_grow_to_cover_out(
        n,
        OUT + form.prefix.len() as i64 + 5 + form.suffix.len() as i64,
        &mut body,
    );
    // w = OUT.
    const_i32(OUT, &mut body);
    set(w, &mut body);

    // Write the static PREFIX: for each byte, `store8(w, byte); w += 1`.
    for &b in &form.prefix {
        get(w, &mut body);
        const_i32(b as i64, &mut body);
        store8(&mut body);
        get(w, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(w, &mut body);
    }

    // Write LEB128(n): t = n; loop { b = t & 0x7f; t >>= 7 (unsigned); if t!=0 b|=0x80; store8(w,b); w++;
    // if t!=0 continue }. A do-while over `t`.
    get(n, &mut body);
    set(t, &mut body);
    body.push(op::LOOP);
    body.push(wasm_abi::BLOCK_EMPTY);
    {
        // store8(w, (t & 0x7f) | (t>=0x80 ? 0x80 : 0))
        get(w, &mut body);
        // low 7 bits
        get(t, &mut body);
        const_i32(0x7f, &mut body);
        body.push(op::I32_AND);
        // continuation bit: (t >>u 7) != 0 ? 0x80 : 0  → compute ((t >>u 7) != 0) * 0x80 via select-free
        // arithmetic: push 0x80 if more bits remain. Use: more = (t >>u 7); (more != 0) → 0x80 else 0.
        get(t, &mut body);
        const_i32(7, &mut body);
        body.push(op::I32_SHR_U); // more = t >>u 7
        const_i32(0, &mut body);
        body.push(op::I32_NE); // (more != 0) as 0/1
        const_i32(0x80, &mut body);
        body.push(op::I32_MUL); // 0 or 0x80
        body.push(op::I32_OR); // (t&0x7f) | cont
        store8(&mut body);
        // w += 1
        get(w, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(w, &mut body);
        // t >>u= 7
        get(t, &mut body);
        const_i32(7, &mut body);
        body.push(op::I32_SHR_U);
        tee(t, &mut body); // t = t>>7, leave it on stack
        // continue the loop while t != 0 (br_if to loop label 0)
        body.push(op::BR_IF);
        uleb128(0, &mut body);
    }
    body.push(op::END); // end loop

    // COPY LOOP: i = 0; block { loop { if i>=n br 1; store8(w+i, bytes-get(rep,i)); i++; br 0 } }.
    const_i32(0, &mut body);
    set(i, &mut body);
    body.push(op::BLOCK);
    body.push(wasm_abi::BLOCK_EMPTY);
    body.push(op::LOOP);
    body.push(wasm_abi::BLOCK_EMPTY);
    {
        // if i >= n: br 1 (exit block)
        get(i, &mut body);
        get(n, &mut body);
        body.push(op::I32_GE_U);
        body.push(op::BR_IF);
        uleb128(1, &mut body);
        // store8(w + i, bytes-get(rep, i))
        get(w, &mut body);
        get(i, &mut body);
        body.push(op::I32_ADD); // addr = w + i
        get(rep, &mut body);
        get(i, &mut body);
        call_op("bytes-get", &mut body); // → byte value (0..=255)
        store8(&mut body);
        // i += 1
        get(i, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(i, &mut body);
        body.push(op::BR);
        uleb128(0, &mut body);
    }
    body.push(op::END); // end loop
    body.push(op::END); // end block
    // w += n  (advance the write cursor past the copied payload).
    get(w, &mut body);
    get(n, &mut body);
    body.push(op::I32_ADD);
    set(w, &mut body);

    // Write the static SUFFIX.
    for &b in &form.suffix {
        get(w, &mut body);
        const_i32(b as i64, &mut body);
        store8(&mut body);
        get(w, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(w, &mut body);
    }

    // Release the escaped handle ONLY if encode owns it (own self); a borrow self leaves reclamation to
    // the host/dtor, so the value survives for a repeated call.
    rep_src.emit_drop_if_owned(rep, &mut body, import_index);

    // Store the retarea at [0..8]: ptr = OUT, len = w - OUT. Then return 0 (the retptr).
    const_i32(0, &mut body); // addr for ptr store
    const_i32(OUT, &mut body); // ptr = OUT
    body.push(op::I32_STORE);
    body.push(0x02); // align 2 (4-byte)
    body.push(0x00); // offset 0
    const_i32(4, &mut body); // addr for len store
    get(w, &mut body);
    const_i32(OUT, &mut body);
    body.push(op::I32_SUB); // len = w - OUT
    body.push(op::I32_STORE);
    body.push(0x02);
    body.push(0x00);
    // return the retarea pointer (0).
    const_i32(0, &mut body);
    body.push(op::END);
    let mut e = uleb_bytes(body.len() as u64);
    e.extend_from_slice(&body);
    e
}

/// The `t-to-bytes(borrow rep) -> i32` body (VM-3): copy the RAW byte payload of `rep` into the (ptr,len)
/// retarea as a `list<u8>` — the raw content, NO value-form framing (unlike `t-encode`). The param IS the
/// heap rep (borrow), so it reads directly (no `resource.rep`) and does NOT drop (repeatable). Retarea at
/// `[0..8]` (ptr,len); the bytes are written starting at `OUT=8`; `n = bytes-len(rep)`, then a
/// `bytes-get(rep,i)` → `store8(OUT+i)` copy loop, then store `(ptr=OUT, len=n)` and return `0`. Returns
/// the body bytes WITHOUT the leading length ULEB (the caller prefixes it). No value-form prefix/LEB/suffix
/// — this is the "just give me the bytes" affordance, distinct from `encode_bytes_walk_body`.
pub(super) fn to_bytes_body(import_index: &std::collections::HashMap<&str, u32>) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let call_op = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(import_index[name] as u64, out);
    };
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    let store8 = |out: &mut Vec<u8>| {
        out.push(op::I32_STORE8);
        out.push(0x00);
        out.push(0x00);
    };
    const OUT: i64 = 8;
    let mut body = Vec::new();
    // Locals after param 0 = rep: n(i32), i(i32). One i32 group of 2.
    uleb128(1, &mut body);
    uleb128(2, &mut body);
    body.push(wasm_abi::CORE_I32);
    let (rep, n, i) = (0u32, 1u32, 2u32);
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };
    // n = bytes-len(rep)  (borrows rep — no drop, the host owns it).
    get(rep, &mut body);
    call_op("bytes-len", &mut body);
    set(n, &mut body);
    // GROW to cover [OUT, OUT+n) before the copy — a >64-KiB Bytes returned via the `to-bytes` member
    // would otherwise write OOB past the single initial page (the #7793 page-boundary class, on the
    // to-bytes escape copy-out). No-op when memory already suffices.
    emit_grow_to_cover_out(n, OUT, &mut body);
    // COPY LOOP: i = 0; block { loop { if i>=n br 1; store8(OUT+i, bytes-get(rep,i)); i++; br 0 } }.
    const_i32(0, &mut body);
    set(i, &mut body);
    body.push(op::BLOCK);
    body.push(wasm_abi::BLOCK_EMPTY);
    body.push(op::LOOP);
    body.push(wasm_abi::BLOCK_EMPTY);
    {
        get(i, &mut body);
        get(n, &mut body);
        body.push(op::I32_GE_U);
        body.push(op::BR_IF);
        uleb128(1, &mut body);
        // store8(OUT + i, bytes-get(rep, i))
        const_i32(OUT, &mut body);
        get(i, &mut body);
        body.push(op::I32_ADD);
        get(rep, &mut body);
        get(i, &mut body);
        call_op("bytes-get", &mut body);
        store8(&mut body);
        get(i, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(i, &mut body);
        body.push(op::BR);
        uleb128(0, &mut body);
    }
    body.push(op::END); // end loop
    body.push(op::END); // end block
    // retarea [0..8]: ptr = OUT, len = n.
    const_i32(0, &mut body);
    const_i32(OUT, &mut body);
    body.push(op::I32_STORE);
    body.push(0x02);
    body.push(0x00);
    const_i32(4, &mut body);
    get(n, &mut body);
    body.push(op::I32_STORE);
    body.push(0x02);
    body.push(0x00);
    const_i32(0, &mut body); // return the retarea pointer (0)
    body.push(op::END);
    body
}

/// The `t-encode(handle) -> i32` walker for a RUNTIME RECURSIVE sum (a linked list, a tree). Unlike the
/// fixed-template walkers, it delegates the recursion + document assembly to the runtime `value-encode`
/// op: recover the heap `rep`, build the compiler-baked shape DESCRIPTOR as a heap `Bytes` (reading its
/// constant bytes from the data section at `desc_off`, `bytes-set`ting them into a fresh buffer), call
/// `value-encode(rep, desc)` to render the value-form document (another heap `Bytes`), then COPY that
/// document into linear memory and return `(ptr, len)`. Releases all three handles (`rep`/`desc`/`doc`).
/// `DESIGN-recursive-sum-escape-walker.md` (approach C). Reuses the copy-loop shape of the bytes walker.
/// The descriptor bytes are COMPILE-TIME CONSTANTS, so they are `bytes-set` with literal `i32.const`
/// values (no data-section blob / memory load needed).
pub(super) fn encode_recursive_sum_walk_body(
    descriptor: &[u8],
    rep_src: RepSource,
    import_index: &std::collections::HashMap<&str, u32>,
) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let call_op = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(import_index[name] as u64, out);
    };
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    let store8 = |out: &mut Vec<u8>| {
        out.push(op::I32_STORE8);
        out.push(0x00);
        out.push(0x00);
    };
    // Output region begins after the 8-byte retarea (ptr,len), exactly as the bytes walker.
    const OUT: i64 = 8;
    let mut body = Vec::new();
    // Locals after param 0 = handle: rep, desc(handle), doc(handle), n(i32 doc len), i(i32 loop). 5 i32.
    uleb128(1, &mut body);
    uleb128(5, &mut body);
    body.push(wasm_abi::CORE_I32);
    let (rep, desc, doc, n, i) = (1u32, 2u32, 3u32, 4u32, 5u32);
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };

    // Recover the heap rep (own: resource.rep; borrow: the param IS the rep).
    rep_src.emit_bind_rep(rep, &mut body, import_index);

    // desc = bytes-alloc(len); then for each descriptor byte, desc = bytes-set(desc, j, <const byte>).
    // The descriptor bytes are compile-time constants, so each is a literal `i32.const` (no data-section
    // blob / memory load). `bytes-set` returns the buffer handle — re-`set` the local each time.
    const_i32(descriptor.len() as i64, &mut body);
    call_op("bytes-alloc", &mut body);
    set(desc, &mut body);
    for (j, &byte) in descriptor.iter().enumerate() {
        get(desc, &mut body);
        const_i32(j as i64, &mut body); // index
        const_i32(byte as i64, &mut body); // the constant descriptor byte
        call_op("bytes-set", &mut body);
        set(desc, &mut body);
    }

    // doc = value-encode(rep, desc)  (BORROWS both; returns a fresh owned Bytes document).
    get(rep, &mut body);
    get(desc, &mut body);
    call_op("value-encode", &mut body);
    set(doc, &mut body);
    // n = bytes-len(doc).
    get(doc, &mut body);
    call_op("bytes-len", &mut body);
    set(n, &mut body);

    // Grow linear memory to cover OUT+n before the copy-out (large documents can exceed the 1-page min).
    emit_grow_to_cover_out(n, OUT, &mut body);

    // COPY LOOP: i = 0; block { loop { if i>=n br 1; store8(OUT+i, bytes-get(doc, i)); i++; br 0 } }.
    const_i32(0, &mut body);
    set(i, &mut body);
    body.push(op::BLOCK);
    body.push(wasm_abi::BLOCK_EMPTY);
    body.push(op::LOOP);
    body.push(wasm_abi::BLOCK_EMPTY);
    {
        get(i, &mut body);
        get(n, &mut body);
        body.push(op::I32_GE_U);
        body.push(op::BR_IF);
        uleb128(1, &mut body);
        // store8(OUT + i, bytes-get(doc, i))
        const_i32(OUT, &mut body);
        get(i, &mut body);
        body.push(op::I32_ADD);
        get(doc, &mut body);
        get(i, &mut body);
        call_op("bytes-get", &mut body);
        store8(&mut body);
        get(i, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(i, &mut body);
        body.push(op::BR);
        uleb128(0, &mut body);
    }
    body.push(op::END); // end loop
    body.push(op::END); // end block

    // Release the handles: `rep` ONLY if encode owns it (own self — balances make's alloc; a borrow self
    // leaves rep-reclamation to the host/dtor), plus `desc` + `doc` (temporaries this body built, ALWAYS
    // dropped regardless of the self mode). The value heap is acyclic; each drop cascades to its children.
    rep_src.emit_drop_if_owned(rep, &mut body, import_index);
    get(desc, &mut body);
    call_op("drop", &mut body);
    get(doc, &mut body);
    call_op("drop", &mut body);

    // Store the retarea at [0..8]: ptr = OUT, len = n. Return 0 (the retptr).
    const_i32(0, &mut body);
    const_i32(OUT, &mut body);
    body.push(op::I32_STORE);
    body.push(0x02);
    body.push(0x00);
    const_i32(4, &mut body);
    get(n, &mut body);
    body.push(op::I32_STORE);
    body.push(0x02);
    body.push(0x00);
    const_i32(0, &mut body);
    body.push(op::END);
    let mut e = uleb_bytes(body.len() as u64);
    e.extend_from_slice(&body);
    e
}

/// §3c — the member's `(ptr,len) -> retptr` core body for ANY WIT bytes-roundtrip export member (the fold's
/// `apply` is the first such member, not the contract). Lifts the incoming `list<u8>` value-form document
/// (lowered by the host to `ptr`/`len` in linear memory) into a runtime Bytes handle, value-DECODEs it to
/// the compound param rep (guided by `param_desc`), calls the member's selected body `member_body_abs(rep)`,
/// value-ENCODEs the result to a canonical document (guided by `result_desc`), and copies that out to the
/// `(ptr=OUT, len=n)` retarea (retptr 0). This is the inverse pair of [`encode_recursive_sum_walk_body`]
/// (which only ENCODEs a borrowed rep): here the rep comes from value-decode and the result from the body call.
///
/// OWNERSHIP (leak-critical — MUST be checked with the debug-counters live-objects harness): `value-decode`
/// and `value-encode` BORROW their `(bytes, desc)` / `(rep, desc)` args (mirrors the recursive-sum walker,
/// which drops `desc`+`doc` after encode). The member body CONSUMES its `rep` param (Perceus owned-arg).
/// So this drops `bh`, `pdesc`, `result`, `rdesc`, `doc` — but NOT `rep` (the body took it).
pub(super) fn emit_bytes_roundtrip_apply_body(
    param_desc: &[u8],
    result_desc: &[u8],
    member_body_abs: u32,
    import_index: &std::collections::HashMap<&str, u32>,
    const_result: Option<&[u8]>,
) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let call_op = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(import_index[name] as u64, out);
    };
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    // Output region begins after the 8-byte retarea. Safe against the input at `ptr` because the copy-IN
    // loop below consumes the input into `bh` BEFORE the copy-OUT loop writes at OUT.
    const OUT: i64 = 8;
    // PRE-ENCODE (Axis 2, provider path): a member whose RESULT is a compile-time CONSTANT (independent of the
    // event) has its canonical bare value-form bytes precomputed at compile time (`constant_value_form_bare` —
    // byte-identical to the runtime `value-encode` op for the same constant). Emit an apply body that IGNORES
    // the incoming event, writes those constant bytes straight to OUT, and returns — NO value-decode, NO member
    // body call, NO per-event value-encode + NO heap alloc/drop. The persistent reducer instance answers every
    // event with a memory write of static bytes. (The input list the host lowered via `cabi_realloc` sits above
    // OUT and is simply left unread.)
    if let Some(cbytes) = const_result {
        let mut body = Vec::new();
        uleb128(0, &mut body); // no locals
        for (j, &b) in cbytes.iter().enumerate() {
            const_i32(OUT + j as i64, &mut body); // address
            const_i32(b as i64, &mut body); // value
            body.push(op::I32_STORE8);
            body.push(0x00); // align
            body.push(0x00); // offset
        }
        // retarea: mem[0] = OUT (ptr), mem[4] = len; return retptr 0.
        const_i32(0, &mut body);
        const_i32(OUT, &mut body);
        body.push(op::I32_STORE);
        body.push(0x02);
        body.push(0x00);
        const_i32(4, &mut body);
        const_i32(cbytes.len() as i64, &mut body);
        body.push(op::I32_STORE);
        body.push(0x02);
        body.push(0x00);
        const_i32(0, &mut body); // return retptr 0
        body.push(op::END);
        let mut e = uleb_bytes(body.len() as u64);
        e.extend_from_slice(&body);
        return e;
    }
    let mut body = Vec::new();
    // Locals after params (ptr=0, len=1): bh, pdesc, rep, result, rdesc, doc, n, i — one group of 8 i32.
    uleb128(1, &mut body);
    uleb128(8, &mut body);
    body.push(wasm_abi::CORE_I32);
    let (ptr, len) = (0u32, 1u32);
    let (bh, pdesc, rep, result, rdesc, doc, n, i) =
        (2u32, 3u32, 4u32, 5u32, 6u32, 7u32, 8u32, 9u32);
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };
    // Bake a compile-time-constant descriptor into a heap Bytes handle stashed in `dst` (bytes-alloc + a
    // literal `bytes-set` per byte — same idiom the recursive-sum walker uses for its descriptor).
    let bake_desc = |desc: &[u8], dst: u32, out: &mut Vec<u8>| {
        const_i32(desc.len() as i64, out);
        call_op("bytes-alloc", out);
        set(dst, out);
        for (j, &b) in desc.iter().enumerate() {
            get(dst, out);
            const_i32(j as i64, out);
            const_i32(b as i64, out);
            call_op("bytes-set", out);
            set(dst, out);
        }
    };

    // (1) bh = bytes-alloc(len); copy-IN loop: for i in 0..len { bh = bytes-set(bh, i, load8_u(ptr+i)) }.
    get(len, &mut body);
    call_op("bytes-alloc", &mut body);
    set(bh, &mut body);
    const_i32(0, &mut body);
    set(i, &mut body);
    body.push(op::BLOCK);
    body.push(wasm_abi::BLOCK_EMPTY);
    body.push(op::LOOP);
    body.push(wasm_abi::BLOCK_EMPTY);
    {
        get(i, &mut body);
        get(len, &mut body);
        body.push(op::I32_GE_U);
        body.push(op::BR_IF);
        uleb128(1, &mut body); // exit the block
        get(bh, &mut body);
        get(i, &mut body);
        get(ptr, &mut body);
        get(i, &mut body);
        body.push(op::I32_ADD);
        body.push(op::I32_LOAD8_U);
        body.push(0x00); // align
        body.push(0x00); // offset
        call_op("bytes-set", &mut body);
        set(bh, &mut body);
        get(i, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(i, &mut body);
        body.push(op::BR);
        uleb128(0, &mut body); // continue the loop
    }
    body.push(op::END); // end loop
    body.push(op::END); // end block

    // (2) rep = value-decode(bytes-lift, param_desc).
    bake_desc(param_desc, pdesc, &mut body);
    get(bh, &mut body);
    get(pdesc, &mut body);
    call_op("value-decode", &mut body);
    set(rep, &mut body);

    // (3) result = member-body(rep)  (the member's selected body; CONSUMES rep).
    get(rep, &mut body);
    body.push(op::CALL);
    uleb128(member_body_abs as u64, &mut body);
    set(result, &mut body);

    // (4) doc = value-encode(result, result_desc).
    bake_desc(result_desc, rdesc, &mut body);
    get(result, &mut body);
    get(rdesc, &mut body);
    call_op("value-encode", &mut body);
    set(doc, &mut body);

    // (5) n = bytes-len(doc); copy-OUT loop: for i in 0..n { store8(OUT+i, bytes-get(doc, i)) }.
    get(doc, &mut body);
    call_op("bytes-len", &mut body);
    set(n, &mut body);
    // Grow linear memory to cover OUT+n before the copy-out (a large roundtrip result can exceed 1 page).
    emit_grow_to_cover_out(n, OUT, &mut body);
    const_i32(0, &mut body);
    set(i, &mut body);
    body.push(op::BLOCK);
    body.push(wasm_abi::BLOCK_EMPTY);
    body.push(op::LOOP);
    body.push(wasm_abi::BLOCK_EMPTY);
    {
        get(i, &mut body);
        get(n, &mut body);
        body.push(op::I32_GE_U);
        body.push(op::BR_IF);
        uleb128(1, &mut body);
        const_i32(OUT, &mut body);
        get(i, &mut body);
        body.push(op::I32_ADD);
        get(doc, &mut body);
        get(i, &mut body);
        call_op("bytes-get", &mut body);
        body.push(op::I32_STORE8);
        body.push(0x00);
        body.push(0x00);
        get(i, &mut body);
        const_i32(1, &mut body);
        body.push(op::I32_ADD);
        set(i, &mut body);
        body.push(op::BR);
        uleb128(0, &mut body);
    }
    body.push(op::END);
    body.push(op::END);

    // (6) Drops (see OWNERSHIP above): bh, pdesc, result, rdesc, doc — NOT rep (the body consumed it).
    for h in [bh, pdesc, result, rdesc, doc] {
        get(h, &mut body);
        call_op("drop", &mut body);
    }

    // (7) retarea: mem[0] = OUT (ptr), mem[4] = n (len); return retptr 0.
    const_i32(0, &mut body);
    const_i32(OUT, &mut body);
    body.push(op::I32_STORE);
    body.push(0x02); // align 2 (4-byte)
    body.push(0x00); // offset
    const_i32(4, &mut body);
    get(n, &mut body);
    body.push(op::I32_STORE);
    body.push(0x02);
    body.push(0x00);
    const_i32(0, &mut body);
    body.push(op::END);
    let mut e = uleb_bytes(body.len() as u64);
    e.extend_from_slice(&body);
    e
}

/// Emit a `memory.grow`-to-cover guard: before a copy-OUT loop writes `n` bytes at `OUT`, ensure linear
/// memory is at least `ceil((OUT + n) / 65536)` pages. The initial memory is `(memory 1)` = one 64KiB page
/// (growable, no max), so a document whose canonical bytes exceed `65536 - OUT` would otherwise write past
/// the page boundary and trap (`memory fault @ 0x10000` — the snowflake large-recursive-sum-return OOB).
/// `n_local` is the i32 local holding the document byte length; `out_offset` is the fixed `OUT` base.
///
/// Shape: `if (needed_pages - memory.size) > 0 { memory.grow(needed_pages - memory.size); drop }` where
/// `needed_pages = (n + OUT + 65535) >> 16`. `memory.grow` is a no-op path when memory already suffices
/// (the `IF` guard is false), and it never traps — it returns the previous size (or -1 on failure), which
/// we `drop`. The delta is recomputed inside the `IF` rather than stashed in a local, keeping this a pure
/// append with no local-count bookkeeping in the callers.
pub(super) fn emit_grow_to_cover_out(n_local: u32, out_offset: i64, body: &mut Vec<u8>) {
    use crate::backend::wasm::wasm_abi::op;
    const MEMORY_SIZE: u8 = 0x3f;
    const MEMORY_GROW: u8 = 0x40;
    // needed_pages = (n + OUT + 65535) >> 16 = ceil((OUT + n) / 65536).
    let needed = |body: &mut Vec<u8>| {
        body.push(op::LOCAL_GET);
        uleb128(n_local as u64, body);
        body.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(out_offset + 65535, body);
        body.push(op::I32_ADD);
        body.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(16, body);
        body.push(op::I32_SHR_U);
    };
    // (needed_pages - memory.size) > 0 ?
    needed(body);
    body.push(MEMORY_SIZE);
    body.push(0x00); // mem index 0
    body.push(op::I32_SUB);
    body.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(0, body);
    body.push(op::I32_GT_S);
    body.push(op::IF);
    body.push(wasm_abi::BLOCK_EMPTY);
    {
        // memory.grow(needed_pages - memory.size); drop
        needed(body);
        body.push(MEMORY_SIZE);
        body.push(0x00);
        body.push(op::I32_SUB);
        body.push(MEMORY_GROW);
        body.push(0x00); // mem index 0
        body.push(op::DROP);
    }
    body.push(op::END);
}

/// §3c — a minimal bump-allocator `cabi_realloc(orig_ptr, orig_size, align, new_size) -> i32` body for a
/// bytes-roundtrip member module. The host calls this to LOWER the incoming `list<u8>` document into the
/// guest's owned memory (the canonical component ABI), so — unlike the resource builders' return-0 stub —
/// it must hand back real, `align`-aligned, non-overlapping space. It bump-allocates off a module global
/// (`bump_global`, the high-water cursor, initialized above the fixed `OUT=8` retarea) and never frees:
/// a member invocation is one call, the whole instance is torn down after, so leak-forever is correct and
/// simplest. Ignores `orig_ptr`/`orig_size` (the host only ever grows a fresh 0-ptr allocation for the param list).
pub(crate) fn emit_bump_realloc_body(bump_global: u32) -> Vec<u8> {
    use crate::backend::wasm::wasm_abi::op;
    let mut body = Vec::new();
    // One i32 local (index 4, after the 4 params) to hold the aligned base.
    uleb128(1, &mut body);
    uleb128(1, &mut body);
    body.push(wasm_abi::CORE_I32);
    let emit = |out: &mut Vec<u8>, o: u8| out.push(o);
    let get = |out: &mut Vec<u8>, l: u64| {
        out.push(op::LOCAL_GET);
        uleb128(l, out);
    };
    let const_i32 = |out: &mut Vec<u8>, v: i64| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    // aligned = (global.get(bump) + align - 1) & (0 - align)   [align is a power of two → -align is the mask]
    body.push(op::GLOBAL_GET);
    uleb128(bump_global as u64, &mut body);
    get(&mut body, 2); // align
    emit(&mut body, op::I32_ADD);
    const_i32(&mut body, 1);
    emit(&mut body, op::I32_SUB);
    const_i32(&mut body, 0);
    get(&mut body, 2); // align
    emit(&mut body, op::I32_SUB); // 0 - align
    emit(&mut body, op::I32_AND);
    body.push(op::LOCAL_SET);
    uleb128(4, &mut body); // aligned
    // global.set(bump, aligned + new_size)
    get(&mut body, 4);
    get(&mut body, 3); // new_size
    emit(&mut body, op::I32_ADD);
    body.push(op::GLOBAL_SET);
    uleb128(bump_global as u64, &mut body);
    // GROW linear memory to cover [aligned, aligned + new_size) BEFORE returning: the host writes `new_size`
    // bytes at the returned `aligned` after cabi_realloc returns, and a list<u8> the host lowers here that is
    // larger than the remaining space in the initial 1-page (65536B) memory would otherwise be written
    // OUT-OF-BOUNDS at the wasm-page granule. This is the §3c bytes-roundtrip MEMBER module's own bump
    // allocator — the defensive TWIN of the DEFINE-mode `cabi_realloc` grow in `core_module_impl` (that one
    // is the allocator corpus 05 rcw4 witnesses; a >64-KiB inbound `list<u8>` to a member here would OOB
    // identically). The copy-OUT path already guards this via `emit_grow_to_cover_out`; every guest-emitted
    // bump `cabi_realloc` needs the same grow-to-cover. needed_pages = ceil((aligned + new_size) / 65536);
    // grow the shortfall over memory.size (memory.grow never traps, a no-op when memory already suffices).
    {
        const MEMORY_SIZE: u8 = 0x3f;
        const MEMORY_GROW: u8 = 0x40;
        // needed_pages = (aligned + new_size + 65535) >> 16
        let needed = |out: &mut Vec<u8>| {
            get(out, 4); // aligned
            get(out, 3); // new_size
            out.push(op::I32_ADD);
            const_i32(out, 65535);
            out.push(op::I32_ADD);
            const_i32(out, 16);
            out.push(op::I32_SHR_U);
        };
        // (needed_pages - memory.size) > 0 ?
        needed(&mut body);
        body.push(MEMORY_SIZE);
        body.push(0x00); // mem index 0
        emit(&mut body, op::I32_SUB);
        const_i32(&mut body, 0);
        emit(&mut body, op::I32_GT_S);
        body.push(op::IF);
        body.push(wasm_abi::BLOCK_EMPTY);
        {
            // memory.grow(needed_pages - memory.size); drop
            needed(&mut body);
            body.push(MEMORY_SIZE);
            body.push(0x00);
            emit(&mut body, op::I32_SUB);
            body.push(MEMORY_GROW);
            body.push(0x00); // mem index 0
            body.push(op::DROP);
        }
        body.push(op::END);
    }
    // return aligned
    get(&mut body, 4);
    body.push(op::END);
    let mut e = uleb_bytes(body.len() as u64);
    e.extend_from_slice(&body);
    e
}

/// §3c — assemble the CORE MODULE for ANY WIT bytes-roundtrip provider member (the fold's `apply` is the
/// first such member, not the contract): a single exported `<member>(ptr,len) -> retptr` that value-DECODEs
/// the incoming `list<u8>` document, runs the member's body, and value-ENCODEs the `list<u8>` result. Unlike
/// [`runtime_resource_core_module_form_ex2`] this is a plain function (no resource type / make / t-encode /
/// dtor / methods): the module imports the `k` runtime ops, OWNS a memory + a real bump-allocator
/// `cabi_realloc` (so the host can lower the input list via the canonical component ABI), and exports the
/// member func + `memory` + `cabi_realloc` for the envelope.
///
/// `member_body_abs` is the member body's absolute core-func index (the caller selects `funcs` with an
/// import base of `imports.len()`, so a `CallImport(i)` resolves to `call i` and a self/body call to
/// `k + its emission position`). This increment handles the closure-free, host-import-free member; the
/// caller declines the fused shapes for now.
#[allow(clippy::too_many_arguments)]
pub fn bytes_roundtrip_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    member_body_abs: u32,
    param_desc: &[u8],
    result_desc: &[u8],
    member_name: &str,
    layout: &Layout,
    // PRE-ENCODE: `Some(bytes)` when the member RESULT is a compile-time constant — the apply body writes these
    // precomputed bare value-form bytes and skips decode/body/encode entirely (per-event serialization gone).
    const_result: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    // §2d STATIC-DATA on the PROVIDER path: this hand-assembled envelope now emits its OWN build-once
    // GLOBAL/START sections (mirroring `core_module_impl`) so a constant Bytes/String/Tuple/Record/small-List
    // in a reducer's body — including its RETURNED effect-list's constant parts — is built ONCE in `start`
    // (immortal, census-excluded) and read with `global.get`, amortized across every event the persistent
    // reducer instance folds, rather than rebuilt per call. Static globals occupy indices `0..n_static`
    // (bytes) then `n_static..n_static+n_compounds` (compounds); the mutable realloc bump cursor FOLLOWS them
    // (index `n_static+n_compounds`, was 0). `n_init == 0` (no constant hoisted) → no GLOBAL-static/START/init
    // additions and the bump cursor stays global 0 → byte-identical to the pre-static envelope.
    let n_static = layout.static_bytes.len();
    let n_compounds = layout.static_compounds.len();
    let n_init = (n_static > 0 || n_compounds > 0) as usize;
    let bump_global: u32 = (n_static + n_compounds) as u32;

    // ── Type section ── import functypes 0..k, then one per defined body (k..k+n), then apply
    // `(i32,i32)->i32` (k+n) and cabi_realloc `(i32×4)->i32` (k+n+1).
    let mut type_items = Vec::new();
    for o in imports {
        type_items.extend_from_slice(&import_functype(o));
    }
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let apply_type_idx = k + n;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(2, &[wasm_abi::CORE_I32, wasm_abi::CORE_I32]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let realloc_type_idx = k + n + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // The `start` init func type `() -> ()` (index k+n+2), present only when there are static globals to build.
    let init_type_idx = k + n + 2;
    if n_init == 1 {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(0, &[]));
        t.extend_from_slice(&wasm_vec(0, &[]));
        type_items.extend_from_slice(&t);
    }
    let total_types = k + n + 2 + n_init;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── the k runtime ops (func indices 0..k), from the runtime import name. Guest OWNS
    // its memory (canonical ABI), so no memory import.
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut import_items = Vec::new();
    for (j, o) in imports.iter().enumerate() {
        import_items.extend_from_slice(&import_item(o.name, j as u32));
        import_index.insert(o.name, j as u32);
    }
    let import_sec = section(2, &wasm_vec(k, &import_items));

    // ── Function section ── defined bodies (types k..k+n), then apply (type k+n), realloc (type k+n+1).
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((k + i) as u64, &mut func_items);
    }
    uleb128(apply_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    // The `start` init defined func LAST (type k+n+2) so apply/realloc indices don't shift.
    if n_init == 1 {
        uleb128(init_type_idx as u64, &mut func_items);
    }
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + 2 + n_init, &func_items),
    );
    let apply_abs = (k + n) as u32;
    let realloc_abs = apply_abs + 1;
    let init_func_abs = realloc_abs + 1; // k+n+2 — the START-named init (valid only when n_init == 1)

    // ── Memory section ── one owned memory, min 1 page.
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Global section ── the STATIC-VALUE globals FIRST (mutable i32, init 0 — the `start` init overwrites
    // each with the once-built immortal handle): `0..n_static` bytes then `n_static..n_static+n_compounds`
    // compounds, matching the `global.get` indices the selected body emits (`try_emit_static_bytes` at `pos`,
    // `try_emit_static_compound` at `n_static+pos`). Then the MUTABLE i32 realloc bump cursor LAST (index
    // `n_static+n_compounds`), init above the fixed OUT=8 retarea (16 gives slack); `cabi_realloc` bumps it to
    // hand the host non-overlapping space for the input list.
    let global_sec = {
        let mut items = Vec::new();
        for _ in 0..(n_static + n_compounds) {
            items.push(wasm_abi::CORE_I32);
            items.push(0x01); // mutable
            items.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(0, &mut items);
            items.push(op::END);
        }
        items.push(wasm_abi::CORE_I32);
        items.push(0x01); // mutable
        items.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(16, &mut items);
        items.push(op::END);
        section(
            wasm_abi::CORE_SEC_GLOBAL,
            &wasm_vec(n_static + n_compounds + 1, &items),
        )
    };

    // ── Export section ── the apply member (by its declared name), the owned memory, and cabi_realloc —
    // the three the bytes-roundtrip envelope aliases + canon-lifts through.
    let export_sec = {
        let export = |name: &str, kind: u8, idx: u32| {
            let mut item = uleb_bytes(name.len() as u64);
            item.extend_from_slice(name.as_bytes());
            item.push(kind);
            uleb128(idx as u64, &mut item);
            item
        };
        let mut items = Vec::new();
        items.extend_from_slice(&export(member_name, wasm_abi::EXPORT_KIND_FUNC, apply_abs));
        items.extend_from_slice(&export("memory", wasm_abi::EXPORT_KIND_MEMORY, 0));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(3, &items))
    };

    // ── Code section ── defined bodies (emission order), then the apply body + the realloc body.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    code_items.extend_from_slice(&emit_bytes_roundtrip_apply_body(
        param_desc,
        result_desc,
        member_body_abs,
        &import_index,
        const_result,
    ));
    code_items.extend_from_slice(&emit_bump_realloc_body(bump_global));
    // The `start` init body LAST (when there are static globals): build each constant ONCE, mark it immortal,
    // and store the handle in its global — the SAME sequence `core_module_impl`'s init uses, resolved through
    // the same `import_index` via `code_entry`. Static bytes first (globals `0..n_static`), then the precomputed
    // static-compound init (`build_static_compound_init`, `global.set`ting `n_static+k`).
    if n_init == 1 {
        let mut code: Vec<crate::backend::wasm::lir::Lir> = Vec::new();
        for (g, bytes) in layout.static_bytes.iter().enumerate() {
            code.push(Lir::ConstI32(bytes.len() as i32));
            code.push(Lir::CallImport("bytes-alloc"));
            for (i, &b) in bytes.iter().enumerate() {
                code.push(Lir::ConstI32(i as i32));
                code.push(Lir::ConstI32(b as i32));
                code.push(Lir::CallImport("bytes-set"));
            }
            code.push(Lir::CallImport("mark-immortal"));
            code.push(Lir::GlobalSet(g as u32));
        }
        code.extend_from_slice(&layout.static_compound_init);
        let init = SelectedFunc {
            params: Vec::new(),
            ret: crate::ty::Ty::Unit,
            code,
            // The static-compound init is stack-threaded EXCEPT a hoisted Map/Set with a LIST key, whose
            // `emit_key_canonicalize` stashes the raw key + descriptor in two i32 scratch locals. Declare
            // exactly the scratch the init uses (`static_compound_init_locals`, all i32 handles) — else the
            // init's `local.get`/`local.set` reference undeclared locals = invalid wasm (the ikc1/itf2 bug).
            declared: vec![ValType::I32; layout.static_compound_init_locals as usize],
            src_body: None,
            locals: Vec::new(),
            scopes: Vec::new(),
            stmt_lines: Vec::new(),
        };
        code_items.extend_from_slice(&code_entry(&init, &import_index));
    }
    let code_sec = section(
        wasm_abi::CORE_SEC_CODE,
        &wasm_vec(n + 2 + n_init, &code_items),
    );

    // ── Start section (8) ── names the init func (k+n+2); laid between EXPORT (7) and CODE (10). Absent when
    // there are no static globals (byte-identical to the pre-static envelope).
    let start_sec = if n_init == 1 {
        section(wasm_abi::CORE_SEC_START, &uleb_bytes(init_func_abs as u64))
    } else {
        Vec::new()
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&global_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&start_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// §3c GAP B — the HOST-FUSED variant of [`bytes_roundtrip_core_module`]: a bytes-roundtrip member whose
/// body also calls a HOST interface (e.g. `kv`). It differs from the pure form in ONE way — the memory is
/// IMPORTED (`mem`.`mem`, memory 0), not owned. A host `list<u8>` arg is canon-LOWERED at the component
/// level (reading (ptr,len) from a memory) BEFORE the program instantiates, so that memory must be a shared
/// module the envelope provides (see [`assemble_host_runtime_mem`]); the apply body's value-decode/encode
/// marshal + `cabi_realloc` bump then use that SAME shared memory 0. Import order (host FIRST so a
/// `Lir::CallHostImport(i)=call i` resolves): host func imports `0..h` (module `"host"`), runtime ops
/// `h..h+k` (module `"heap"`, resolved by name via `import_index`), then the `"mem"` memory import. The
/// caller selects `funcs` with import base `h+k` and `host_needs_memory` set; `member_body_abs` is the
/// member body's absolute core index. Exports the member func + `cabi_realloc` (NOT memory — it is imported).
#[allow(clippy::too_many_arguments)]
pub fn bytes_roundtrip_host_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    host_fns: &[crate::backend::wasm::host::HostImport],
    member_body_abs: u32,
    param_desc: &[u8],
    result_desc: &[u8],
    member_name: &str,
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    let h = host_fns.len();
    let k = imports.len();
    let n = funcs.len();
    // §2d STATIC-DATA on the HOST-FUSED provider path (mirrors the pure `bytes_roundtrip_core_module`): a
    // constant in the reducer body / its returned effect-list builds ONCE in `start` (immortal) + `global.get`
    // per event. This envelope has NO owned bump cursor (the shared `mem` module owns it), so the static-value
    // globals are the ONLY globals — indices `0..n_static` (bytes) then `n_static..n_static+n_compounds`
    // (compounds), matching the `global.get` indices the selected body emits. `n_init == 0` → no GLOBAL/START/
    // init additions → byte-identical to the pre-static host envelope.
    let n_static = layout.static_bytes.len();
    let n_compounds = layout.static_compounds.len();
    let n_init = (n_static > 0 || n_compounds > 0) as usize;

    // ── Type section ── host functypes 0..h, runtime h..h+k, defined h+k..h+k+n, apply (h+k+n), realloc.
    let mut type_items = Vec::new();
    for f in host_fns {
        type_items.extend_from_slice(&host_import_functype(f));
    }
    for o in imports {
        type_items.extend_from_slice(&import_functype(o));
    }
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let apply_type_idx = h + k + n;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(2, &[wasm_abi::CORE_I32, wasm_abi::CORE_I32]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // A host op that RETURNS `option<list<u8>>` (kv.get) needs the guest to IMPORT the shared `cabi_realloc`
    // — the select lift allocates the spilled-result retptr area with it (the apply body itself allocates
    // nothing; it writes its result at the fixed OUT=8 retarea). Import its `(i32×4)->i32` functype here at
    // type index `h+k+n+1`. A set with NO option-result op (e.g. kv.put) imports no realloc → byte-identical.
    let needs_realloc = host_fns.iter().any(|f| f.spilled_result.is_some());
    let realloc_type_idx = (h + k + n + 1) as u32;
    if needs_realloc {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // The `start` init func type `() -> ()` LAST (index h+k+n+1+needs_realloc), present iff there are statics.
    let init_type_idx = (h + k + n + 1 + needs_realloc as usize) as u32;
    if n_init == 1 {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(0, &[]));
        t.extend_from_slice(&wasm_vec(0, &[]));
        type_items.extend_from_slice(&t);
    }
    let type_sec = section(
        wasm_abi::CORE_SEC_TYPE,
        &wasm_vec(h + k + n + 1 + needs_realloc as usize + n_init, &type_items),
    );

    // ── Import section ── host func imports (module "host", 0..h), runtime ops (module "heap", h..h+k),
    // then the SHARED memory (module "mem", name "mem", memory 0). The lowered host ops read their list<u8>
    // args out of this shared memory (envelope: `canon_lower_item_mem`), and the apply body marshals into it.
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut import_items = Vec::new();
    for (i, f) in host_fns.iter().enumerate() {
        import_items.extend_from_slice(&host_import_item(&f.op, i as u32));
    }
    for (j, o) in imports.iter().enumerate() {
        let ti = (h + j) as u32;
        import_items.extend_from_slice(&import_item(o.name, ti));
        import_index.insert(o.name, ti);
    }
    // The shared `cabi_realloc` FUNC import (module "mem", func index h+k) — present iff a host op returns
    // an option<list<u8>>. The select lift calls it (via `import_index`) to allocate the retptr area; it
    // shifts the DEFINED funcs to `h+k+1..`. `mem` also exports the memory (below); both come from instance 0.
    if needs_realloc {
        let mut it = uleb_bytes("mem".len() as u64);
        it.extend_from_slice(b"mem");
        it.extend_from_slice(&uleb_bytes("cabi_realloc".len() as u64));
        it.extend_from_slice(b"cabi_realloc");
        it.push(0x00); // import desc: func
        uleb128(realloc_type_idx as u64, &mut it);
        import_items.extend_from_slice(&it);
        import_index.insert("cabi_realloc", (h + k) as u32);
    }
    // The `mem`.`mem` memory import (desc 0x02, limits flag 0x00 min-only, min 1 page).
    let mut mem_import = uleb_bytes("mem".len() as u64);
    mem_import.extend_from_slice(b"mem");
    mem_import.extend_from_slice(&uleb_bytes("mem".len() as u64));
    mem_import.extend_from_slice(b"mem");
    mem_import.push(0x02);
    mem_import.push(0x00);
    uleb128(1, &mut mem_import);
    import_items.extend_from_slice(&mem_import);
    let import_sec = section(
        2,
        &wasm_vec(h + k + 1 + needs_realloc as usize, &import_items),
    );

    // ── Function section ── defined bodies + apply (TYPE indices unchanged). The cabi_realloc import (when
    // present) shifts the DEFINED FUNC indices by +1 — so `apply_abs` (and `member_body_abs` from the caller)
    // account for `needs_realloc`. func_sec still lists the same n+1 type indices.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((h + k + i) as u64, &mut func_items);
    }
    uleb128(apply_type_idx as u64, &mut func_items);
    // The `start` init defined func LAST (type init_type_idx) so the apply index doesn't shift.
    if n_init == 1 {
        uleb128(init_type_idx as u64, &mut func_items);
    }
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + 1 + n_init, &func_items),
    );
    let apply_abs = (h + k + n + needs_realloc as usize) as u32;
    let init_func_abs = apply_abs + 1; // the START-named init (valid only when n_init == 1)

    // NO memory section — memory 0 is the imported shared `mem`. The shared mem module owns the realloc bump
    // cursor, so this envelope's ONLY globals are the §2d static-value slots (mutable i32, init 0 — the `start`
    // init overwrites each with the once-built immortal handle): `0..n_static` bytes then compounds, matching
    // the `global.get` indices the selected body emits. Absent (no global section) when nothing is hoisted.
    let global_sec = if n_init == 1 {
        let mut items = Vec::new();
        for _ in 0..(n_static + n_compounds) {
            items.push(wasm_abi::CORE_I32);
            items.push(0x01); // mutable
            items.push(wasm_abi::op::I32_CONST);
            crate::backend::wasm::encode::sleb128(0, &mut items);
            items.push(wasm_abi::op::END);
        }
        section(
            wasm_abi::CORE_SEC_GLOBAL,
            &wasm_vec(n_static + n_compounds, &items),
        )
    } else {
        Vec::new()
    };

    // ── Export section ── the member func only. `cabi_realloc` is exported by the shared mem module (the
    // apply lift + kv.get lower alias it from THERE), not by the guest.
    let export_sec = {
        let mut item = uleb_bytes(member_name.len() as u64);
        item.extend_from_slice(member_name.as_bytes());
        item.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128(apply_abs as u64, &mut item);
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(1, &item))
    };

    // ── Code section ── defined bodies, then the apply body. NO realloc body.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    code_items.extend_from_slice(&emit_bytes_roundtrip_apply_body(
        param_desc,
        result_desc,
        member_body_abs,
        &import_index,
        None, // host-fused constant-result pre-encode is a later slice
    ));
    // The `start` init body LAST (when there are statics): build each constant ONCE + mark-immortal +
    // global.set — identical to the pure envelope's init, resolved through the same `import_index`.
    if n_init == 1 {
        let mut code: Vec<crate::backend::wasm::lir::Lir> = Vec::new();
        for (g, bytes) in layout.static_bytes.iter().enumerate() {
            code.push(Lir::ConstI32(bytes.len() as i32));
            code.push(Lir::CallImport("bytes-alloc"));
            for (i, &b) in bytes.iter().enumerate() {
                code.push(Lir::ConstI32(i as i32));
                code.push(Lir::ConstI32(b as i32));
                code.push(Lir::CallImport("bytes-set"));
            }
            code.push(Lir::CallImport("mark-immortal"));
            code.push(Lir::GlobalSet(g as u32));
        }
        code.extend_from_slice(&layout.static_compound_init);
        let init = SelectedFunc {
            params: Vec::new(),
            ret: crate::ty::Ty::Unit,
            code,
            // The static-compound init is stack-threaded EXCEPT a hoisted Map/Set with a LIST key, whose
            // `emit_key_canonicalize` stashes the raw key + descriptor in two i32 scratch locals. Declare
            // exactly the scratch the init uses (`static_compound_init_locals`, all i32 handles) — else the
            // init's `local.get`/`local.set` reference undeclared locals = invalid wasm (the ikc1/itf2 bug).
            declared: vec![ValType::I32; layout.static_compound_init_locals as usize],
            src_body: None,
            locals: Vec::new(),
            scopes: Vec::new(),
            stmt_lines: Vec::new(),
        };
        code_items.extend_from_slice(&code_entry(&init, &import_index));
    }
    let code_sec = section(
        wasm_abi::CORE_SEC_CODE,
        &wasm_vec(n + 1 + n_init, &code_items),
    );

    // ── Start section (8) ── names the init func; between EXPORT (7) and CODE (10). Absent when no statics.
    let start_sec = if n_init == 1 {
        section(wasm_abi::CORE_SEC_START, &uleb_bytes(init_func_abs as u64))
    } else {
        Vec::new()
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&global_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&start_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// Emit the instructions that WALK to one hole's leaf and WRITE its bytes into the output buffer (at the
/// hole's absolute `offset`). Shared by the flat tuple/record walker and the per-variant sum walker.
/// `rep` is the local holding the root heap handle; `scratch` an i64 scratch local. The walk starts at
/// `rep`, calls `sum-payload` first if the hole is `via_sum_payload` (a sum variant payload leaf), then
/// applies the `arr-get` path; the leaf read + byte writes match `LeafFill`. Ops resolve by name.
pub(super) fn emit_hole_fill(
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
pub(super) struct Placed {
    pub(super) byte_off: usize,
    pub(super) ret_off: usize,
}

/// Flatten the placed templates to `(byte_off, ret_off)` pairs in variant/discriminant order — the
/// layout the sum walker needs (where each variant's bytes + return area sit in the data section).
pub(super) fn placed_pairs(placed: &[Placed]) -> Vec<(usize, usize)> {
    placed.iter().map(|p| (p.byte_off, p.ret_off)).collect()
}

/// The SUM `t-encode(handle) -> i32` walker. Locals: 0 = resource handle, 1 = i32 rep, 2 = i64 scratch,
/// 3 = i32 discriminant. Recovers the rep, reads `sum-disc(rep)` into `disc`, then an if-chain: for each
/// variant `k`, `if disc == k` fill variant `k`'s holes (each reached through `sum-payload`), `drop` the
/// rep, and return variant `k`'s `(ptr,len)` area (`ret_off`). A trailing `unreachable` closes the chain
/// (the discriminant is always one of the closed variant set). Each variant's holes are written at its
/// own `byte_off` region (its template bytes double as that region's output buffer).
pub(super) fn encode_sum_walk_body(
    variants: &[crate::lower::ValueFormTemplate],
    placed: &[(usize, usize)],
    rep_src: RepSource,
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
    // Recover the heap rep (own: resource.rep; borrow: the param IS the rep).
    rep_src.emit_bind_rep(rep, &mut body, import_index);
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
        // drop the rep ONLY if encode owns it (own self); a borrow self leaves it live. Then return this
        // variant's ret area pointer.
        rep_src.emit_drop_if_owned(rep, &mut body, import_index);
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
