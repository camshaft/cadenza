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

// The R2 value-form encode walkers + bytes-roundtrip assemblers, split out to keep this file under the
// 512 KiB lint-mandates limit. Re-exported so `serialize::bytes_roundtrip_core_module` (and the other
// public entry points) resolve unchanged, and so this module's own unqualified calls to the walkers still
// resolve.
mod encode_walk;
pub(crate) use encode_walk::*;

/// The `\0asm` version-1 core-module preamble — from the generated `wasm_abi` table (`Module::HEADER`
/// as `wasm-encoder` writes it), not a hand-typed byte string.
const CORE_MAGIC: &[u8] = wasm_abi::CORE_MAGIC;

/// The core functype `0x60 <params-vec> <results-vec>` of a runtime import op (from its generated
/// signature). Each ABI type projects to its CORE valtype byte (`AbiValType::core_byte` — a `u32`
/// handle lowers to i32, an `s64` to i64, …); the component-boundary bytes are the envelope's concern.
/// A runtime op returns at most one core value; `dup`/`drop` return none.
///
/// `bytes-new`/`bytes-read` are the exception: they carry a `list<u8>` the scalar `RtOp` model cannot
/// express (their `params`/`result` drop the list, which is why they are `lowerable: false`). Their
/// canon-lowered CORE import signatures are hand-written to match the envelope's list-aware canon-lower:
/// `bytes-new: (ptr:i32, len:i32) -> (handle:i32)` (the `list<u8>` arg lowered to a `(ptr,len)` pair), and
/// `bytes-read: (buf:i32, retptr:i32) -> ()` (a `list<u8>` result is 2 flats > `MAX_FLAT_RESULTS`, so it
/// returns through a guest-provided trailing return-area pointer, not a core result).
fn import_functype(o: &RtOp) -> Vec<u8> {
    let mut out = vec![wasm_abi::CORE_FUNCTYPE_FORM];
    match o.name {
        "bytes-new" => {
            out.extend_from_slice(&wasm_vec(2, &[wasm_abi::CORE_I32, wasm_abi::CORE_I32]));
            out.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
            return out;
        }
        "bytes-read" => {
            out.extend_from_slice(&wasm_vec(2, &[wasm_abi::CORE_I32, wasm_abi::CORE_I32]));
            out.extend_from_slice(&wasm_vec(0, &[]));
            return out;
        }
        _ => {}
    }
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

/// The CORE functype `0x60 <params-vec> <results-vec>` of a HOST-delegated op — its parameter and result
/// CORE valtype bytes. A `Unit` domain/result contributes no core slot (elided at the boundary). A SCALAR
/// param is one core slot (`AbiValType::core_byte`); a STRING param is TWO core slots `(i32 ptr, i32 len)`
/// — the canonical ABI lowering of `string` into linear memory.
/// Flatten ONE record-field ABI into its CORE valtype byte(s), appending to `out`: a scalar → 1 slot
/// (`core_byte`); a `Bytes` → 2 slots `(ptr,len)`; a NESTED record → its fields' flattening inline
/// (recursively — a nested record does NOT spill, its fields join the parent's flattened run). The count +
/// order MUST match the canonical ABI flattening of the component `record` param, or the module is invalid.
fn flatten_record_field_abi(f: &crate::backend::wasm::host::RecordFieldAbi, out: &mut Vec<u8>) {
    use crate::backend::wasm::host::RecordFieldAbi;
    match f {
        RecordFieldAbi::Scalar(v) => out.push(v.core_byte()),
        RecordFieldAbi::Bytes => out.extend_from_slice(&[wasm_abi::CORE_I32, wasm_abi::CORE_I32]),
        RecordFieldAbi::Record(sub) => {
            for (_, sf) in sub {
                flatten_record_field_abi(sf, out);
            }
        }
        // A `result<list<u8>, enum>` field flattens (canonical variant flatten) to `(disc:i32, join(ok=
        // (ptr,len), err=(enum-disc)))` = `(disc, i32, i32)` — 3 slots.
        RecordFieldAbi::Result { .. } => {
            out.extend_from_slice(&[wasm_abi::CORE_I32, wasm_abi::CORE_I32, wasm_abi::CORE_I32])
        }
        // A `list<T>` field flattens to `(ptr, count)` — 2 slots, like `Bytes` (count in place of len) —
        // regardless of the element type (the element data lives behind the pointer, not in the flattened run).
        RecordFieldAbi::List(_) => out.extend_from_slice(&[wasm_abi::CORE_I32, wasm_abi::CORE_I32]),
        // A `tuple<…>` field flattens its elements INLINE (positional), like a nested record — each element's
        // flattened slots join the parent's run (a tuple does not spill).
        RecordFieldAbi::Tuple(elems) => {
            for e in elems {
                flatten_record_field_abi(e, out);
            }
        }
        // An `option<T>` field flattens (canonical variant flatten) to `(disc:i32, flatten(payload))` — the
        // disc slot then the payload's own flattened slots (one scalar this increment).
        RecordFieldAbi::Option(payload) => {
            out.push(wasm_abi::CORE_I32); // the discriminant
            flatten_record_field_abi(payload, out);
        }
        // A general `variant` flattens (canonical variant flatten) to `(disc:i32, join(case payloads))`. The
        // payload join slot is the widest core int (i64 if any payload case is a 64-bit int, else i32 for
        // int/bool/char) or the uniform float core — MATCHING `wit_ctype::flatten_variant` (the import's
        // component type) and the guest marshal's pushed valtype (`select::variant_register_join_vt`); a
        // mixed int/float payload is excluded by the detector. Using the FIRST case's width here (a bug for a
        // mixed-width variant) would declare a core sig the guest push disagrees with → an invalid module.
        RecordFieldAbi::Variant(cases) => {
            out.push(wasm_abi::CORE_I32); // the discriminant
            let mut join: Option<u8> = None;
            for pv in cases.iter().filter_map(|(_, p)| *p) {
                let cb = pv.core_byte();
                join = Some(match join {
                    None => cb,
                    Some(prev) if prev == cb => cb,
                    Some(a)
                        if (a == wasm_abi::CORE_I32 && cb == wasm_abi::CORE_I64)
                            || (a == wasm_abi::CORE_I64 && cb == wasm_abi::CORE_I32) =>
                    {
                        wasm_abi::CORE_I64
                    }
                    Some(_) => cb, // a float mix is excluded by the detector; keep last defensively
                });
            }
            if let Some(cb) = join {
                out.push(cb);
            }
        }
    }
}

fn host_import_functype(f: &crate::backend::wasm::host::HostImport) -> Vec<u8> {
    use crate::backend::wasm::host::HostParam;
    let mut item = vec![0x60];
    let mut params = Vec::new();
    for p in &f.params {
        match p {
            HostParam::Scalar(v) => params.push(v.core_byte()),
            // Str AND Bytes both cross as `(ptr: i32, len: i32)` at the CORE level — 2 i32 slots. (Only the
            // COMPONENT boundary type differs: string inline vs a `list<u8>` type-index; see mod.rs.)
            HostParam::Str | HostParam::Bytes => {
                params.extend_from_slice(&[wasm_abi::CORE_I32, wasm_abi::CORE_I32])
            }
            // A RECORD param (shape d, all-scalar fields) FLATTENS to one core slot per field, in field
            // order (the component `record` type declares its fields in the same NAME-LEX order). The
            // component boundary type is a `record` DEFINED type (see mod.rs `host_op_comp_functype`); the
            // CORE marshalling is just the scalar slots the guest decomposes the record into.
            HostParam::Record(field_abis) => {
                for (_, f) in field_abis {
                    flatten_record_field_abi(f, &mut params);
                }
            }
            // An ENUM param crosses as ONE `i32` core slot — the discriminant (a payloadless enum's in-guest
            // rep). The component boundary type is an `enum` DEFINED type (see mod.rs `host_op_comp_functype`).
            HostParam::Enum(_) => params.push(wasm_abi::CORE_I32),
            // A `list<T>` param crosses as `(ptr: i32, count: i32)` — 2 core slots (like `Bytes`'s `(ptr,len)`,
            // count in place of len). The component boundary type is a `(list <elem>)` DEFINED type.
            HostParam::List(_) => {
                params.extend_from_slice(&[wasm_abi::CORE_I32, wasm_abi::CORE_I32])
            }
            // A bare scalar-payload VARIANT param flattens (canonical variant flatten) to `(disc:i32,
            // join(case payloads))` — the same core shape as a `RecordFieldAbi::Variant` field, now at the
            // param position. The join slot is the widest core int (i64 if any 64-bit case, else i32) or the
            // uniform float; it MUST match `wit_ctype::flatten_variant` (the component type) and the guest
            // push (`select::variant_register_join_vt`). The component boundary type is a `variant` DEFINED
            // type (see mod.rs `host_op_comp_functype`).
            HostParam::Variant(cases) => {
                params.push(wasm_abi::CORE_I32); // the discriminant
                let mut join: Option<u8> = None;
                for pv in cases.iter().filter_map(|(_, p)| *p) {
                    let cb = pv.core_byte();
                    join = Some(match join {
                        None => cb,
                        Some(prev) if prev == cb => cb,
                        Some(a)
                            if (a == wasm_abi::CORE_I32 && cb == wasm_abi::CORE_I64)
                                || (a == wasm_abi::CORE_I64 && cb == wasm_abi::CORE_I32) =>
                        {
                            wasm_abi::CORE_I64
                        }
                        Some(_) => cb, // a float mix is excluded by the detector; keep last defensively
                    });
                }
                if let Some(cb) = join {
                    params.push(cb);
                }
            }
            // A top-level `option<scalar>` param flattens (canonical variant flatten) to `(disc:i32,
            // flatten(payload))` — the disc slot then the payload's own flattened slots (one scalar this
            // increment) — IDENTICAL to a `RecordFieldAbi::Option` field (`flatten_record_field_abi`). The
            // component boundary type is the built-in `option<T>` (see mod.rs `host_op_comp_functype`).
            HostParam::Option(payload) => {
                params.push(wasm_abi::CORE_I32); // the discriminant
                flatten_record_field_abi(payload, &mut params);
            }
            // A top-level `tuple<scalar…>` param flattens its elements POSITIONALLY INLINE — one scalar core
            // slot per element in element order, no discriminant (unlike Option) — IDENTICAL to a
            // `RecordFieldAbi::Tuple` field (`flatten_record_field_abi`). The component boundary type is the
            // built-in `tuple<T…>` (see mod.rs `host_op_comp_functype`).
            HostParam::Tuple(elems) => {
                for e in elems {
                    flatten_record_field_abi(e, &mut params);
                }
            }
        }
    }
    // `params` now holds exactly the FLATTENED core-slot bytes (a scalar = 1, a string/bytes = 2, a record
    // = one per SCALAR field / two per BYTES field), so its length IS the core param-slot count.
    let mut slot_count = params.len();
    // A compound `option<list<u8>>` RESULT (S0) is returned via a caller-provided RETPTR: the canonical ABI
    // lowers a >1-flat result to a TRAILING i32 return-pointer param (the callee writes the flattened
    // `(disc, listptr, listlen)` there) and the core function returns NOTHING. Mirrors the runtime's nfc
    // string->string lower core sig `(ptr, len, retptr) -> ()` (store 9a5728f5 core type 2).
    if f.spilled_result.is_some() {
        params.push(wasm_abi::CORE_I32);
        slot_count += 1;
    }
    item.extend_from_slice(&wasm_vec(slot_count, &params));
    if f.spilled_result.is_some() {
        item.extend_from_slice(&wasm_vec(0, &[])); // no core result — written via the retptr param
    } else if f.enum_result.is_some() {
        // A payloadless `enum` result crosses BY VALUE as ONE `i32` (the discriminant) — like a scalar at the
        // core level (no retptr), but its COMPONENT result type is the `enum` DEFINED type (the op's result_cref).
        item.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
    } else {
        match f.result {
            Some(r) => item.extend_from_slice(&wasm_vec(1, &[r.core_byte()])),
            None => item.extend_from_slice(&wasm_vec(0, &[])),
        }
    }
    item
}

/// One core import item for a HOST op: `<mod-len>"host" <name-len><name> 00 <typeidx>` — imported from
/// module `"host"` (the name the component's host-instance is bound under), the op resolved by its name.
fn host_import_item(op_name: &str, type_idx: u32) -> Vec<u8> {
    const HOST_MODULE: &str = "host";
    let mut item = uleb_bytes(HOST_MODULE.len() as u64);
    item.extend_from_slice(HOST_MODULE.as_bytes());
    item.extend_from_slice(&uleb_bytes(op_name.len() as u64));
    item.extend_from_slice(op_name.as_bytes());
    item.push(0x00); // import desc: func
    uleb128(type_idx as u64, &mut item);
    item
}

/// The core functype `0x60 <params> <result>` of a cross-component extern op (X4b) — its parameter/result
/// core valtypes. A scalar crosses by value; a runtime-owned COMPOUND crosses as its `u32` handle (an i32
/// core valtype; X5b/U5), so the core functype is uniform over both.
fn extern_import_functype(f: &crate::backend::wasm::host::ExternImport) -> Vec<u8> {
    let mut item = vec![0x60];
    let params: Vec<u8> = f.params.iter().map(|v| v.core_byte()).collect();
    item.extend_from_slice(&wasm_vec(params.len(), &params));
    match &f.result {
        Some(v) => {
            let r = [v.core_byte()];
            item.extend_from_slice(&wasm_vec(1, &r));
        }
        None => item.extend_from_slice(&wasm_vec(0, &[])),
    }
    item
}

/// One core import item for a cross-component extern op — imported from module `"peer"` (the name the
/// consumer's peer-instance is bound under), the op resolved by its name (X4b).
fn extern_import_item(op_name: &str, type_idx: u32) -> Vec<u8> {
    const PEER_MODULE: &str = "peer";
    let mut item = uleb_bytes(PEER_MODULE.len() as u64);
    item.extend_from_slice(PEER_MODULE.as_bytes());
    item.extend_from_slice(&uleb_bytes(op_name.len() as u64));
    item.extend_from_slice(op_name.as_bytes());
    item.push(0x00); // import desc: func
    uleb128(type_idx as u64, &mut item);
    item
}

/// Serialize one flat instruction, appending its bytes to `out`. `import_index` maps a runtime op's
/// name to its core function index (its position `0..k` in the import section), so a `CallImport`
/// resolves by name to the same index the import section assigned. Exhaustive over `Lir`.
///
/// This is the recursive instruction serializer: a `match` over every `Lir` variant, no wildcard arm —
/// so a new instruction variant the serializer does not handle is a compile-time non-exhaustiveness
/// error, never a silent fall-through. It consumes an already-selected `Lir` and only writes bytes: it
/// resolves no name, decides no type, and chooses no effect handler — those decisions were all made by
/// earlier phases, so emission is the pure serialization of decisions already made.
//= spec/capabilities/compiler-pipeline.md#the-compiler-operates-on-ast-values
//# The compiler MUST serialize instruction values to bytes through a recursive function that pattern-matches the instruction sum type exhaustively over its variants, so that an instruction variant the serializer does not handle is a compile-time error rather than a silent fall-through.
//= spec/capabilities/compiler-pipeline.md#emission-serializes-a-lowered-representation
//# The step that emits instruction bytes MUST consume an already-lowered representation and MUST NOT itself resolve a name, decide a type, or choose an effect's handler, so that emission is the serialization of decisions already made.
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
        // `f64.const` — the opcode then the 8 raw bit-pattern bytes, little-endian (NOT LEB128; the
        // float-const immediate is a fixed-width IEEE-754 encoding). `bits` already IS the pattern.
        Lir::F64ConstBits(bits) => {
            out.push(op::F64_CONST);
            out.extend_from_slice(&bits.to_le_bytes());
        }
        // `f32.const` — the opcode then the 4 raw bit-pattern bytes, little-endian (fixed-width, NOT LEB).
        Lir::F32ConstBits(bits) => {
            out.push(op::F32_CONST);
            out.extend_from_slice(&bits.to_le_bytes());
        }
        // Float arithmetic / equality / width conversion — a single opcode byte each (operands on the
        // stack), from the generated table. Mirrors the integer arith serialization.
        Lir::F64Add => out.push(op::F64_ADD),
        Lir::F64Sub => out.push(op::F64_SUB),
        Lir::F64Mul => out.push(op::F64_MUL),
        Lir::F64Div => out.push(op::F64_DIV),
        Lir::F32Add => out.push(op::F32_ADD),
        Lir::F32Sub => out.push(op::F32_SUB),
        Lir::F32Mul => out.push(op::F32_MUL),
        Lir::F32Div => out.push(op::F32_DIV),
        Lir::F64Eq => out.push(op::F64_EQ),
        Lir::F64Ne => out.push(op::F64_NE),
        Lir::F32Eq => out.push(op::F32_EQ),
        Lir::F32Ne => out.push(op::F32_NE),
        Lir::F32DemoteF64 => out.push(op::F32_DEMOTE_F64),
        Lir::F64PromoteF32 => out.push(op::F64_PROMOTE_F32),
        Lir::I64ReinterpretF64 => out.push(op::I64_REINTERPRET_F64),
        Lir::I32ReinterpretF32 => out.push(op::I32_REINTERPRET_F32),
        Lir::F64Lt => out.push(op::F64_LT),
        Lir::F64Le => out.push(op::F64_LE),
        Lir::F64Gt => out.push(op::F64_GT),
        Lir::F64Ge => out.push(op::F64_GE),
        Lir::F32Lt => out.push(op::F32_LT),
        Lir::F32Le => out.push(op::F32_LE),
        Lir::F32Gt => out.push(op::F32_GT),
        Lir::F32Ge => out.push(op::F32_GE),
        Lir::F64ConvertI64S => out.push(op::F64_CONVERT_I64_S),
        Lir::F32ConvertI64S => out.push(op::F32_CONVERT_I64_S),
        Lir::LocalGet(idx) => {
            out.push(op::LOCAL_GET);
            uleb128(*idx as u64, out);
        }
        Lir::LocalSet(idx) => {
            out.push(op::LOCAL_SET);
            uleb128(*idx as u64, out);
        }
        // `i32.store8` — opcode then the memarg `(align, offset)` as two ulebs. align=0 (byte store, no
        // alignment); offset is the static displacement. Stack (already pushed): [addr, val]. Writes the
        // low byte of `val` to memory 0 at `addr + offset`.
        Lir::I32Store8 { offset } => {
            out.push(op::I32_STORE8);
            uleb128(0, out); // align (log2) = 0
            uleb128(*offset as u64, out); // offset
        }
        Lir::I32Store { offset } => {
            out.push(op::I32_STORE);
            uleb128(2, out); // align (log2) = 2 (natural i32 alignment)
            uleb128(*offset as u64, out); // offset
        }
        Lir::I64Store { offset } => {
            out.push(op::I64_STORE);
            uleb128(3, out); // align (log2) = 3 (natural i64 alignment)
            uleb128(*offset as u64, out);
        }
        Lir::F32Store { offset } => {
            out.push(op::F32_STORE);
            uleb128(2, out); // align (log2) = 2
            uleb128(*offset as u64, out);
        }
        Lir::F64Store { offset } => {
            out.push(op::F64_STORE);
            uleb128(3, out); // align (log2) = 3
            uleb128(*offset as u64, out);
        }
        Lir::I32Store16 { offset } => {
            out.push(op::I32_STORE16);
            uleb128(1, out); // align (log2) = 1 (2-byte)
            uleb128(*offset as u64, out);
        }
        Lir::I32Load { offset } => {
            out.push(op::I32_LOAD);
            uleb128(2, out); // align (log2) = 2 (natural i32 alignment)
            uleb128(*offset as u64, out); // offset
        }
        Lir::I32Load8U { offset } => {
            out.push(op::I32_LOAD8_U);
            uleb128(0, out); // align (log2) = 0
            uleb128(*offset as u64, out); // offset
        }
        Lir::I64Load { offset } => {
            out.push(op::I64_LOAD);
            uleb128(3, out); // align (log2) = 3 (natural i64)
            uleb128(*offset as u64, out);
        }
        Lir::F32Load { offset } => {
            out.push(op::F32_LOAD);
            uleb128(2, out); // align (log2) = 2 (natural f32)
            uleb128(*offset as u64, out);
        }
        Lir::F64Load { offset } => {
            out.push(op::F64_LOAD);
            uleb128(3, out); // align (log2) = 3 (natural f64)
            uleb128(*offset as u64, out);
        }
        Lir::I32Load8S { offset } => {
            out.push(op::I32_LOAD8_S);
            uleb128(0, out); // align (log2) = 0
            uleb128(*offset as u64, out);
        }
        Lir::I32Load16S { offset } => {
            out.push(op::I32_LOAD16_S);
            uleb128(1, out); // align (log2) = 1 (natural i16)
            uleb128(*offset as u64, out);
        }
        Lir::I32Load16U { offset } => {
            out.push(op::I32_LOAD16_U);
            uleb128(1, out); // align (log2) = 1 (natural i16)
            uleb128(*offset as u64, out);
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
        Lir::CallIndirect(type_index) => {
            // `call_indirect <type> <table>`: the opcode, then the functype's TYPE-section index, then
            // the table index (always 0 — the one funcref table). The arguments and the table-slot i32
            // are already on the stack (the slot is popped as the indirection index).
            out.push(op::CALL_INDIRECT);
            uleb128(*type_index as u64, out);
            uleb128(0, out); // table index 0
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
        Lir::CallHostImport(index) => {
            // A host import occupies core-func index `index` — the host-import set is laid FIRST in the
            // core module's import section (this increment is host-ONLY, so no runtime ops precede it, and
            // the index is exactly the op's position in the host-import set). `call <index>`.
            out.push(op::CALL);
            uleb128(*index as u64, out);
        }
        Lir::CallExternImport(index) => {
            // A cross-component extern (peer) import occupies core-func index `index` — the serializer lays
            // the peer imports FIRST (`0..e`, ahead of the runtime ops; X5's extern-first order), so the
            // index is exactly the op's position in the extern-import set. `call <index>`. (An extern op
            // never coexists with a host effect — that fusion declines upstream — so no host base shifts it.)
            out.push(op::CALL);
            uleb128(*index as u64, out);
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
        Lir::Return => out.push(op::RETURN),
        Lir::Drop => out.push(op::DROP),
        // `if (empty) unreachable end` — trap when the i32 condition is nonzero, leaving nothing.
        Lir::IfUnreachableEnd => {
            out.push(op::IF);
            out.push(wasm_abi::BLOCK_EMPTY); // empty block type
            out.push(op::UNREACHABLE);
            out.push(op::END);
        }
        // `if (empty) i32.const INT_MIN ; i32.const -1 ; i32.div_s ; drop ; end` — trap with wasm's
        // native INTEGER-OVERFLOW reason when the i32 condition is nonzero, leaving nothing. `i32.div_s`
        // of `i32::MIN / -1` is the one arithmetic op that traps as "integer overflow" (a bare
        // `unreachable` reports only "unreachable"), so a runtime integer overflow surfaces its kind. The
        // `drop` balances the block's stack (the divide result never materializes — the op traps first).
        Lir::IfIntegerOverflowEnd => {
            out.push(op::IF);
            out.push(wasm_abi::BLOCK_EMPTY); // empty block type
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(i32::MIN as i64, out);
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(-1, out);
            out.push(op::I32_DIV_S);
            out.push(op::DROP);
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

/// The byte offset of each `Lir` instruction WITHIN this function's `code_entry` (relative to the
/// entry's first byte — the size prefix), one per `f.code` index. Used for per-construct debug line
/// rows (`DESIGN-debug-line-granularity-rcdzc.md`): an absolute DWARF code offset is `code_base +
/// FuncCodeRange.code_start + instr_offsets(f)[lir_index]`. Replays the exact `code_entry` byte layout
/// (size prefix + local decls, then the instructions), recording the running offset before each
/// instruction — so it stays byte-exact with the emitted entry. `imports` fixes the `CallImport` map.
pub fn instr_offsets(f: &SelectedFunc, imports: &[&RtOp]) -> Vec<u32> {
    let import_index: std::collections::HashMap<&str, u32> = imports
        .iter()
        .enumerate()
        .map(|(i, o)| (o.name, i as u32))
        .collect();
    // The inner stream = local decls, then instructions. Track each instruction's position within it.
    let mut inner = Vec::new();
    let groups = rle(&f.declared);
    uleb128(groups.len() as u64, &mut inner);
    for (count, vt) in groups {
        uleb128(count as u64, &mut inner);
        inner.push(vt.byte());
    }
    let mut offsets = Vec::with_capacity(f.code.len());
    for i in &f.code {
        offsets.push(inner.len() as u32); // position of this instruction within the inner stream
        instr(i, &import_index, &mut inner);
    }
    // Every instruction sits after the entry's size-prefix uleb (`uleb(inner_total_len)`); shift by it.
    let prefix_len = uleb_bytes(inner.len() as u64).len() as u32;
    for off in &mut offsets {
        *off += prefix_len;
    }
    offsets
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

/// The core functype `0x60 <params-vec> <results-vec>` of an EXTRA closure-application signature
/// (`Layout::closure_call_types`) — its param valtypes are ALREADY the full `(env:i32, args…)` list, and
/// its result is a solved type (unit → zero results). The same wire shape `functype` emits, but taking a
/// pre-computed param-valtype list + result type rather than a `SelectedFunc` (there is no function body —
/// this functype exists only so a `call_indirect` to a never-built closure type has a type-section index).
fn closure_call_functype(
    param_vts: &[crate::backend::wasm::lir::ValType],
    ret: &Ty,
) -> Result<Vec<u8>, String> {
    let mut out = vec![wasm_abi::CORE_FUNCTYPE_FORM];
    let param_bytes: Vec<u8> = param_vts.iter().map(|vt| vt.byte()).collect();
    out.extend_from_slice(&wasm_vec(param_bytes.len(), &param_bytes));
    match valtype_of(ret) {
        Some(vt) => out.extend_from_slice(&wasm_vec(1, &[vt.byte()])),
        None if matches!(ret, Ty::Unit) => out.extend_from_slice(&wasm_vec(0, &[])),
        None => {
            return Err(
                "closure application result type has no machine representation".to_string(),
            );
        }
    }
    Ok(out)
}

/// One defined function's emitted CODE byte range, paired with the source occurrence it derives from —
/// the D1b line-table primitive (`DESIGN-debug-info-rcdzc.md` §2.1b). `code_start`/`code_end` are byte
/// offsets of the function's `code_entry` (its `<size><locals><instrs>end` bytes) RELATIVE TO THE START
/// OF THE CODE-SECTION PAYLOAD — i.e. relative to the first byte after the code section's id + length
/// prefix. `src` is the function's `src_body` occurrence (`None` for a synthesized escape walker). The
/// compose chain the DWARF line program needs is `code_offset → src → span → (file, line, col)`, where
/// `span` comes from the `spans` sidecar and the ABSOLUTE code offset is `code_section_base +
/// code_start` (D2 adds the base; the base is the code-section-payload's offset in the module, known
/// once the earlier sections are laid). Computed by a PURE re-derivation of `core_module`'s code
/// layout, so it stays exactly in sync without threading state through the emit family or touching the
/// executed-section bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FuncCodeRange {
    pub src: Option<crate::ast::StructId>,
    pub code_start: u32,
    pub code_end: u32,
}

/// The per-function code byte ranges within the code-section payload — the D1b `code_offset → StructId`
/// line table (§2.1b). A PURE function of the same `funcs`/`imports` slice `core_module` serializes,
/// re-deriving each function's `code_entry` size in emission order so the ranges match the emitted code
/// section byte-for-byte. `imports` is unused for offsets (the code section holds only DEFINED bodies)
/// but taken for signature symmetry with `core_module` and to make the "same inputs" contract explicit.
/// This is the inert internal value D1b lands: nothing in the executed bytes changes, and D2 turns these
/// ranges (+ the `spans` sidecar) into the `.debug_line` program. A body whose `functype` has no machine
/// rep would have already declined in `core_module`, so `code_entry` here never faults.
/// One function's code-section entry bytes (`<size><locals><instrs>end`) — the public witness of the
/// per-function byte length [`code_ranges`] and `core_module` both derive from `code_entry`. Exposed so
/// tests (and D2's line-program builder) can assert a range's length against the real entry. `imports`
/// fixes the `import_index` a `CallImport` resolves against (same as `core_module`).
pub fn code_entry_bytes(f: &SelectedFunc, imports: &[&RtOp]) -> Vec<u8> {
    let import_index: std::collections::HashMap<&str, u32> = imports
        .iter()
        .enumerate()
        .map(|(i, o)| (o.name, i as u32))
        .collect();
    code_entry(f, &import_index)
}

pub fn code_ranges(funcs: &[SelectedFunc], imports: &[&RtOp]) -> Vec<FuncCodeRange> {
    // The code-section PAYLOAD begins with the function COUNT (a `wasm_vec` count uleb), then each
    // function's `code_entry` bytes concatenated. So the first entry starts past that count prefix.
    let _ = imports; // offsets depend only on the defined bodies; imports affect call INDICES, not sizes
    let import_index: std::collections::HashMap<&str, u32> = imports
        .iter()
        .enumerate()
        .map(|(i, o)| (o.name, i as u32))
        .collect();
    let mut offset = uleb_bytes(funcs.len() as u64).len() as u32;
    let mut ranges = Vec::with_capacity(funcs.len());
    for f in funcs {
        let entry = code_entry(f, &import_index);
        let code_start = offset;
        let code_end = offset + entry.len() as u32;
        ranges.push(FuncCodeRange {
            src: f.src_body,
            code_start,
            code_end,
        });
        offset = code_end;
    }
    ranges
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
pub fn name_section(module_name: &str, func_names: &[(u32, String)]) -> Vec<u8> {
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
) -> Result<Vec<u8>, String> {
    core_module_impl(funcs, imports, &[], &[], layout, &[], false)
}

/// [`core_module`] plus boundary WRAPPER funcs (typed WIT interface-export emit, W4c): each [`WrapperDesc`]
/// is appended as a `(flattened) -> result` core func that builds a def's `record` param from the flattened
/// boundary leaves, calls the def, and returns its result — exported under the member name (shadowing the
/// def's export). For a guest exporting an interface whose funcs carry records. `imports` must already
/// include the rebuild ops (`arr-alloc`/`arr-set`/`box-*`) the wrappers use.
pub fn core_module_with_wrappers(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    host_fns: &[crate::backend::wasm::host::HostImport],
    wrappers: &[WrapperDesc],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    core_module_impl(funcs, imports, host_fns, &[], layout, wrappers, false)
}

/// [`core_module_with_wrappers`] for the PURE-REDUCER BULK path (operator seq-1024/1026): a pure reducer (no
/// host ops) whose typed interface marshals `list<u8>` gets the IMPORT-realloc / shared-`mem` shape — it
/// imports `"mem"`.`"mem"` + `"mem"`.`"cabi_realloc"` (instead of defining its own) so the bulk
/// `bytes-new`/`bytes-read` canon-lower with Memory+Realloc against the shared allocator provided by the mem
/// module BEFORE the program instance (the instantiation-order fix). Pair with
/// [`crate::backend::wasm::envelope::assemble_typed_interface_with_host_runtime_mem`] (zero host groups,
/// `needs_realloc = true`). Only call when [`wrappers_use_bytes`] holds.
pub fn core_module_with_wrappers_bulk(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    wrappers: &[WrapperDesc],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    core_module_impl(funcs, imports, &[], &[], layout, wrappers, true)
}

/// Whether a typed interface's boundary wrappers marshal any `list<u8>`/`Bytes` (a bytes-leaf param, a
/// `CopyBytes`/`SpillRecord` result, or a top-level memory-bearing leaf param) — i.e. whether the pure
/// reducer needs the bulk `bytes-new`/`bytes-read` shared-`mem` shape. MUST match `core_module_impl`'s
/// `wrapper_needs_memory` predicate (the two are kept in lockstep: `force_realloc` coincides with it).
pub fn wrappers_use_bytes(wrappers: &[WrapperDesc]) -> bool {
    wrappers.iter().any(|w| {
        w.params
            .iter()
            .flatten()
            .flatten()
            .any(FieldRebuild::has_bytes_leaf)
            || matches!(w.result, ResultLower::SpillRecord { .. })
            || matches!(w.result, ResultLower::CopyBytes)
            || w.mem_leaf_params.iter().any(Option::is_some)
    })
}

/// Whether a wrapper's flattened boundary params SPILL to a memory-indirect single-pointer core param: the
/// canonical ABI passes MORE than [`crate::backend::wasm::wit_ctype::MAX_FLAT_PARAMS`] flat core values by
/// having the caller write them to linear memory and pass ONE `i32` pointer, which the wrapper reads back
/// (wfp1: 17 scalar params). Keyed off the flattened core param count — the same threshold
/// [`crate::backend::wasm::wit_ctype::sig_needs_memory`] uses, so the envelope assembler (which binds the
/// canon-lift Memory+Realloc off `sig_needs_memory`) and this core-side reader agree.
pub(crate) fn wrapper_params_spill(param_vts: &[u8]) -> bool {
    param_vts.len() > crate::backend::wasm::wit_ctype::MAX_FLAT_PARAMS
}

/// The canonical spilled-param-area layout for a wrapper whose params spill: each flattened core param laid
/// out at its NATURALLY-ALIGNED byte offset (the canonical memory layout of a tuple of the flattened params —
/// `i32`/`f32` are 4-byte 4-aligned, `i64`/`f64` are 8-byte 8-aligned). Returns, per flat leaf in order, the
/// `(offset, load_op, align_log2)` the wrapper reads it with (`local.get 0` = the spill pointer, then this
/// load). Parallel to `param_vts`.
fn spilled_param_layout(param_vts: &[u8]) -> Vec<(u32, u8, u32)> {
    use crate::backend::wasm::wasm_abi::{CORE_F32, CORE_F64, CORE_I64, op};
    let mut out = Vec::with_capacity(param_vts.len());
    let mut off = 0u32;
    for &vt in param_vts {
        let (size, load, align) = match vt {
            CORE_I64 => (8u32, op::I64_LOAD, 3u32),
            CORE_F32 => (4, op::F32_LOAD, 2),
            CORE_F64 => (8, op::F64_LOAD, 3),
            // CORE_I32 and any other single-slot value read as i32.
            _ => (4, op::I32_LOAD, 2),
        };
        off = off.next_multiple_of(size); // natural alignment == the value's own size here
        out.push((off, load, align));
        off += size;
    }
    out
}

/// [`core_module`] with a leading CROSS-COMPONENT extern-import set (X4b): `extern_fns` are peer ops
/// imported from module `"peer"`, laid FIRST (core-func indices `0..e`; the extern-first order), so a
/// `Lir::CallExternImport(i)` resolves to `i`. This entry is the extern-ONLY program (`host_fns` and
/// `imports` empty); an extern + value-heap runtime consumer uses [`core_module_with_extern_runtime`].
pub fn core_module_with_extern(
    funcs: &[SelectedFunc],
    extern_fns: &[crate::backend::wasm::host::ExternImport],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    core_module_impl(funcs, &[], &[], extern_fns, layout, &[], false)
}

/// [`core_module`] with BOTH a peer extern-import set AND the value-heap runtime (X5): peer ops from
/// module `"peer"` at core funcs `0..e`, runtime ops from `"heap"` at `e..e+k`. For a consumer that
/// receives a compound `value` handle from a peer and INSPECTS it (a projection imports runtime ops).
pub fn core_module_with_extern_runtime(
    funcs: &[SelectedFunc],
    extern_fns: &[crate::backend::wasm::host::ExternImport],
    imports: &[&RtOp],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    core_module_impl(funcs, imports, &[], extern_fns, layout, &[], false)
}

/// [`core_module`] with a leading HOST-import set (E2h-2): `host_fns` are host-delegated ops imported
/// from module `"host"`, occupying core-func indices `0..h` AHEAD of the runtime ops and defined funcs.
/// The host-only scope (this increment) means `imports` is empty when `host_fns` is non-empty, but the
/// layout is uniform (host first, then runtime, then defined). Kept a separate entry so the runtime path
/// is byte-identical.
pub fn core_module_with_host(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    host_fns: &[crate::backend::wasm::host::HostImport],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    core_module_impl(funcs, imports, host_fns, &[], layout, &[], false)
}

/// A boundary WRAPPER core function to APPEND to the emitted module — a `(flattened params) -> result` func
/// that builds each guest value-heap value from the flattened boundary params, calls the compiled def, and
/// returns its result. Emitted for a typed WIT interface-export member whose param is a `record` (the canon
/// lift hands the record's flattened fields, but the compiled def wants a value-heap handle). The wrapper's
/// export NAME shadows the def's — the interface aliases the wrapper, not the def, so the def stays internal.
/// The wrapper appends AFTER all defined + lifted funcs, so no existing index shifts.
pub struct WrapperDesc {
    /// The exported member name (the interface aliases this → the wrapper).
    pub name: String,
    /// The wrapper's core param valtypes — the flattened boundary params (a `record` param flattens to its
    /// fields' core valtypes, in order).
    pub param_vts: Vec<u8>,
    /// The wrapper's core result valtypes — the flattened boundary result (a scalar = one valtype, `unit` =
    /// none). Passed straight through from the def's result (a scalar leaves the def raw).
    pub result_vts: Vec<u8>,
    /// Per DEF param, how to build it from the flattened boundary leaves: a `record` param is
    /// `Some(fields)` (built via [`emit_cell_rebuild`]); a scalar param is `None` (a `local.get` passthrough).
    /// In def-param order; the flattened-leaf cursor advances across all of them. A record param's `fields`
    /// are in WIT/flattened-param ORDER (so the cursor consumes them sequentially).
    pub params: Vec<Option<Vec<FieldRebuild>>>,
    /// Parallel to `params`: for a `record` param whose WIT field order ≠ the guest's name-lex slot order,
    /// `Some(slots)` gives each WIT field's target cell SLOT (name-lex position); `None` = identity (a
    /// name-lex-ordered record, or a scalar param). The permute for a declaration-ordered WIT record.
    pub param_slots: Vec<Option<Vec<u32>>>,
    /// Parallel to `params`: for a TOP-LEVEL memory-bearing leaf param (a `String`/`Bytes` crossing the
    /// boundary as `string`/`list<u8>`, flattened to `(ptr, len)`), `Some((kind, drop_after))` says to lift
    /// it — copy the bytes out of linear memory into a value-heap byte-leaf handle (like a
    /// [`FieldRebuild::BytesLeaf`], but the handle is passed DIRECTLY as the def arg, not stored into a
    /// record cell; a Cadenza `String` IS a UTF-8 byte-leaf, so no decode). `drop_after` = the def only
    /// BORROWS this param (does not consume it), so the wrapper — which OWNS the lifted value — must `drop`
    /// it after the call (the borrowed-owned-operand reclaim; a consuming def, e.g. `String.concat`, takes
    /// ownership and sets this false so the wrapper does not double-free). `None` = a scalar/`record` param
    /// (handled via `params`). Reuses the two `list<u8>` scratch locals + memory 0 (`wrapper_needs_memory`).
    pub mem_leaf_params: Vec<Option<(MemLeafKind, bool)>>,
    /// The shape-descriptor bytes for each [`MemLeafKind::ValueForm`] param, indexed by the variant's `u32`.
    /// A value-form param (`BigInt`/`Rational`/`Symbol`) crosses as `list<u8>` and is reconstructed by
    /// `value-decode(bytes, desc)`; `desc` is baked from these bytes (the same descriptor `Value.encode`/
    /// `Value.decode` compute — via `value_form_param_descriptor`). Empty when no param is a value-form leaf
    /// (every other route leaves this untouched — a byte-neutral passthrough).
    pub value_form_descs: Vec<Vec<u8>>,
    /// Parallel to `params`: for a TOP-LEVEL `option`/`result` param (a two-variant sum crossing the boundary
    /// as a native `option<T>`/`result<ok,err>`, flattened to `(disc, payload…)`), `Some(rebuild)` says to
    /// branch on the boundary disc and build the guest sum cell (`sum-new`) — leaving the handle DIRECTLY as
    /// the def arg (not stored into a record cell). Reuses the closure-arg [`SumArgRebuild`] via
    /// [`emit_sum_field`], which reads the disc at the wrapper's running leaf cursor (its `base_param` is
    /// ignored). `None` = a scalar/`record`/mem-leaf param. `Some((rebuild, drop_after))`: `drop_after` = the
    /// def only BORROWS the built cell (matches it to read a copied-out payload, does not consume it), so the
    /// wrapper — its owner — must `drop` it after the call (the same borrowed-owned-operand reclaim as a
    /// mem-leaf param; a consuming def would set this false so the wrapper does not double-free).
    pub sum_params: Vec<Option<(SumArgRebuild, bool)>>,
    /// Parallel to `params`: for a TOP-LEVEL payloadless-ENUM param (a `Ty::Sum` `db.is_enum_disc`, crossing
    /// the boundary as a WIT `enum{…}` whose canonical flatten is a single `i32` case index) whose WIT case
    /// order ≠ the guest decl order, `Some(inv_perm)` remaps the boundary disc to the guest disc BY NAME
    /// (`inv_perm[wit_disc] = guest_disc`) via a nested-if chain, leaving the guest disc as the def arg — the
    /// PARAM twin of `ResultLower::EnumRemap` (#7036, SHAPE 64). An ORDER-MATCHING enum param is `None` (the
    /// boundary `i32` disc IS the guest disc — a plain `local.get` passthrough via the `params` `None` arm);
    /// a non-enum param is also `None`. An enum-disc value is a raw `i32` (no heap handle), so there is no
    /// reclaim/`drop_after` — pure `i32` compare/select, no runtime op, no memory.
    pub enum_disc_params: Vec<Option<Vec<u32>>>,
    /// Parallel to `params`: for a `record`/tuple-cell param (`params[i] = Some(fields)`), whether the wrapper
    /// must `drop` the rebuilt cell AFTER the def call — the #9014 envelope-decode shell-drop. The wrapper
    /// allocates a FRESH cell (`emit_cell_rebuild`) and passes it BORROWED to the def; when safe (the dup-aware
    /// `select::record_cell_param_droppable` gate: every consuming occurrence of the cell binder is a Perceus
    /// dup_site, so a forwarded field was dup'd and survives the cell's deep-drop cascade) it is a dead owned
    /// temporary after the call, so the wrapper (its owner) deep-drops it — reclaiming the cell + its boxed
    /// fields. `false` for a non-record param (`params[i] = None`, nothing to drop) OR a record param whose
    /// def MOVES a field out verbatim (a last-consume, not a dup_site) — dropping then would double-free. The
    /// record-cell twin of `mem_leaf_params`/`sum_params` `drop_after`.
    pub record_param_drop_after: Vec<bool>,
    /// Parallel to `record_param_drop_after` (28-wit:310 SHAPE-9 shell-reclaim co-fix). For a record/tuple-cell
    /// param whose shell is NOT blanket-droppable *because* compound field(s) move out verbatim, `Some(paths)`
    /// = the MULTISET of escaped-field projection paths (one `Vec<usize>` `arr-get`-index chain per escaping
    /// occurrence, from `select::escaped_field_projections`). The wrapper projects each field off the cell and
    /// `dup`s it (→ rc≥2) BEFORE the def call, then deep-drops the shell after the call (cascade → rc≥1, so the
    /// result's field handle survives) — reclaiming the shell without the escaped-field UAF that a blanket drop
    /// would cause. `None` = keep the all-or-nothing `record_param_drop_after` behavior (mutually exclusive:
    /// `escaped_field_projections` returns `None` when the shell is blanket-droppable).
    pub record_param_escaped_fields: Vec<Option<Vec<Vec<usize>>>>,
    /// The compiled def's absolute core func index to `call` after building its args.
    pub def_abs: u32,
    /// How the wrapper turns the def's return value into the boundary result — pass a scalar straight through,
    /// or read a returned record HANDLE's fields and spill them to a return area in memory.
    pub result: ResultLower,
}

/// Emit a payloadless-enum discriminant REMAP: given a source disc in local `src_local`, leave `map[src]`
/// on the stack via a nested-if chain (`if src == 0 { map[0] } else if src == 1 { map[1] } else … map[n-1]`).
/// Pure wasm — i32 compare/select only, no runtime op, no memory. Shared by the enum RESULT remap
/// (`ResultLower::EnumRemap`, `map = perm[guest] = wit`) and the enum PARAM remap
/// (`WrapperDesc::enum_disc_params`, `map = inv_perm[wit] = guest`). `map` is only ever a genuine reorder
/// (identity is a passthrough that never calls this), so `n ≥ 2` and the chain has a real `else`.
fn emit_enum_disc_remap(map: &[u32], src_local: u32, inner: &mut Vec<u8>) {
    let n = map.len();
    for (i, &dst) in map.iter().enumerate() {
        if i + 1 == n {
            // The final `else` value: map[last].
            inner.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(dst as i64, inner);
        } else {
            // `if src == i { map[i] } else { …next… }`
            inner.push(op::LOCAL_GET);
            uleb128(src_local as u64, inner);
            inner.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(i as i64, inner);
            inner.push(op::I32_EQ);
            inner.push(op::IF);
            inner.push(wasm_abi::CORE_I32); // block returns i32
            inner.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(dst as i64, inner);
            inner.push(op::ELSE);
        }
    }
    // Close each nested `if` (one END per arm but the last): append `n-1` END bytes.
    inner.resize(inner.len() + n.saturating_sub(1), op::END);
}

/// The kind of a TOP-LEVEL memory-bearing leaf param (see [`WrapperDesc::mem_leaf_params`]): a `Bytes`
/// value-heap handle lifted straight from the boundary `(ptr, len)`, or a `String` built from those bytes
/// (`str-from-bytes`). Both copy the bytes out of linear memory 0 (the `bytes-alloc`/`bytes-set` loop the
/// `list<u8>`-leaf import marshal already uses); a `Str` appends one `str-from-bytes` call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemLeafKind {
    /// A `Bytes` param: the copied-out `list<u8>` handle IS the def arg.
    Bytes,
    /// A `String` param: the copied-out UTF-8 byte-leaf IS the String (a Cadenza String is a byte-leaf; no
    /// decode — a WIT `string` param is valid UTF-8). Same lift as `Bytes`; distinct only for the WIT type.
    Str,
    /// A `list<scalar>` (or NESTED `list<list<…<scalar>>>`) param: build a value-heap vec (`vec-empty` +
    /// per-element read/box/`vec-push`) from the canonical `(ptr, len)` layout, rather than a raw byte copy.
    /// The [`ListElem`] carries the scalar leaf's read+box (Int8/16/32/64, UInt*, Float32/64, Bool) plus a
    /// `nest_lists` depth: `0` = a flat `list<scalar>`; `k>0` = a nested list whose elements are `(ptr,len)`
    /// sub-lists recursively lifted `k` levels to the scalar (el8/eln). A `list<u8>` here is a genuine
    /// `List UInt8` value (a vec of boxed u8s), NOT `Bytes` (a packed byte-leaf) — distinct value reps.
    List(ListElem),
    /// A value-form leaf param (`BigInt`/`Rational`/`Symbol`) that has NO scalar boundary rep: it crosses as
    /// the canonical `list<u8>` value-form (the same bytes `Value.encode`/`Value.decode` use), flattened to
    /// `(ptr, len)`. The wrapper copies those bytes out of linear memory, bakes the type's SHAPE DESCRIPTOR
    /// (indexed into [`WrapperDesc::value_form_descs`] by this `u32`), and calls `value-decode(bytes, desc)`
    /// to reconstruct the value-heap handle passed DIRECTLY as the def arg — the param twin of the R2
    /// `Core::ValueDecode` lift (eb1/er1/ey1). The `bytes`/`desc` are BORROWED by `value-decode`; the wrapper
    /// (their owner) drops both inside the lift. The decoded handle is a FRESH OWNED value reclaimed after the
    /// call like any other borrowed mem-leaf param (`drop_after`).
    ValueForm(u32),
}

/// How ONE scalar element of a `list<scalar>` param is read out of linear memory and boxed into the value
/// heap, for [`emit_list_leaf_lift`]. The canonical layout lays element `j` at `ptr + j*stride`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ListElem {
    /// The wasm load opcode (`i64.load`/`i32.load`/`i32.load8_u`/`f64.load`/…) reading the element at its
    /// computed address.
    pub load_op: u8,
    /// The load's natural-alignment memarg (log2 bytes): 0 (byte), 1 (i16), 2 (i32/f32), 3 (i64/f64).
    pub load_align: u32,
    /// The element's canonical byte stride (`canonical_size`): 1/2/4/8 by width.
    pub stride: u32,
    /// `Some(signed)` when a NARROW int (loaded into an i32 slot) must be i32→i64 extended before `box-int`
    /// (which takes i64); `None` for a full-width i64, an f32/f64 (boxed by `box-float`/`box-float32`), or a
    /// bool (`box-bool` takes the i32 directly).
    pub extend: Option<bool>,
    /// The box op that wraps the loaded scalar into a value-heap handle (`box-int`/`box-float`/`box-float32`/
    /// `box-bool`).
    pub box_op: &'static str,
    /// The number of enclosing `list<…>` levels between the outer list's element position and this scalar
    /// leaf — `0` for a flat `list<scalar>` (each element is the scalar itself), `k>0` for a nested
    /// `list<list<…<scalar>>>` (each element is a `(ptr, len)` sub-list at canonical stride 8, recursively
    /// lifted `k` levels down to the scalar). Uniform per level because every `list<T>` has the same
    /// `(ptr: i32, len: i32)` boundary rep regardless of `T`, so the nesting is a depth COUNT, not a tree
    /// (el8/eln1-3). `emit_list_leaf_lift` recurses on this count.
    pub nest_lists: u32,
}

/// How a boundary wrapper produces its result from the value the compiled def returns.
pub enum ResultLower {
    /// Return the def's result unchanged — a scalar the def returns raw (matching the flattened boundary
    /// scalar) or `unit` (no result). `WrapperDesc::result_vts` is that flattened scalar (or empty).
    Passthrough,
    /// The def returns a value-heap compound HANDLE; the boundary result is that compound, which SPILLS to
    /// linear memory — the wrapper allocates a return area (`cabi_realloc`), writes the canonical form via
    /// `write`, and returns the area pointer (`result_vts = [i32]`). Needs memory + `cabi_realloc`
    /// (`wrapper_needs_memory` covers it).
    SpillRecord {
        /// The result's canonical byte size (the `cabi_realloc` request).
        size: u32,
        /// The result's canonical alignment (the `cabi_realloc` align arg).
        align: u32,
        /// The recursive plan for writing the value-heap value's canonical form.
        write: CanonWrite,
    },
    /// The def returns a value-heap `Bytes`/`list<u8>` HANDLE; the boundary result is a `list<u8>` — the
    /// wrapper copies the runtime bytes into a fresh `cabi_realloc`'d buffer and writes the `(ptr, len)`
    /// pair to the retptr'd 8-byte return area (mirrors the single-export bytes-roundtrip copy-out
    /// [`emit_bytes_roundtrip_apply_body`], but the buffer + retarea come from `cabi_realloc` like
    /// [`ResultLower::SpillRecord`]). `result_vts = [i32]` (the retptr). Needs memory + `cabi_realloc`.
    CopyBytes,
    /// A payloadless-`enum` result whose guest decl case-order DIFFERS from the WIT case-order (same case
    /// NAMES, permuted). The def returns the guest's raw i32 disc; the boundary WIT `enum` reads the wire disc
    /// per its OWN order, so the wrapper REMAPS guest-disc → WIT-disc by name before returning it (`perm[i]` =
    /// the WIT case index of the guest's `i`th case). An IDENTITY permutation is just `Passthrough`; this
    /// variant is used only for a genuine reorder (the name-keyed enum-boundary remap — the disc analogue of
    /// the record RESULT's write-by-name field reorder). `result_vts = [i32]`; no memory. (v-rust-backend
    /// WIT-semantics ruling: an enum case-order mismatch is a should-work name-keyed remap, SHAPE 64.)
    EnumRemap { perm: Vec<u32> },
    /// A single-scalar-field RECORD result that flattens to ONE core value (`record{v: s64}` → `[s64]`,
    /// returned DIRECTLY, not by pointer — MAX_FLAT_RESULTS=1). The def returns the record HANDLE; the wrapper
    /// reads that one field off the handle (`arr-get(handle, field_cell)` → unbox `read`, narrowing a ≤32-bit
    /// value) and returns the scalar as the flattened result. `result_vts = [the scalar valtype]`. No memory
    /// (the flat scalar is returned in a register). The flat-1-value-record sibling of `SpillRecord` (which
    /// covers a >1-value record via a retptr).
    FlatScalarField {
        field_cell: u32,
        read: &'static str,
        wrap_i64: bool,
    },
}

/// A recursive plan for writing ONE value-heap value's canonical-ABI form into linear memory — the reducer
/// result-lower (the inverse of the param-side [`FieldRebuild`]). The emitter holds the value's runtime
/// HANDLE and a base address; each node writes at `base + offset`.
pub enum CanonWrite {
    /// A boxed scalar: unbox (`read` = `get-int`/`get-bool`/`get-float`/`get-float32`), optionally narrow the
    /// unboxed i64 to i32 (a ≤32-bit int slot), and `store` (`i64.store`/`i32.store`/`i32.store8`/`…16`/
    /// `f64.store`/`f32.store`).
    Scalar {
        read: &'static str,
        wrap_i64: bool,
        store: u8,
    },
    /// A payloadless `enum` (all-nullary sum): the guest value is a BARE i32 discriminant (`db.is_enum_disc`
    /// — no heap box, unlike a payload-carrying variant which is a heap `sum`). So store that i32 DIRECTLY at
    /// the field offset with NO unbox `read` (a `Scalar`'s `get-int` would wrongly treat the bare disc as a
    /// heap handle). `store` is the disc width (`i32.store8`/`16`/`32` by case count). The guest's raw disc IS
    /// the WIT case index (the arm is gated on decl-order == WIT-case-order, so no remap). The guest-export
    /// result-side twin of a host-import enum arg/result (bare-i32 enum-disc).
    EnumDisc { store: u8 },
    /// A fixed record (an `arr` cell): per field, `arr-get(handle, index)` → write recursively at the field's
    /// canonical offset.
    Record { fields: Vec<CanonField> },
    /// A `list<T>` (a value-heap `vec`): `vec-len` → count; allocate `count * elem_size` bytes
    /// (`cabi_realloc`); per element `vec-get(handle, i)` → write recursively at element stride `elem_size`;
    /// store `(elem_ptr, count)` as the canonical `(ptr, len)`. An empty list stores `(base, 0)`.
    List {
        elem_size: u32,
        elem_align: u32,
        elem: Box<CanonWrite>,
    },
    /// A `list<u8>` (`Bytes`): `bytes-len` → count; allocate `count` bytes; copy them out; store `(ptr, count)`.
    Bytes,
    /// A variant/sum (a value-heap sum cell): `sum-disc` → the guest's DECL disc `d`; branch per arm `k`
    /// (`if d == k`) to store that arm's canonical BOUNDARY disc at the variant base, and, if the arm carries a
    /// payload, `sum-payload` → write it recursively at `payload_offset`. `arms` is indexed by guest decl disc
    /// (the value `sum-disc` returns); `disc_store` is the disc's canonical store width.
    Variant {
        disc_store: u8,
        payload_offset: u32,
        arms: Vec<VariantArm>,
    },
}

/// One arm of a [`CanonWrite::Variant`], indexed by the guest's decl disc. `boundary_disc` is the canonical
/// WIT case index this arm maps to (option: Some→1/None→0; result: Ok→0/Err→1; an aligned user variant maps
/// identity). `payload` is `Some` for a payload-carrying arm, `None` for a nullary one.
pub struct VariantArm {
    pub boundary_disc: u32,
    pub payload: Option<Box<CanonWrite>>,
}

/// One field of a [`CanonWrite::Record`]: read record slot `index` off the handle (`arr-get`) and write it at
/// canonical byte `offset` within the record.
pub struct CanonField {
    pub index: u32,
    pub offset: u32,
    pub write: CanonWrite,
}

fn core_module_impl(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    host_fns: &[crate::backend::wasm::host::HostImport],
    extern_fns: &[crate::backend::wasm::host::ExternImport],
    layout: &Layout,
    wrappers: &[WrapperDesc],
    // PURE-REDUCER BULK (operator seq-1024/1026): a pure reducer (no host ops) whose typed interface carries
    // `list<u8>` marshaling wants the IMPORT-realloc / shared-`mem` shape (like the host `_mem` path) so the
    // bulk `bytes-new`/`bytes-read` canon-lower with Memory+Realloc pre-instantiation. The caller passes this
    // true ONLY for such a wrapper (`wrappers_use_bytes`), so it always coincides with `wrapper_needs_memory`.
    // Host paths pass false → byte-identical (import_realloc still driven by the host spilled-result rule).
    force_realloc: bool,
) -> Result<Vec<u8>, String> {
    let n = funcs.len();
    let h = host_fns.len();
    let e = extern_fns.len();
    let n_wrap = wrappers.len();
    // A host op with a COMPOUND result (`list<u8>`/`option<list<u8>>`/`list<tuple>`) is canon-lowered with a
    // Memory + Realloc option — the host allocates the spilled result into guest memory, which the guest lift
    // reads. That realloc must be available at LOWER-time (before the program core), so it is the SHARED
    // `cabi_realloc` the mem module exports and this core IMPORTS as `"mem"`.`"cabi_realloc"` (a FUNC import,
    // right after the runtime ops), instead of DEFINING its own. One allocator over the one shared memory: the
    // host-op lower + the guest's own retptr/leaf allocs all bump the same cursor. Absent → the wrapper DEFINES
    // its own `cabi_realloc` (the memoryless / list-param-only paths).
    let import_realloc = host_fns.iter().any(|h| h.spilled_result.is_some()) || force_realloc;
    // The realloc IMPORT (when a compound host result is present) occupies a func-import slot after the runtime
    // ops, so a defined func's index (and its type index, kept in lockstep) shifts by +1.
    let import_count = h + imports.len() + e + import_realloc as usize;
    // A wrapper carrying a `list<u8>` leaf reads the list's bytes out of linear memory the canon lift lowered
    // it into, so the core module DEFINES + exports memory 0 and a `cabi_realloc` (a bump allocator over a
    // mutable global) — the canon lift's Memory+Realloc options reference them. `wrapper_needs_memory` gates
    // ALL of this (extra functype/func/code/export + the memory & global sections); false → byte-identical to
    // before. This DEFINE path is distinct from the host-string `needs_memory` IMPORT path (`mem`.`mem`); the
    // two never coexist here (a reducer guest performs no host-string call), so memory 0 has one owner.
    // Memory is needed when a wrapper reads a `list<u8>` leaf out of it (a bytes param) OR writes a spilled
    // record RESULT into it (a `SpillScalarRecord` — the wrapper allocates a return area via `cabi_realloc`).
    let wrapper_needs_memory = wrappers.iter().any(|w| {
        // A wrapper whose params SPILL reads each param out of the memory-indirect spill area (memory 0).
        wrapper_params_spill(&w.param_vts)
            || w.params
                .iter()
                .flatten()
                .flatten()
                .any(FieldRebuild::has_bytes_leaf)
            || matches!(w.result, ResultLower::SpillRecord { .. })
            // A `list<u8>`/Bytes result member allocates its buffer + retarea via `cabi_realloc` too.
            || matches!(w.result, ResultLower::CopyBytes)
            // A TOP-LEVEL memory-bearing leaf param (String/Bytes) reads its bytes out of linear memory too.
            || w.mem_leaf_params.iter().any(Option::is_some)
            // A TOP-LEVEL sum param whose selected arm reads linear memory: a `Bytes` (list<u8>) arm copies
            // bytes out, and a `list<scalar>` arm loads each element (option<bytes>/option<list<…>>).
            || w.sum_params.iter().flatten().any(|(rebuild, _)| {
                let arm_reads_memory = |a: &SumArgArm| {
                    matches!(a.payload, SumArmPayload::Bytes { .. } | SumArmPayload::List(_))
                        || matches!(&a.payload, SumArmPayload::Compound(fs) if fs.iter().any(FieldRebuild::has_bytes_leaf))
                };
                arm_reads_memory(&rebuild.arm_true) || arm_reads_memory(&rebuild.arm_false)
            })
    });
    // A host op with a COMPOUND result also needs the SHARED linear memory (imported `"mem"`.`"mem"`) — the
    // host writes the spilled result there and the guest lift reads it. (Same `import_realloc` condition; the
    // shared cabi_realloc bumps over this shared memory.)
    let host_result_needs_memory = import_realloc;
    // A DEFINED `cabi_realloc` (bump over a defined memory) — only when the wrapper needs memory AND we are NOT
    // importing the shared allocator. In `import_realloc` mode the mem module owns the allocator.
    let n_realloc = (wrapper_needs_memory && !import_realloc) as usize;

    // §2d STATIC BYTES (`DESIGN-static-data.md`): the distinct fully-constant `Bytes` payloads built ONCE
    // into module globals by a `start` init function. Each occupies a mutable-i32 GLOBAL at its table index
    // (`0..n_static`); a defined `cabi_realloc` cursor, when present, follows at `n_static`. The init is one
    // extra DEFINED function laid AFTER the wrappers + realloc (so no existing func/type index shifts): its
    // func index is `import_count + n + n_wrap + n_realloc`, its functype the last defined-function functype.
    // `n_static == 0` → no GLOBAL/START/init additions and the realloc cursor stays global 0, byte-identical.
    let n_static = layout.static_bytes.len();
    // §2d increment 6: the build-once static COMPOUND globals (markable constant Tuple/Record), laid AFTER
    // the byte globals — compound `k`'s global is `n_static + k`. The START init exists if there are static
    // bytes OR static compounds.
    let n_compounds = layout.static_compounds.len();
    let n_init = (n_static > 0 || n_compounds > 0) as usize;
    // The realloc bump cursor's global index: it sits AFTER both the static-bytes AND static-compound
    // globals, so its `global.get`/`global.set` in the defined `cabi_realloc` body address
    // `n_static + n_compounds` (0 when there are no static globals, byte-identical to before).
    let realloc_cursor_global = (n_static + n_compounds) as u64;

    // Type section, then the imports, in ONE fixed order: EXTERN peer functypes FIRST (type indices
    // `0..e`), then HOST (`e..e+h`), then RUNTIME (`e+h..import_count`), then one functype per defined
    // function (`import_count..import_count+n`). Numbering imports' types first keeps a defined func's type
    // index equal to `import_count + its emission position`, which the function section references. Extern
    // and host never coexist (the emit guard forbids extern+host), so an extern-only program keeps host
    // empty (extern at `0..e`, `CallExternImport(i)=call i`) and a host program keeps extern empty (host at
    // `0..h`, `CallHostImport(i)=call i`) — both indices stay valid; extern-first is chosen so extern+RUNTIME
    // lays peer ops before runtime ops, matching the `assemble_extern_runtime` envelope's alias order.
    // Collected as ONE functype byte-seq PER type (in the fixed positional order below) so identical
    // functypes can be DEDUPED into a single type-section entry — most programs import many same-signature
    // heap ops (e.g. several `(i32) -> i32`), and wasm-opt collapses those duplicate functypes; we do the
    // same at emit. `type_remap[old_positional_index] = new_deduped_index` translates every downstream type
    // reference (import descriptors + the function section). See the dedup + guard below the collection.
    let mut type_seqs: Vec<Vec<u8>> = Vec::new();
    for f in extern_fns {
        type_seqs.push(extern_import_functype(f));
    }
    for f in host_fns {
        type_seqs.push(host_import_functype(f));
    }
    for o in imports {
        type_seqs.push(import_functype(o));
    }
    // The `cabi_realloc` IMPORT functype `(i32×4)->i32`, placed WITHIN the import block (right after runtime,
    // type index `e+h+k`) so the `import_count`↔type-index lockstep holds (defined funcs stay at
    // `import_count + i`). Present only in `import_realloc` mode; the DEFINE-mode realloc functype is laid last.
    if import_realloc {
        let mut ft = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        ft.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        ft.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_seqs.push(ft);
    }
    for f in funcs {
        type_seqs.push(functype(f)?);
    }
    // EXTRA closure-application functypes (see `Layout::closure_call_types`): a `call_indirect` applying a
    // closure whose shape NO lifted lambda supplies (a dynamically-dead but statically-emitted variant
    // application) references one of these. Laid AFTER every defined-function functype, so their type
    // indices are `import_count + n + i` — exactly what `Layout::closure_call_type_index` returns
    // (`n == order.len() + lifted.len()`). Empty for a program whose applied closures all have a lifted
    // body (then this loop adds nothing and the type section is byte-identical to before).
    let extra = layout.closure_call_types.len();
    for (param_vts, ret) in &layout.closure_call_types {
        type_seqs.push(closure_call_functype(param_vts, ret)?);
    }
    // Boundary WRAPPER functypes, LAST (type indices `import_count + n + extra + w`): a core functype
    // `(param_vts) -> (result_vts)`. Empty for a program with no wrappers → byte-identical to before.
    for w in wrappers {
        let mut ft = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        if wrapper_params_spill(&w.param_vts) {
            // MEMORY-INDIRECT spill: the flattened params exceed the canonical flat-param cap, so the core
            // func takes a SINGLE `i32` pointer to the spilled param area (the canon lift, with its
            // Memory+Realloc options, writes the params there). The wrapper body reads each back.
            ft.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        } else {
            ft.extend_from_slice(&wasm_vec(w.param_vts.len(), &w.param_vts));
        }
        ft.extend_from_slice(&wasm_vec(w.result_vts.len(), &w.result_vts));
        type_seqs.push(ft);
    }
    // The DEFINE-mode `cabi_realloc` functype LAST (type index `import_count + n + extra + n_wrap`), when a
    // wrapper needs memory AND we are not importing the shared allocator (`n_realloc == 1`): `(i32 old_ptr,
    // i32 old_size, i32 align, i32 new_size) -> i32`. In `import_realloc` mode the functype is the import's
    // (laid in the import block above), so this is absent.
    if n_realloc == 1 {
        let mut ft = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        ft.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        ft.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_seqs.push(ft);
    }
    // The STATIC-BYTES `start` init functype `() -> ()`, LAST (type index `import_count + n + extra +
    // n_wrap + n_realloc`) — the init takes no params and returns nothing (it builds each static bytes and
    // stores the handle in its global). Present only when there are static bytes (`n_init == 1`).
    if n_init == 1 {
        let mut ft = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        ft.extend_from_slice(&wasm_vec(0, &[]));
        ft.extend_from_slice(&wasm_vec(0, &[]));
        type_seqs.push(ft);
    }
    debug_assert_eq!(
        type_seqs.len(),
        import_count + n + extra + n_wrap + n_realloc + n_init
    );
    // DEDUP the collected functypes into distinct entries, recording `type_remap[old_positional_index] =
    // new_deduped_index`. GUARD: dedup ONLY when the program has NO `call_indirect` — i.e. no closure-call
    // functypes (`extra == 0`) AND no lifted lambdas (no funcref table). In that case the ONLY type-index
    // references are the import descriptors + the function section (both remapped below), so the dedup is
    // fully contained here. A closure program bakes its `call_indirect` type index into the emitted Lir
    // (via `Layout::closure_call_type_index`), which this pass does NOT remap — so for those we keep the
    // IDENTITY layout (byte-identical to before); deduping their type section would desync those baked
    // indices. (Closure-program dedup is a follow-up that must also remap `Lir::CallIndirect` + the DWARF
    // code-range re-derivation.)
    let can_dedup = extra == 0 && layout.lifted.is_empty();
    let mut type_remap: Vec<u32> = Vec::with_capacity(type_seqs.len());
    let mut type_items: Vec<u8> = Vec::new();
    let mut type_count = 0usize;
    {
        let mut seen: std::collections::HashMap<Vec<u8>, u32> = std::collections::HashMap::new();
        for seq in type_seqs {
            if can_dedup {
                if let Some(&ix) = seen.get(&seq) {
                    type_remap.push(ix);
                    continue;
                }
                seen.insert(seq.clone(), type_count as u32);
            }
            type_remap.push(type_count as u32);
            type_items.extend_from_slice(&seq);
            type_count += 1;
        }
    }
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(type_count, &type_items));

    // Import section (id 2) — HOST func imports first (from module `"host"`, indices `0..h`), then one
    // func import per runtime op (from module `"heap"`, `h..import_count`). Omitted entirely when there
    // are no imports of either kind. The order fixes both the host-import index a `CallHostImport`
    // resolves against and the `import_index` map a runtime `CallImport` resolves against.
    // A host string ARGUMENT lowers to `(ptr, len)` read out of linear memory, so a program with a host
    // string arg IMPORTS a memory (from module `"mem"`, name `"mem"`) that the component's shared-memory
    // module provides and the string op's canon-lower reads. The import is a MEMORY desc (`0x02`) with
    // limits `{ min: 1 }`. Placed AFTER the func imports (a memory import does not occupy a func index).
    // The core module imports `mem` when a host op passes a string — either a CONST string (laid in the
    // data segment) OR a RUNTIME string (marshaled into `mem` by the `_mem` copy loop, `host_needs_memory`).
    let needs_memory =
        !layout.host_strings.is_empty() || layout.host_needs_memory || host_result_needs_memory;
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let import_sec = if import_count == 0 && !needs_memory {
        Vec::new()
    } else {
        let mut import_items = Vec::new();
        let mut import_n = 0usize;
        // EXTERN peer ops FIRST (from module `"peer"`, indices `0..e`) — so `CallExternImport(i)=call i`
        // AND, when composed with the runtime, the peer ops precede the runtime ops (matching the
        // `assemble_extern_runtime` envelope). Extern + host never coexist, so `e>0 ⇒ h=0`.
        // NOTE: an import descriptor's TYPE index is `type_remap`-translated (dedup may have collapsed its
        // functype), but its FUNCTION index (its position in the import section, stored in `import_index`
        // for `CallImport`/`CallExternImport`/`CallHostImport` resolution) is UNCHANGED — dedup touches only
        // the type section, never import ordering. In the identity (no-dedup) case `type_remap[x] == x`, so
        // this is byte-identical to before.
        for (i, f) in extern_fns.iter().enumerate() {
            import_items.extend_from_slice(&extern_import_item(&f.op, type_remap[i]));
            import_n += 1;
        }
        for (i, f) in host_fns.iter().enumerate() {
            import_items.extend_from_slice(&host_import_item(&f.op, type_remap[e + i]));
            import_n += 1;
        }
        // RUNTIME ops — from module `"heap"` at FUNC indices `e+h..import_count`, resolved BY NAME via
        // `import_index` (so the shift by `e+h` is automatic wherever a `CallImport` looks up its op). The
        // descriptor's type index is the deduped `type_remap[e+h+j]`; the func index stored in `import_index`
        // stays the positional `e+h+j`.
        for (j, o) in imports.iter().enumerate() {
            let func_idx = (e + h + j) as u32;
            import_items.extend_from_slice(&import_item(o.name, type_remap[e + h + j]));
            import_index.insert(o.name, func_idx);
            import_n += 1;
        }
        // The SHARED `cabi_realloc` FUNC import (module `"mem"`, func index `e+h+k`, its functype at the same
        // type index — laid in the import block above), when a host op returns a compound value. The host-op
        // canon-lower's Realloc option + the guest's `CallImport("cabi_realloc")` (select's host-result lift +
        // the wrapper's spill) all resolve to it via `import_index`; the mem module owns the one bump cursor.
        if import_realloc {
            let func_idx = (e + h + imports.len()) as u32;
            let mut it = uleb_bytes("mem".len() as u64);
            it.extend_from_slice(b"mem");
            it.extend_from_slice(&uleb_bytes("cabi_realloc".len() as u64));
            it.extend_from_slice(b"cabi_realloc");
            it.push(0x00); // import desc: func
            uleb128(type_remap[e + h + imports.len()] as u64, &mut it); // deduped TYPE index
            import_items.extend_from_slice(&it);
            import_index.insert("cabi_realloc", func_idx); // positional FUNC index (unchanged)
            import_n += 1;
        }
        if needs_memory {
            // `mem`.`mem` — a memory import, desc kind `0x02`, limits `{ min: 1 }` (flag 0x00, min 1).
            let mut item = uleb_bytes("mem".len() as u64);
            item.extend_from_slice(b"mem");
            item.extend_from_slice(&uleb_bytes("mem".len() as u64));
            item.extend_from_slice(b"mem");
            item.push(0x02); // import desc: memory
            item.push(0x00); // limits flag: min only
            uleb128(1, &mut item); // min 1 page
            import_items.extend_from_slice(&item);
            import_n += 1;
        }
        section(2, &wasm_vec(import_n, &import_items))
    };

    // Function section: defined func `i` (function index `import_count + i`) uses type index
    // `import_count + i` (the import functypes came first).
    // Each entry is the defined function's TYPE index, `type_remap`-translated (dedup may have collapsed the
    // functype). The FUNCTION indices themselves are unchanged (dedup touches only the type section); only
    // the type reference each function declares is remapped. Identity (`type_remap[x]==x`) when not deduped.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128(type_remap[import_count + i] as u64, &mut func_items);
    }
    // WRAPPER funcs, appended AFTER the n defined+lifted funcs (function indices `import_count + n + w`),
    // each referencing its wrapper functype (`import_count + n + extra + w`).
    for w in 0..n_wrap {
        uleb128(
            type_remap[import_count + n + extra + w] as u64,
            &mut func_items,
        );
    }
    // The DEFINED `cabi_realloc` LAST (function index `import_count + n + n_wrap`), its functype at
    // `import_count + n + extra + n_wrap`. Present only in DEFINE mode (`n_realloc == 1`); in `import_realloc`
    // mode the shared allocator is imported, so no defined func here.
    if n_realloc == 1 {
        uleb128(
            type_remap[import_count + n + extra + n_wrap] as u64,
            &mut func_items,
        );
    }
    // The STATIC-BYTES `start` init func LAST (function index `import_count + n + n_wrap + n_realloc`), its
    // functype at `import_count + n + extra + n_wrap + n_realloc` (the last one laid above). `n_init == 0` →
    // no entry, byte-identical.
    if n_init == 1 {
        uleb128(
            type_remap[import_count + n + extra + n_wrap + n_realloc] as u64,
            &mut func_items,
        );
    }
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + n_wrap + n_realloc + n_init, &func_items),
    );
    // The init function's own absolute wasm function index — named by the START section, run at
    // instantiation to build every static bytes once.
    let init_func_index = (import_count + n + n_wrap + n_realloc) as u64;

    // Export section: export every boundary function under its verbatim name, by its absolute core
    // function index (`layout.abs`, which already includes the import shift).
    let mut export_items = Vec::new();
    let mut export_n = 0usize;
    for e in &layout.exports {
        // A wrapper SHADOWS the def's export (the interface aliases the wrapper); the def stays internal
        // (still a core func the wrapper calls, just unexported). Matched by kebab-normalized name — the
        // wrapper's name is the WIT member (`on-message`, kebab) while the def's export is the guest name
        // (`onMessage`), so a raw compare would miss the shadow and leave a stray def export.
        if wrappers.iter().any(|w| {
            crate::backend::common::export_name::kebab_extern_name(&w.name)
                == crate::backend::common::export_name::kebab_extern_name(&e.name)
        }) {
            continue;
        }
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
        export_n += 1;
    }
    // WRAPPER exports — the member name → the wrapper's func index (`import_count + n + w`).
    for (w, wrap) in wrappers.iter().enumerate() {
        let mut item = uleb_bytes(wrap.name.len() as u64);
        item.extend_from_slice(wrap.name.as_bytes());
        item.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128((import_count + n + w) as u64, &mut item);
        export_items.extend_from_slice(&item);
        export_n += 1;
    }
    // `memory` + `cabi_realloc` exports — the canon lift's Memory/Realloc options alias them off this program
    // instance. Both are present ONLY in DEFINE mode: when memory 0 is DEFINED here (`!needs_memory`) and the
    // realloc is DEFINED (`n_realloc == 1`). In `import_realloc` mode the program IMPORTS both from the shared
    // `"mem"` module and the component aliases them off the mem instance, so the program re-exports neither.
    if wrapper_needs_memory && !needs_memory {
        let mut mem_item = uleb_bytes("memory".len() as u64);
        mem_item.extend_from_slice(b"memory");
        mem_item.push(wasm_abi::EXPORT_KIND_MEMORY);
        uleb128(0, &mut mem_item);
        export_items.extend_from_slice(&mem_item);
        export_n += 1;
    }
    if n_realloc == 1 {
        let mut ra_item = uleb_bytes("cabi_realloc".len() as u64);
        ra_item.extend_from_slice(b"cabi_realloc");
        ra_item.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128((import_count + n + n_wrap) as u64, &mut ra_item);
        export_items.extend_from_slice(&ra_item);
        export_n += 1;
    }
    let export_sec = section(
        wasm_abi::CORE_SEC_EXPORT,
        &wasm_vec(export_n, &export_items),
    );

    // Code section: bodies in emission order.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // WRAPPER bodies — one `(flattened) -> result` func per wrapper: build each def param from the flattened
    // boundary leaves (a `record` param via `emit_cell_rebuild`, a scalar via `local.get`), `call` the def,
    // return its result. The runtime rebuild ops (`arr-alloc`/`arr-set`/`box-*`) resolve through
    // `import_index` (the caller must add them to the used-op set so they are imported).
    if n_wrap > 0 {
        let imp = |name: &str| -> u64 {
            *import_index
                .get(name)
                .unwrap_or_else(|| panic!("wrapper needs runtime op `{name}` in the import set"))
                as u64
        };
        // `cabi_realloc`'s absolute core func index — the wrapper calls it to allocate a spilled-result area.
        // In DEFINE mode it is the last defined func (`import_count + n + n_wrap`); in `import_realloc` mode it
        // is the shared allocator FUNC IMPORT at `e + h + imports.len()` (before the runtime memory import).
        let realloc_abs = if import_realloc {
            (e + h + imports.len()) as u64
        } else {
            (import_count + n + n_wrap) as u64
        };
        for wrap in wrappers {
            // Scratch i32 locals live AFTER the flattened params (indices from `p`): a `list<u8>` leaf uses two
            // (a `buf` handle + a copy counter); the result-lower allocates as many as its recursive plan
            // needs (a returned-value handle, the return pointer, plus per-level handles + list count/base/
            // index for the canonical writer). The body is emitted into `inner` with `next_local` tracking the
            // high-water local index, then the local-decl group prepends the final count — so an arbitrarily
            // deep result writer declares exactly the locals it used. No scratch → zero locals, byte-identical.
            // MEMORY-INDIRECT spill: the core func has ONE `i32` param (the spill pointer at local 0); each
            // logical param is READ from the spill area (`local.get 0` + a naturally-aligned load) rather than
            // a flat param local. So `p` (the first scratch local index) is 1, and `spill_layout` gives each
            // flat leaf's `(offset, load_op, align)`.
            let spilled = wrapper_params_spill(&wrap.param_vts);
            let spill_layout = if spilled {
                spilled_param_layout(&wrap.param_vts)
            } else {
                Vec::new()
            };
            let p = if spilled {
                1u32
            } else {
                wrap.param_vts.len() as u32
            };
            let has_bytes = wrap
                .params
                .iter()
                .flatten()
                .flatten()
                .any(FieldRebuild::has_bytes_leaf)
                // A TOP-LEVEL memory-bearing leaf param copies bytes out of memory too (same two scratch locals).
                || wrap.mem_leaf_params.iter().any(Option::is_some)
                // A TOP-LEVEL sum param whose selected arm carries a `list<u8>` payload (Bytes, or a compound
                // with a bytes leaf) copies bytes out of memory in its arm build — it needs the scratch too.
                // A `list<scalar>` arm (option<list<…>>) likewise reuses the scratch as its vec accumulator +
                // element cursor.
                || wrap.sum_params.iter().flatten().any(|(rebuild, _)| {
                    let arm_needs_scratch = |a: &SumArgArm| {
                        matches!(a.payload, SumArmPayload::Bytes { .. } | SumArmPayload::List(_))
                            || matches!(&a.payload, SumArmPayload::Compound(fs) if fs.iter().any(FieldRebuild::has_bytes_leaf))
                    };
                    arm_needs_scratch(&rebuild.arm_true) || arm_needs_scratch(&rebuild.arm_false)
                });
            let scratch = if has_bytes { Some((p, p + 1)) } else { None };
            let mut next_local = p + if has_bytes { 2 } else { 0 };
            let mut inner = Vec::new();
            let mut leaf = 0u32;
            // Locals holding a lifted memory-leaf handle the def only BORROWED — the wrapper owns them and
            // reclaims (`drop`) them AFTER the def call (a consuming param sets `drop_after` false, so nothing
            // is saved and the def's own consumption reclaims it — never a double-free).
            let mut drop_locals: Vec<u32> = Vec::new();
            for (pi, pp) in wrap.params.iter().enumerate() {
                // A TOP-LEVEL memory-bearing leaf param (String/Bytes) takes precedence: copy its boundary
                // `(ptr, len)` bytes out of linear memory into a value-heap byte-leaf handle and leave that
                // handle on the stack as the def arg DIRECTLY (NOT wrapped in a record cell). A `String` and a
                // `Bytes` param have the SAME lift: a Cadenza `String` value IS a flat UTF-8 byte-leaf, built
                // exactly by `bytes-alloc` + `bytes-set` (the `Core::ConstStr` emit + the `str-new` rep), so the
                // copied buffer is already a canonical String handle — no `str-from-bytes` decode (a WIT
                // `string` param is guaranteed valid UTF-8, and that op would re-wrap it). Only the boundary
                // TYPE differs (`string` vs `list<u8>`), fixed at the routing site via `ty_natural_wit`.
                if let Some((kind, drop_after)) = wrap.mem_leaf_params.get(pi).copied().flatten() {
                    let (buf, ctr) =
                        scratch.expect("a memory-bearing leaf param needs the scratch locals");
                    match kind {
                        // String/Bytes: a raw UTF-8/byte copy-in (the copied byte-leaf IS the value).
                        MemLeafKind::Str | MemLeafKind::Bytes => {
                            emit_bytes_leaf_copy_in(
                                leaf,
                                false, // a top-level string/bytes ptr is i32 (no variant-join widening)
                                buf,
                                ctr,
                                import_realloc,
                                &imp,
                                &mut inner,
                            ); // → [buf]
                        }
                        // list<scalar> (or nested list<list<…>>): build a value-heap vec by reading + boxing
                        // each element per its width, recursing on `nest_lists` for sub-lists.
                        MemLeafKind::List(elem) => {
                            emit_list_leaf_lift(
                                &elem,
                                leaf,
                                buf,
                                ctr,
                                &mut next_local,
                                &imp,
                                &mut inner,
                            ); // → [vec]
                        }
                        // BigInt/Rational/Symbol: copy the `(ptr, len)` value-form bytes out of linear memory,
                        // bake the type's shape descriptor, and `value-decode(bytes, desc)` → the value-heap
                        // handle (eb1/er1/ey1). Needs two fresh locals (desc handle, decoded-result stash).
                        MemLeafKind::ValueForm(idx) => {
                            let desc = &wrap.value_form_descs[idx as usize];
                            let desc_local = next_local;
                            let res_local = next_local + 1;
                            next_local += 2;
                            emit_value_form_lift(
                                desc,
                                leaf,
                                buf,
                                ctr,
                                desc_local,
                                res_local,
                                import_realloc,
                                &imp,
                                &mut inner,
                            ); // → [handle]
                        }
                    }
                    leaf += 2; // the string/list flattened to (ptr, len)
                    if drop_after {
                        // The def borrows this param; save the handle (it stays on the stack as the def arg)
                        // so the wrapper can reclaim it after the call.
                        let dl = next_local;
                        next_local += 1;
                        inner.push(op::LOCAL_TEE);
                        uleb128(dl as u64, &mut inner); // [buf] stays; buf also saved in `dl`
                        drop_locals.push(dl);
                    }
                    continue;
                }
                // A TOP-LEVEL option/result param: branch on the boundary disc and build the guest sum cell,
                // leaving its handle DIRECTLY as the def arg (like the mem-leaf path — not stored in a record
                // cell). `emit_sum_field` reads the disc at `leaf` and advances it past `(disc, payload…)`.
                // `drop_after` = the def only BORROWS the cell (matches it), so the wrapper (its owner) drops it
                // after the call — save the handle in a local first (it stays on the stack as the def arg).
                if let Some((rebuild, drop_after)) =
                    wrap.sum_params.get(pi).and_then(|s| s.as_ref())
                {
                    emit_sum_field(
                        rebuild,
                        &mut leaf,
                        import_realloc,
                        &imp,
                        scratch,
                        &mut inner,
                    ); // → [sum-handle]
                    if *drop_after {
                        let dl = next_local;
                        next_local += 1;
                        inner.push(op::LOCAL_TEE);
                        uleb128(dl as u64, &mut inner); // handle stays on the stack; also saved in `dl`
                        drop_locals.push(dl);
                    }
                    continue;
                }
                // A TOP-LEVEL payloadless-ENUM param whose WIT case order ≠ the guest decl order: the boundary
                // `i32` disc (at `leaf`) is REMAPPED to the guest disc by name (`inv_perm[wit] = guest`) and
                // left on the stack DIRECTLY as the def arg — the PARAM twin of `ResultLower::EnumRemap`. An
                // order-MATCHING enum param carries `None` here (the disc IS the guest disc, handled by the
                // `params` `None` passthrough below). No heap handle, so nothing to reclaim.
                if let Some(inv_perm) = wrap.enum_disc_params.get(pi).and_then(|s| s.as_ref()) {
                    emit_enum_disc_remap(inv_perm, leaf, &mut inner); // → [guest disc]
                    leaf += 1;
                    continue;
                }
                match pp {
                    None => {
                        if spilled {
                            // Read the scalar from the spill area: `local.get 0` (the spill pointer) then a
                            // naturally-aligned load at this leaf's canonical offset.
                            let (offset, load_op, align) = spill_layout[leaf as usize];
                            inner.push(op::LOCAL_GET);
                            uleb128(0, &mut inner);
                            inner.push(load_op);
                            uleb128(align as u64, &mut inner);
                            uleb128(offset as u64, &mut inner);
                        } else {
                            inner.push(op::LOCAL_GET);
                            uleb128(leaf as u64, &mut inner);
                        }
                        leaf += 1;
                    }
                    Some(fields) => {
                        let slots = wrap.param_slots.get(pi).and_then(|s| s.as_deref());
                        emit_cell_rebuild(
                            fields,
                            &mut leaf,
                            import_realloc,
                            &imp,
                            scratch,
                            slots,
                            &mut inner,
                        ); // → [cell]
                        // #9014 envelope shell-drop: the wrapper OWNS this freshly-rebuilt cell and passes it
                        // BORROWED to the def. When `record_param_drop_after` (the dup-aware
                        // `record_cell_param_droppable` gate) says the cell is a dead owned temporary after the
                        // call — every forwarded field was dup'd, so it survives the deep-drop cascade — save
                        // it in a drop-local (it stays on the stack as the def arg) and deep-drop it in the
                        // post-call reclaim loop below, exactly like the mem-leaf/sum `drop_after` path.
                        // 28-wit:310 SHAPE-9: the cell is NOT blanket-droppable because compound field(s) move
                        // out verbatim, so reclaim the shell WITHOUT the escaped-field UAF — save the cell, then
                        // for each escaped-occurrence path project the field off the cell (`arr-get` BORROWS)
                        // and `dup` it (→ rc≥2) NOW (before the call); the post-call reclaim loop deep-drops the
                        // shell, whose cascade returns each dup'd field to rc≥1 so the result's handle survives.
                        // dup-BEFORE-deep-drop ordering is the whole safety; multiplicity is authoritative (one
                        // `dup` per escaped-occurrence entry — never dedup). Mutually exclusive with
                        // `record_param_drop_after` (Some only when the shell is NOT blanket-droppable).
                        if let Some(paths) = wrap
                            .record_param_escaped_fields
                            .get(pi)
                            .and_then(|f| f.as_ref())
                        {
                            let dl = next_local;
                            next_local += 1;
                            inner.push(op::LOCAL_TEE);
                            uleb128(dl as u64, &mut inner); // [cell] stays as the def arg; also saved in `dl`
                            for path in paths {
                                inner.push(op::LOCAL_GET);
                                uleb128(dl as u64, &mut inner); // [cell, cell-copy]
                                for &idx in path {
                                    // project field `idx` off the (nested) cell — `arr-get` borrows + consumes
                                    // the cell-copy, leaving the field box (a heap-cell handle for a compound).
                                    inner.push(op::I32_CONST);
                                    crate::backend::wasm::encode::sleb128(idx as i64, &mut inner); // [cell-copy, idx]
                                    inner.push(op::CALL);
                                    uleb128(imp("arr-get"), &mut inner); // [field-box] (borrows)
                                }
                                inner.push(op::CALL);
                                uleb128(imp("dup"), &mut inner); // rc+1 on the escaped leaf; consumes it → [cell]
                            }
                            drop_locals.push(dl); // deep-drop the shell after the call (cascade → field rc≥1)
                        } else if wrap
                            .record_param_drop_after
                            .get(pi)
                            .copied()
                            .unwrap_or(false)
                        {
                            let dl = next_local;
                            next_local += 1;
                            inner.push(op::LOCAL_TEE);
                            uleb128(dl as u64, &mut inner); // [cell] stays; also saved in `dl`
                            drop_locals.push(dl);
                        }
                    }
                }
            }
            inner.push(op::CALL);
            uleb128(wrap.def_abs as u64, &mut inner); // → [def result]
            // Reclaim each lifted memory-leaf param the def only BORROWED (the wrapper is its sole owner).
            // `drop` takes the handle and leaves the def result beneath untouched (stack-balanced).
            for dl in &drop_locals {
                inner.push(op::LOCAL_GET);
                uleb128(*dl as u64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            // Result: a scalar/unit passes straight through; a compound spills to a memory return area.
            if let ResultLower::SpillRecord { size, align, write } = &wrap.result {
                let rec = next_local;
                let retptr = next_local + 1;
                next_local += 2;
                emit_result_spill(
                    rec,
                    retptr,
                    &mut next_local,
                    realloc_abs,
                    *size,
                    *align,
                    write,
                    // seq-916 loop-ii: bulk nested-Bytes LOWER via `bytes-read` when the shared allocator +
                    // bytes-read canon-lower are present (the `import_realloc`/`_mem` mode) — same gate the
                    // `CopyBytes` bare-result bulk uses below.
                    import_realloc,
                    &imp,
                    &mut inner,
                );
            }
            // A `list<u8>`/Bytes result member: copy the runtime bytes to a `cabi_realloc`'d buffer and
            // write the `(ptr,len)` retarea (the multi-member-interface twin of the single-export provider).
            if matches!(wrap.result, ResultLower::CopyBytes) {
                let rec = next_local;
                let retptr = next_local + 1;
                next_local += 2;
                emit_result_copy_bytes(
                    rec,
                    retptr,
                    &mut next_local,
                    realloc_abs,
                    import_realloc,
                    &imp,
                    &mut inner,
                );
            }
            // A reordered payloadless-enum result: the def left its GUEST disc on the stack; remap it to the
            // WIT disc by name (`perm[guest] = wit`) via a nested-if chain, leaving the WIT disc as the return
            // value. `perm` is only present for a genuine reorder (identity is `Passthrough`), so the chain has
            // ≥2 arms. Pure wasm (compare/select on i32) — no runtime op, no memory.
            if let ResultLower::EnumRemap { perm } = &wrap.result {
                let d = next_local;
                next_local += 1;
                inner.push(op::LOCAL_SET);
                uleb128(d as u64, &mut inner); // d = guest disc (consumes the def result)
                emit_enum_disc_remap(perm, d, &mut inner); // → [wit disc]
            }
            // A flat single-scalar-field record result: the def left the record HANDLE on the stack; read its
            // one field (`arr-get(handle, field_cell)` → unbox `read`, narrowing a ≤32-bit value) and return
            // that scalar as the flattened result — [handle] → [scalar]. No memory (returned in a register).
            if let ResultLower::FlatScalarField {
                field_cell,
                read,
                wrap_i64,
            } = &wrap.result
            {
                // Save the def's record result HANDLE before the borrowing field read, so the wrapper can
                // reclaim it after (the def returns an OWNED record — callee-owns-result — and `arr-get` only
                // BORROWS it). SHAPE 76 reclaim, the flat-1-value twin of the SpillRecord path's deep-drop.
                let rec = next_local;
                next_local += 1;
                inner.push(op::LOCAL_TEE);
                uleb128(rec as u64, &mut inner); // [handle] (also stashed in `rec`)
                inner.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(*field_cell as i64, &mut inner); // [handle, field_cell]
                inner.push(op::CALL);
                uleb128(imp("arr-get"), &mut inner); // [field-box] (borrows handle)
                inner.push(op::CALL);
                uleb128(imp(read), &mut inner); // [scalar]
                if *wrap_i64 {
                    inner.push(op::I32_WRAP_I64); // narrow the i64 heap cell to its ≤32-bit result slot
                }
                // Reclaim the owned record result handle — deep-drop (its sole scalar child was copied out by
                // the unbox above, so the cascade is balanced). Stack-neutral: the scalar result stays below
                // this `local.get; drop`.
                inner.push(op::LOCAL_GET);
                uleb128(rec as u64, &mut inner); // [scalar, handle]
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner); // [scalar]
            }
            inner.push(op::END);
            let n_locals = next_local - p;
            let mut body = Vec::new();
            if n_locals > 0 {
                body.extend_from_slice(&uleb_bytes(1)); // one local-decl group
                body.extend_from_slice(&uleb_bytes(n_locals as u64));
                body.push(wasm_abi::CORE_I32);
            } else {
                body.extend_from_slice(&uleb_bytes(0));
            }
            body.extend_from_slice(&inner);
            let mut entry = uleb_bytes(body.len() as u64);
            entry.extend_from_slice(&body);
            code_items.extend_from_slice(&entry);
        }
    }
    // The DEFINED `cabi_realloc` body LAST — a bump allocator over global 0: `p = (g + align - 1) & -align`
    // (align is a power of 2, so `-align == ~(align-1)`), advance `g = p + new_size`, return `p`. One extra
    // i32 local (index 4, after the 4 params) holds `p`. Present only in DEFINE mode (`n_realloc == 1`); in
    // `import_realloc` mode the shared allocator is imported (no defined body, no global cursor).
    if n_realloc == 1 {
        let mut body = Vec::new();
        body.extend_from_slice(&uleb_bytes(1)); // one local-decl group
        body.extend_from_slice(&uleb_bytes(1)); // of one
        body.push(wasm_abi::CORE_I32); // i32 (local 4 = aligned p)
        // p = (cursor + align - 1) & -align  (cursor global sits after the static-bytes globals)
        body.push(op::GLOBAL_GET);
        uleb128(realloc_cursor_global, &mut body);
        body.push(op::LOCAL_GET);
        uleb128(2, &mut body); // align
        body.push(op::I32_ADD);
        body.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(1, &mut body);
        body.push(op::I32_SUB);
        body.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut body);
        body.push(op::LOCAL_GET);
        uleb128(2, &mut body); // align
        body.push(op::I32_SUB); // -align
        body.push(op::I32_AND);
        body.push(op::LOCAL_TEE);
        uleb128(4, &mut body); // p → local 4, left on stack
        // cursor = p + new_size
        body.push(op::LOCAL_GET);
        uleb128(3, &mut body); // new_size
        body.push(op::I32_ADD);
        body.push(op::GLOBAL_SET);
        uleb128(realloc_cursor_global, &mut body);
        // GROW linear memory to cover [p, p + new_size) BEFORE returning: memory is declared `(memory 1)` = one
        // 64-KiB page, but the host writes `new_size` bytes at the returned `p` after cabi_realloc returns (the
        // canonical-ABI lower of an incoming `list<u8>` param), and the guest itself copies a >1-page result
        // buffer out to `p` (the `list<u8>`/Bytes RESULT copy-out) — either write past 65536-`p` faults OOB at
        // the wasm-page boundary. This is the exact twin of the copy-out `emit_grow_to_cover_out` guard and of
        // `emit_bump_realloc_body`'s grow: a ~64-KiB binary-AST document round-tripped through this two-export
        // wrapper (`encode-quoted() -> list<u8>` and `decode-check(list<u8>)`) trapped `@0x10000 in a
        // size-0x10000 memory` (corpus 05 rcw4). needed_pages = ceil((p + new_size) / 65536); grow the shortfall
        // over memory.size (memory.grow never traps, a no-op when memory already suffices).
        {
            const MEMORY_SIZE: u8 = 0x3f;
            const MEMORY_GROW: u8 = 0x40;
            // needed_pages = (p + new_size + 65535) >> 16
            let needed = |out: &mut Vec<u8>| {
                out.push(op::LOCAL_GET);
                uleb128(4, out); // p
                out.push(op::LOCAL_GET);
                uleb128(3, out); // new_size
                out.push(op::I32_ADD);
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(65535, out);
                out.push(op::I32_ADD);
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(16, out);
                out.push(op::I32_SHR_U);
            };
            // (needed_pages - memory.size) > 0 ?
            needed(&mut body);
            body.push(MEMORY_SIZE);
            body.push(0x00); // mem index 0
            body.push(op::I32_SUB);
            body.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(0, &mut body);
            body.push(op::I32_GT_S);
            body.push(op::IF);
            body.push(wasm_abi::BLOCK_EMPTY);
            {
                // memory.grow(needed_pages - memory.size); drop
                needed(&mut body);
                body.push(MEMORY_SIZE);
                body.push(0x00);
                body.push(op::I32_SUB);
                body.push(MEMORY_GROW);
                body.push(0x00); // mem index 0
                body.push(op::DROP);
            }
            body.push(op::END);
        }
        // return p
        body.push(op::LOCAL_GET);
        uleb128(4, &mut body);
        body.push(op::END);
        let mut entry = uleb_bytes(body.len() as u64);
        entry.extend_from_slice(&body);
        code_items.extend_from_slice(&entry);
    }
    // The STATIC-BYTES `start` init body LAST — for each distinct constant `Bytes`, build it ONCE
    // (`bytes-alloc(len)` then a `bytes-set` per byte — the same sequence the inline `Core::BytesOf` emit
    // used, so the once-built value is byte-identical), mark it IMMORTAL, and store the handle in its
    // global. `mark-immortal(handle)->handle` (heap op #3847) sets `rc == IMMORTAL`: the node is
    // census-EXCLUDED (so a build-once static is not a false leak) and `op_dup`/`op_drop` are NO-OPs on it
    // (a consumer over-drop can never free the global's ref → UAF-proof), and `node_rc == IMMORTAL` forces
    // FBIP to path-copy rather than mutate the shared static. Built as an ordinary `SelectedFunc`
    // (`() -> ()`, no locals — the buffer threads on the stack via `bytes-set`'s + `mark-immortal`'s FBIP
    // return) so `code_entry` resolves the ops through the same `import_index`. Present only when there are
    // static bytes.
    if n_init == 1 {
        let mut code: Vec<crate::backend::wasm::lir::Lir> = Vec::new();
        for (g, bytes) in layout.static_bytes.iter().enumerate() {
            code.push(Lir::ConstI32(bytes.len() as i32)); // [len]
            code.push(Lir::CallImport("bytes-alloc")); // → [buf]
            for (i, &b) in bytes.iter().enumerate() {
                code.push(Lir::ConstI32(i as i32)); // [buf, index]
                code.push(Lir::ConstI32(b as i32)); // [buf, index, byte]
                code.push(Lir::CallImport("bytes-set")); // → [buf] (bytes-set returns the buffer)
            }
            code.push(Lir::CallImport("mark-immortal")); // → [buf] (rc=IMMORTAL: census-excluded, dup/drop no-op)
            code.push(Lir::GlobalSet(g as u32)); // store the once-built immortal handle → []
        }
        // §2d increment 6: append the precomputed STATIC-COMPOUND init (`build_static_compound_init`), which
        // for each markable constant Tuple/Record builds its immortal tree + `global.set`s it to
        // `n_static + k` (its global follows the byte globals). Built with `Db` in the backend + carried on
        // the layout (this fn has no `Db`). Empty when there are no static compounds.
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
        &wasm_vec(n + n_wrap + n_realloc + n_init, &code_items),
    );

    // TABLE + ELEMENT sections — present ONLY when the program has lambda-lifted closures (a runtime
    // closure applies through `call_indirect` over the one funcref table). The table holds one funcref
    // per lifted lambda; the active element segment (at offset 0) fills slot `k` with lifted lambda
    // `k`'s absolute wasm function index, so a `Core::Closure { code: k }` stored slot selects its code.
    // Empty for a program with no closure → NO table/element section, byte-identical to before.
    let n_lifted = layout.lifted.len();
    let (table_sec, elem_sec) = if n_lifted == 0 {
        (Vec::new(), Vec::new())
    } else {
        // Table section: one funcref table (element type 0x70), limits { min = max = n_lifted }.
        let mut table_entry = vec![0x70u8]; // funcref element type
        table_entry.push(0x01); // limits flag: has-max
        uleb128(n_lifted as u64, &mut table_entry); // min
        uleb128(n_lifted as u64, &mut table_entry); // max
        let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
        // Element section: one active segment for table 0 at offset 0, listing each lifted function's
        // absolute wasm index in table-slot order (`i32.const 0` offset expr, then the func-index vector).
        let mut seg = Vec::new();
        seg.push(0x00); // segment flags: active, table 0, funcref, func-index list
        seg.push(op::I32_CONST); // offset init expr: i32.const 0
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        let elem_sec = section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg));
        (table_sec, elem_sec)
    };

    // DATA section — one active segment per host-arg string, at its assigned byte offset in the imported
    // memory (`host_strings`). The `Core::HostCall` string-arg emit pushes `(offset, len)` pointing here,
    // and the string op's canon-lower reads the UTF-8 bytes out. Empty (no data section) for a program
    // with no host string arg — byte-identical to before.
    let data_sec = if layout.host_strings.is_empty() {
        Vec::new()
    } else {
        let mut items = Vec::new();
        for (s, offset) in &layout.host_strings {
            let mut seg = vec![0x00]; // active, memory 0
            seg.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(*offset as i64, &mut seg);
            seg.push(op::END);
            seg.extend_from_slice(&uleb_bytes(s.len() as u64));
            seg.extend_from_slice(s.as_bytes());
            items.extend_from_slice(&seg);
        }
        section(
            wasm_abi::CORE_SEC_DATA,
            &wasm_vec(layout.host_strings.len(), &items),
        )
    };

    // MEMORY (sec 5) — memory 0 (min 1 page) the canon lift lowers incoming lists into. Present only when a
    // wrapper needs memory AND it is not IMPORTED as the shared `"mem"` (a host op needing memory imports it
    // instead). STATIC BYTES need NO linear memory — they live on the value-heap rope (`bytes-alloc`), only
    // the GLOBAL section below. Empty otherwise → byte-identical.
    let mem_sec = if wrapper_needs_memory && !needs_memory {
        section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]))
    } else {
        Vec::new()
    };
    // GLOBAL (sec 6) — one mutable i32 per STATIC BYTES payload FIRST (indices `0..n_static`, init 0), then
    // one per STATIC COMPOUND (indices `n_static..n_static+n_compounds`, init 0) — the `start` init overwrites
    // each with its once-built immortal handle — then the `cabi_realloc` bump cursor LAST (index
    // `n_static+n_compounds`) when a DEFINED allocator is present (`n_realloc == 1`, init = the const-string
    // data end rounded up, floored at 16 so a returned pointer is never 0 — see below). `n_realloc == 1`
    // already implies `wrapper_needs_memory`. Empty (byte-identical)
    // when there are no static globals nor a defined cursor.
    let global_sec = if n_static > 0 || n_compounds > 0 || n_realloc == 1 {
        let global_entry = |init: i64| -> Vec<u8> {
            let mut g = vec![wasm_abi::CORE_I32, 0x01]; // i32, mutable
            g.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(init, &mut g);
            g.push(op::END);
            g
        };
        let mut items = Vec::new();
        for _ in 0..(n_static + n_compounds) {
            items.extend_from_slice(&global_entry(0));
        }
        if n_realloc == 1 {
            // Seat the bump cursor ABOVE the const-string data region `[0, const_end)` so a realloc'd host
            // RESULT never clobbers a const-string arg before the host lifts it — the D5 string-arg
            // miscompile, here on the DEFINE-mode (inline-allocator) path; the twin of the
            // `envelope::shared_mem_realloc_module` fix on the `import_realloc` path. `align8(max(16,
            // const_end))`: 16 keeps a returned pointer non-zero AND is byte-identical when the const-string
            // data ends at/below 16.
            let const_end = layout
                .host_strings
                .iter()
                .map(|(s, off)| off + s.len() as u32)
                .max()
                .unwrap_or(0);
            items.extend_from_slice(&global_entry(i64::from((const_end.max(16) + 7) & !7)));
        }
        section(
            wasm_abi::CORE_SEC_GLOBAL,
            &wasm_vec(n_static + n_compounds + n_realloc, &items),
        )
    } else {
        Vec::new()
    };
    // START (sec 8) — names the STATIC-BYTES `start` init function (run at instantiation to build every
    // static bytes once into its global). Present only when there are static bytes; laid between EXPORT (7)
    // and ELEMENT (9) per the core-module section order.
    let start_sec = if n_init == 1 {
        section(wasm_abi::CORE_SEC_START, &uleb_bytes(init_func_index))
    } else {
        Vec::new()
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&global_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&start_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    // The `name` custom section (Mode E, D0) is now appended by `wasm::append_debug_sections` AFTER this
    // returns — uniformly for BOTH the ordinary path and the resource-escape path — so `core_module`
    // stays purely the executed sections (byte-identical to today for every caller).
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
    make_param_vts: &[ValType],
    make_core_slots: &[MakeCoreSlot],
    lifted_table: &[u32],
) -> Result<Vec<u8>, String> {
    runtime_resource_core_module_form(
        funcs,
        imports,
        export_abs,
        EscapeForm::Flat(template),
        make_param_vts,
        make_core_slots,
        lifted_table,
    )
}

/// The value-form the escape `t-encode` walker renders — a single flat template (a tuple/record, ONE
/// shape) or a per-variant sum template (a disc-switch over N shapes). Both share the identical
/// type/import/func/memory/export envelope; only the data layout + the `t-encode` body differ.
pub enum EscapeForm<'a> {
    Flat(&'a crate::lower::ValueFormTemplate),
    /// A FLAT template whose export result is a SCALAR-ERASED value (a runtime `Qty` — it erases to its bare
    /// inner scalar, not a heap handle). The `make` body must BOX that scalar (`box_op`, e.g. `box-int`)
    /// after `call <export>` so `resource-new` receives a real i32 root handle; the template's single leaf
    /// hole then reads `get-int` off it at an empty path, exactly like a one-element runtime tuple. `box_op`
    /// takes the raw scalar and returns the i32 handle. `extend` = `Some(signed)` when the inner is a NARROW
    /// int (an i32 core param) that must be i32→i64 widened BEFORE `box-int` (which takes i64) — signed for
    /// a signed narrow int, unsigned otherwise; `None` for a full-width Int64/UInt64 (already i64). Units are
    /// compile-time-only: the label is baked into `tpl`; only the scalar crosses.
    FlatScalar {
        tpl: &'a crate::lower::ValueFormTemplate,
        box_op: &'static str,
        extend: Option<bool>,
    },
    Sum(&'a crate::lower::SumFormTemplate),
    /// A RUNTIME `Bytes` result — a VARIABLE-length value form the walker builds by LOOPING (the first
    /// non-unrolled `encode()`): write the static prefix, the runtime `bytes-len` as a LEB, a
    /// `bytes-get` copy loop, then the static suffix. `DESIGN-runtime-bytes-escape-walker.md`.
    RuntimeBytes(&'a crate::lower::RuntimeBytesForm),
    /// A RUNTIME RECURSIVE sum (a linked list, a tree) — the walker bakes the shape DESCRIPTOR
    /// (compiler-built bytes) as a heap `Bytes`, calls the runtime `value-encode(rep, desc)` to render
    /// the value-form document (the runtime owns the recursion + document assembly), and copies the
    /// result Bytes out. `DESIGN-recursive-sum-escape-walker.md` (approach C).
    RecursiveSum(&'a [u8]),
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
    make_param_vts: &[ValType],
    make_core_slots: &[MakeCoreSlot],
    lifted_table: &[u32],
) -> Result<Vec<u8>, String> {
    runtime_resource_core_module_form_ex(
        funcs,
        imports,
        export_abs,
        form,
        &[],
        make_param_vts,
        make_core_slots,
        lifted_table,
    )
}

/// A value-resource METHOD the core module emits beyond make/t-encode/cabi_realloc (VM-1..VM-3). Each is a
/// BORROW method — its i32 param IS the heap rep (wasmtime's `lift_borrow` passes the rep, not a table
/// index), so the body reads it directly (no `resource.rep`, no drop — repeatable). Emitted AFTER
/// cabi_realloc, in list order, so the core-func indices match the envelope's alias order (method i at
/// core func k+6+i).
#[derive(Clone, Copy, PartialEq)]
pub enum CoreMethod {
    /// `t-len : (rep i32) -> i32` = `bytes-len(rep)` — a scalar length query (VM-1).
    Len,
    /// `t-is-empty : (rep i32) -> i32` = `bytes-len(rep) == 0` (VM-3c) — a bool query (crosses as `bool`).
    /// `bytes-len(rep); i32.eqz`. Proves a SECOND scalar method (bool result) coexists with `len`.
    IsEmpty,
    /// `t-to-bytes : (rep i32) -> i32` = the RAW payload copied into the (ptr,len) retarea and `0` returned
    /// (VM-3). A `bytes-get` copy loop with NO value-form framing (unlike `t-encode`). Exports memory +
    /// cabi_realloc (already present for encode), so the canonical ABI reads the `list<u8>` from `(OUT, n)`.
    ToBytes,
}

/// [`runtime_resource_core_module_form`] plus a set of extra value-resource `methods` the core also
/// exports (VM-1..VM-3): `Len` (a scalar `bytes-len`) and/or `ToBytes` (the raw payload as `list<u8>`).
/// Each is a borrow method (its i32 param IS the rep — no `resource.rep`, no drop, repeatable), emitted
/// AFTER make/t-encode/cabi_realloc in list order so its core func index (k+6+i) matches the envelope's
/// alias order. Empty `methods` = byte-identical to `runtime_resource_core_module_form`. Only the
/// RuntimeBytes form uses these today (`bytes-len`/`bytes-get`; a List would use `vec-*`, later).
#[allow(clippy::too_many_arguments)]
pub fn runtime_resource_core_module_form_ex(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    form: EscapeForm,
    methods: &[CoreMethod],
    make_param_vts: &[ValType],
    make_core_slots: &[MakeCoreSlot],
    lifted_table: &[u32],
) -> Result<Vec<u8>, String> {
    runtime_resource_core_module_form_ex2(
        funcs,
        imports,
        &[],
        false, // no leading ops → module selector irrelevant (peer default)
        export_abs,
        form,
        methods,
        make_param_vts,
        make_core_slots,
        lifted_table,
        0,   // no static compounds on this wrapper path (byte-identical to before)
        &[], // no static-compound init
    )
}

/// [`runtime_resource_core_module_form_ex`] plus a leading CROSS-COMPONENT (peer) extern-import set
/// (`extern_fns`, from module `"peer"`) — the resource-escape × peer-extern FUSION. A peer-bound op reached
/// in a body whose ENTRYPOINT RESULT escapes as a runtime resource needs its peer import carried into the
/// resource core module, exactly as `core_module_impl` does for the ordinary path. Peer ops occupy core-func
/// indices `0..e` (so `Lir::CallExternImport(i)` = `call i` resolves), then the runtime ops shift to
/// `e..e+k`, resource-new/rep to `e+k`/`e+k+1`, defined funcs from `e+k+2`. `extern_fns` EMPTY = byte-
/// identical to `runtime_resource_core_module_form_ex` (every `e`-term drops to 0).
///
/// LEADING-MODULE SELECTOR: `leading_is_host` chooses the module the leading `e` ops import from — `"peer"`
/// (the cross-component extern shape, default `false`) or `"host"` (the host-effect shape, the
/// host-resource-escape fusion). The ONLY byte-difference is the import item's module STRING at each leading
/// op; the core-func INDEX layout (leading ops `0..e`, runtime `e..e+k`, resource intrinsics, defined funcs)
/// is IDENTICAL because a func import's index does not depend on its module name. So one builder serves both
/// the peer-resource and host-resource fusions (the escape-form + method machinery stays shared).
#[allow(clippy::too_many_arguments)]
pub fn runtime_resource_core_module_form_ex2(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    extern_fns: &[crate::backend::wasm::host::ExternImport],
    leading_is_host: bool,
    export_abs: u32,
    form: EscapeForm,
    methods: &[CoreMethod],
    make_param_vts: &[ValType],
    make_core_slots: &[MakeCoreSlot],
    // The lambda-lifted closures' ABSOLUTE core-func indices, in table-slot order (empty for a closure-free
    // program). When non-empty, this module dispatches a first-class closure via `call_indirect (table 0)`,
    // so lay a funcref table (min=max=len) + an active element segment at offset 0 filling slot `k` with
    // `lifted_table[k]`. WITHOUT this the `call_indirect` referenced a non-existent table 0 → invalid wasm.
    // The caller appends the lifted bodies to `funcs` (so these indices exist) via `append_lifted_bodies`.
    lifted_table: &[u32],
    // BUILD-ONCE STATIC COMPOUNDS in the resource-escape assembler (WIT static encoding, 2026-08-27): the
    // markable constant Tuple/Record/List/Map/Set roots the escaping body USES, hoisted to module GLOBALS so
    // each is built ONCE at instantiation instead of per-`make`. `n_compounds` static-compound globals occupy
    // indices `0..n_compounds` (this builder has no other globals — its `cabi_realloc` is a stub with no
    // cursor), and `static_compound_init` is the flat `Lir` (`select::build_static_compound_init`) that builds
    // each immortal + `global.set`s it, run by a synthesized START init function appended LAST (so it shifts no
    // existing func index). `n_compounds == 0` → no GLOBAL/START/init additions, byte-identical to before. The
    // caller (`emit_runtime_resource`) threads the SAME `static_compounds` onto the layout it selects with, so
    // the body's `Core::Tuple`/… arms emit `global.get idx` (`try_emit_static_compound`) matching these globals.
    n_compounds: usize,
    static_compound_init: &[crate::backend::wasm::lir::Lir],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let e = extern_fns.len();
    let k = imports.len();
    let n = funcs.len();
    // The START init func exists iff there are static compounds to build once.
    let n_init = (n_compounds > 0) as usize;

    // ── Type section ──
    // EXTERN peer functypes 0..e FIRST (matching the import order → `CallExternImport(i)=call i`), then the
    // runtime-op functypes `e..e+k`, then resource-new/resource-rep (both `(i32)->i32`), then one functype
    // per defined body, then the three synthesized-func types (make `()->i32`, encode `(i32)->i32`,
    // cabi_realloc `(i32×4)->i32`). (`e = 0` for the ordinary non-peer resource escape.)
    let mut type_items = Vec::new();
    for f in extern_fns {
        type_items.extend_from_slice(&extern_import_functype(f));
    }
    for o in imports {
        type_items.extend_from_slice(&import_functype(o));
    }
    let i32_to_i32 = {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t
    };
    type_items.extend_from_slice(&i32_to_i32); // resource-new type (index e+k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep type (index e+k+1)
    let defined_type_base = e + k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    // make `(make-params…)->i32` — a NULLARY export gives `make()`; a PARAMETERIZED export forwards its
    // scalar params so the host computes a distinct value per input (the value analogue of the closure
    // resource's `make(k)`, C-HOST-2).
    let make_type_idx = defined_type_base + n;
    {
        let params: Vec<u8> = make_param_vts.iter().map(|v| v.byte()).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
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
    // Each extra method (`t-len`/`t-to-bytes`) is `(i32)->i32` — one functype per method, after the three
    // synthesized types. `method_type_idx[i]` is method i's core type index.
    let mut method_type_idx = Vec::new();
    for _ in methods {
        method_type_idx.push((realloc_type_idx + 1 + method_type_idx.len()) as u32);
        type_items.extend_from_slice(&i32_to_i32);
    }
    // The STATIC-COMPOUND `start` init functype `() -> ()`, LAST — the init takes no params and returns
    // nothing (it builds each static compound and stores its handle in the global). Present iff `n_init == 1`.
    let init_type_idx = defined_type_base + n + 3 + methods.len();
    if n_init == 1 {
        let mut ft = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        ft.extend_from_slice(&wasm_vec(0, &[]));
        ft.extend_from_slice(&wasm_vec(0, &[]));
        type_items.extend_from_slice(&ft);
    }
    let total_types = defined_type_base + n + 3 + methods.len() + n_init;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── e PEER ops (from "peer", indices `0..e`, so `CallExternImport(i)=call i`), then
    // k runtime ops (from "heap", `e..e+k`) + resource-new + resource-rep (`e+k`, `e+k+1`). Builds the
    // `import_index` a runtime `CallImport` resolves against (op name → its `e+i` index — the shift by `e`
    // is automatic wherever a `CallImport` looks up its op by name). (`e = 0` for the ordinary escape.)
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut import_items = Vec::new();
    for (i, f) in extern_fns.iter().enumerate() {
        // The leading ops import from `"host"` (host-effect fusion) or `"peer"` (cross-component extern) —
        // same func-index `i`, module string is the only difference (see `leading_is_host`).
        import_items.extend_from_slice(&if leading_is_host {
            host_import_item(&f.op, i as u32)
        } else {
            extern_import_item(&f.op, i as u32)
        });
    }
    for (j, o) in imports.iter().enumerate() {
        let ti = (e + j) as u32;
        import_items.extend_from_slice(&import_item(o.name, ti));
        import_index.insert(o.name, ti);
    }
    import_items.extend_from_slice(&import_item("resource-new", (e + k) as u32));
    import_items.extend_from_slice(&import_item("resource-rep", (e + k + 1) as u32));
    let import_sec = section(2, &wasm_vec(e + k + 2, &import_items));
    let f_rnew = (e + k) as u32;
    let f_rrep = (e + k + 1) as u32;

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
    for &ti in &method_type_idx {
        uleb128(ti as u64, &mut func_items);
    }
    let n_synth = 3 + methods.len();
    // The STATIC-COMPOUND `start` init func LAST (after the methods), using `init_type_idx`. Appended last so
    // it shifts no existing func index. Present iff `n_init == 1`.
    if n_init == 1 {
        uleb128(init_type_idx as u64, &mut func_items);
    }
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + n_synth + n_init, &func_items),
    );
    let make_abs = (defined_type_base + n) as u32;
    let encode_abs = make_abs + 1;
    let realloc_abs = encode_abs + 1;
    // Method i's core func index (the methods follow realloc, in list order).
    let method_abs = |i: usize| realloc_abs + 1 + i as u32;
    // The init func's ABSOLUTE index — named by the START section, run once at instantiation. It follows the
    // methods: import_count (`e+k+2`) + n defined bodies + n_synth (make/encode/realloc + methods).
    let init_abs = (e + k + 2 + n + n_synth) as u32;

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
        for (i, meth) in methods.iter().enumerate() {
            let name = match meth {
                CoreMethod::Len => "t-len",
                CoreMethod::IsEmpty => "t-is-empty",
                CoreMethod::ToBytes => "t-to-bytes",
            };
            items.extend_from_slice(&export(name, wasm_abi::EXPORT_KIND_FUNC, method_abs(i)));
        }
        let n_exports = 4 + methods.len();
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(n_exports, &items))
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
        // A scalar-erased flat form lays its ONE template exactly like `Flat`; the only difference is the
        // `make` body boxes the scalar (below) before `resource-new`.
        EscapeForm::FlatScalar { tpl, .. } => vec![*tpl],
        EscapeForm::Sum(s) => s.variants.iter().collect(),
        // RuntimeBytes writes its entire output at run time (variable length) — no preloaded template,
        // no data section. The walker uses a fixed retarea at offset 0 and writes the value form after it.
        EscapeForm::RuntimeBytes(_) => Vec::new(),
        // RecursiveSum bakes its descriptor into the data section (below, as a constant blob the walker
        // reads to build the heap descriptor Bytes); no value-form template.
        EscapeForm::RecursiveSum(_) => Vec::new(),
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
    // RecursiveSum needs no data-section blob: its `encode()` builds the descriptor heap Bytes with
    // literal `i32.const` stores (the descriptor bytes are compile-time constants) and delegates the
    // walk to the runtime `value-encode` op.
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
    // make: forward the export's params, `call <export>` (builds the compound → its heap handle on the
    // stack) then `call resource-new` (register the handle → a resource handle). A NULLARY export forwards
    // zero params (byte-identical to the old `make()`); a SCALAR-param export threads its scalar params
    // (locals `0..p`) into the body call; a COMPOUND-param export REBUILDS the cell in-guest from the
    // flattened leaf params (the canonical ABI flattened its `tuple<…>` param into scalar leaves — the same
    // `emit_cell_rebuild` a closure `call` uses) and passes that one handle. So the compound is computed
    // from the host's arguments however they cross.
    {
        let (inner, imp, drop_cells) = {
            let imp = |name: &str| import_index[name] as u64;
            // Each COMPOUND slot needs one i32 local to stash its rebuilt cell handle (for the post-build
            // `local.tee`); scalar slots use no local. The flattened leaf params occupy locals `0..L`, so
            // the compound-cell locals start at `L` (`make_param_vts.len()`), one per compound slot.
            let n_cell_locals = make_core_slots
                .iter()
                .filter(|s| matches!(s, MakeCoreSlot::Tuple(..)))
                .count();
            // MEM-LEAF/VALUE-FORM params need extra i32 scratch: two SHARED locals (`buf`, `ctr`) for the
            // per-byte copy-in loop when any mem-leaf param is present, plus per-param locals — a value-form
            // needs two (`desc`, `res`), and every borrowed mem-leaf needs one to stash its lifted handle for
            // the post-call reclaim. All i32.
            let n_memleaf = make_core_slots
                .iter()
                .filter(|s| matches!(s, MakeCoreSlot::MemLeaf { .. }))
                .count();
            let n_scratch = if n_memleaf > 0 { 2 } else { 0 };
            let n_vf_locals: usize = make_core_slots
                .iter()
                .filter(|s| {
                    matches!(
                        s,
                        MakeCoreSlot::MemLeaf {
                            kind: MemLeafKind::ValueForm(_),
                            ..
                        }
                    )
                })
                .count()
                * 2;
            let n_drop_locals = make_core_slots
                .iter()
                .filter(|s| {
                    matches!(
                        s,
                        MakeCoreSlot::MemLeaf {
                            drop_after: true,
                            ..
                        }
                    )
                })
                .count();
            let n_locals = n_cell_locals + n_scratch + n_vf_locals + n_drop_locals;
            let mut inner = if n_locals == 0 {
                uleb_bytes(0) // no locals — scalar params are forwarded directly
            } else {
                let mut l = uleb_bytes(1); // one local group…
                uleb128(n_locals as u64, &mut l); // …of `n_locals` i32s
                l.push(wasm_abi::CORE_I32);
                l
            };
            // Push each parameter as the export body expects it, in param order — a SCALAR leaf directly
            // (`local.get`), a COMPOUND rebuilt into its cell (from its run of flattened leaves) — threading
            // a leaf cursor across the params AND a cell-local cursor across the compound slots. A mix of
            // scalar + compound, and multiple compounds, compose: leaves run left-to-right, each compound
            // reads its own contiguous run.
            let mut leaf_cursor = 0u32;
            let l_params = make_param_vts.len() as u32;
            let mut cell_local = l_params;
            // Mem-leaf scratch: `buf`/`ctr` are the two SHARED copy-in locals (right after the tuple cells);
            // per-param value-form `desc`/`res` and per-param reclaim locals are handed out from `next_extra`.
            let buf = l_params + n_cell_locals as u32;
            let ctr = buf + 1;
            let mut next_extra = l_params + n_cell_locals as u32 + n_scratch as u32;
            // The cell-locals of the compound params the export only BORROWS — deep-dropped after the call
            // (the make wrapper owns the freshly-rebuilt cell; `emit_tuple_rebuild` tee'd it into the local
            // exactly "for the post-dispatch drop"). A non-droppable slot (a moved-out field) is omitted, so
            // its cell stays live (a defined leak, never a double-free).
            let mut drop_cells: Vec<u32> = Vec::new();
            for slot in make_core_slots {
                match slot {
                    MakeCoreSlot::Scalar => {
                        inner.push(op::LOCAL_GET);
                        uleb128(leaf_cursor as u64, &mut inner);
                        leaf_cursor += 1;
                    }
                    MakeCoreSlot::Tuple(fields, drop_after) => {
                        // Rebuild this compound's cell from the leaves at `leaf_cursor..`; `emit_tuple_rebuild`
                        // stashes into `cell_local` and leaves the handle on the stack as the arg.
                        let rebuild = TupleArgRebuild {
                            fields: fields.clone(),
                            base_param: leaf_cursor,
                        };
                        emit_tuple_rebuild(&rebuild, cell_local, &imp, &mut inner);
                        leaf_cursor += fields.iter().map(FieldRebuild::leaf_count).sum::<u32>();
                        if *drop_after {
                            drop_cells.push(cell_local);
                        }
                        cell_local += 1;
                    }
                    // A memory-bearing / value-form leaf: lift the `(ptr, len)` at `leaf_cursor` via the SAME
                    // shared emitters the typed-interface wrapper uses (per-byte path, `bulk_bytes=false` — no
                    // shared-allocator dependency), leaving the lifted value-heap handle on the stack as the def
                    // arg. A borrowed param is stashed into a reclaim local and dropped after the export call.
                    MakeCoreSlot::MemLeaf {
                        kind,
                        drop_after,
                        desc,
                    } => {
                        match kind {
                            MemLeafKind::Str | MemLeafKind::Bytes => {
                                emit_bytes_leaf_copy_in(
                                    leaf_cursor,
                                    false, // a top-level string/bytes ptr is i32 (no variant-join widening)
                                    buf,
                                    ctr,
                                    false, // per-byte copy-in (no shared allocator on the resource make path)
                                    &imp,
                                    &mut inner,
                                ); // → [handle]
                            }
                            MemLeafKind::ValueForm(_) => {
                                let desc_local = next_extra;
                                let res_local = next_extra + 1;
                                next_extra += 2;
                                emit_value_form_lift(
                                    desc,
                                    leaf_cursor,
                                    buf,
                                    ctr,
                                    desc_local,
                                    res_local,
                                    false, // per-byte copy-in
                                    &imp,
                                    &mut inner,
                                ); // → [handle]
                            }
                            // `list<scalar>` params are deferred (classified out in `export_make_params`); this
                            // arm is unreachable on the make path today.
                            MemLeafKind::List(_) => {
                                return Err(
                                    "a list<scalar> make param is not yet lifted on the resource path"
                                        .to_string(),
                                );
                            }
                        }
                        leaf_cursor += 2; // the string/value-form flattened to (ptr, len)
                        if *drop_after {
                            // Stash the lifted handle (it stays on the stack as the def arg) for the post-call
                            // reclaim — the make wrapper owns it and the def only BORROWS it.
                            let dl = next_extra;
                            next_extra += 1;
                            inner.push(op::LOCAL_TEE);
                            uleb128(dl as u64, &mut inner);
                            drop_cells.push(dl);
                        }
                    }
                }
            }
            (inner, imp, drop_cells)
        };
        let _ = imp;
        let mut inner = inner;
        inner.push(op::CALL);
        uleb128(export_abs as u64, &mut inner);
        // Reclaim each borrow-only compound param cell the make wrapper owns (a dead owned temporary now the
        // export has returned). `drop` takes the handle and leaves the export result beneath untouched
        // (stack-balanced). Guarded on `drop` being imported (the heap-return escape path always imports it);
        // a missing import can only skip the reclaim (leak), never mis-emit.
        if let Some(&drop_idx) = import_index.get("drop") {
            for dl in &drop_cells {
                inner.push(op::LOCAL_GET);
                uleb128(*dl as u64, &mut inner);
                inner.push(op::CALL);
                uleb128(drop_idx as u64, &mut inner);
            }
        }
        // A SCALAR-ERASED result (a runtime Qty) leaves a bare scalar on the stack, not an i32 heap handle —
        // BOX it (`box-int` : (S64)->i32 handle) so `resource-new` gets a real rep. A compound export already
        // returns its handle, so no box for `Flat`/`Sum`/etc. A NARROW-int inner is an i32 core value, so
        // i32→i64 widen it FIRST (signed/unsigned per `extend`) — `box-int` takes i64, and an unextended i32
        // would be an invalid arg. This is the make-side twin of `emit_box_i32_to_i64_extend`.
        if let EscapeForm::FlatScalar { box_op, extend, .. } = &form {
            if let Some(signed) = extend {
                inner.push(if *signed {
                    op::I64_EXTEND_I32_S
                } else {
                    op::I64_EXTEND_I32_U
                });
            }
            inner.push(op::CALL);
            uleb128(import_index[*box_op] as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // t-encode(self: BORROW<t>): the canonical ABI passes the heap REP directly as the param (a borrow is
    // not a table index — wasmtime `lift_borrow` returns the rep), so the walker uses `RepSource::Borrow`
    // (no `resource.rep`, no reclaiming drop — the host keeps ownership and the dtor reclaims on drop, so
    // the resource is repeatable). `resource-rep` stays imported (index math unchanged) but is now unused
    // by `t-encode` — only `make` uses `resource-new`. [[rcdzc-r1-resource-encode-linking-findings]].
    let _ = f_rrep; // borrow: the rep is the param, so resource.rep is not called
    let rep_src = RepSource::Borrow;
    let encode_body = match &form {
        // A scalar-erased flat form walks IDENTICALLY to `Flat`: `make` boxed the scalar into the root cell,
        // so the template's single leaf hole reads `get-int` off the rep at an empty path, exactly as for a
        // one-element runtime tuple.
        EscapeForm::Flat(t) | EscapeForm::FlatScalar { tpl: t, .. } => encode_walk_body(
            t,
            placed[0].byte_off,
            placed[0].ret_off,
            rep_src,
            &import_index,
        ),
        EscapeForm::Sum(s) => {
            encode_sum_walk_body(&s.variants, &placed_pairs(&placed), rep_src, &import_index)
        }
        EscapeForm::RuntimeBytes(form) => encode_bytes_walk_body(form, rep_src, &import_index),
        EscapeForm::RecursiveSum(desc) => {
            encode_recursive_sum_walk_body(desc, rep_src, &import_index)
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
    // Each extra method body (borrow rep = the i32 param — no resource.rep, no drop, repeatable):
    for meth in methods {
        let body = match meth {
            // t-len(rep) -> i32: `bytes-len(rep)`.
            CoreMethod::Len => {
                let f_bytes_len = *import_index
                    .get("bytes-len")
                    .expect("Len method requires the bytes-len op imported");
                let mut inner = uleb_bytes(0); // no locals
                inner.push(op::LOCAL_GET);
                uleb128(0, &mut inner); // the borrow self param = the rep
                inner.push(op::CALL);
                uleb128(f_bytes_len as u64, &mut inner);
                inner.push(op::END);
                inner
            }
            // t-is-empty(rep) -> i32: `bytes-len(rep) == 0` (i32.eqz) — crosses as bool.
            CoreMethod::IsEmpty => {
                let f_bytes_len = *import_index
                    .get("bytes-len")
                    .expect("IsEmpty method requires the bytes-len op imported");
                let mut inner = uleb_bytes(0); // no locals
                inner.push(op::LOCAL_GET);
                uleb128(0, &mut inner);
                inner.push(op::CALL);
                uleb128(f_bytes_len as u64, &mut inner);
                inner.push(op::I32_EQZ); // len == 0
                inner.push(op::END);
                inner
            }
            // t-to-bytes(rep) -> i32: copy the RAW payload into the (ptr=OUT,len=n) retarea, return 0.
            CoreMethod::ToBytes => to_bytes_body(&import_index),
        };
        let mut e = uleb_bytes(body.len() as u64);
        e.extend_from_slice(&body);
        code_items.extend_from_slice(&e);
    }
    // The STATIC-COMPOUND `start` init body LAST — built as an ordinary `SelectedFunc` (`() -> ()`, no locals;
    // the buffers thread on the stack via arr-set/mark-immortal's FBIP return) so `code_entry` resolves its
    // ops (`arr-alloc`/`arr-set`/`box-*`/`mark-immortal[-deep]`/`vec-of-arr`/`map-*`/`set-*`) through the same
    // `import_index` the bodies use. `static_compound_init` `global.set`s each once-built immortal to `0..n`.
    if n_init == 1 {
        // The static-compound init is stack-threaded EXCEPT a hoisted Map/Set whose key/element emit stashes
        // scratch in i32 locals — a nested map/set-in-key or a list-key canonicalize (`emit`/`emit_key_
        // canonicalize` allocate `local.set`/`local.tee` slots). Declare EXACTLY the scratch the init
        // references (`1 + max local index used`, 0 if none), or those `local.*` reference undeclared locals =
        // invalid wasm ("unknown local N: local index out of bounds"). This is the component/resource-escape
        // twin of the ikc1/itf2 bug the OTHER three start-assembly sites already guard with
        // `layout.static_compound_init_locals`; recomputed here since this assembler takes only the init `Lir`,
        // not the layout — the SAME max+1 scan `Layout::with_static_compounds` uses. The old `Vec::new()`
        // (a "no locals — buffers thread on the stack" assumption) was false for a hoisted Map/Set key.
        let init_locals = static_compound_init
            .iter()
            .filter_map(|op| match op {
                Lir::LocalGet(i) | Lir::LocalSet(i) | Lir::LocalTee(i) => Some(*i),
                _ => None,
            })
            .max()
            .map_or(0, |m| m + 1);
        let init = SelectedFunc {
            params: Vec::new(),
            ret: crate::ty::Ty::Unit,
            code: static_compound_init.to_vec(),
            declared: vec![ValType::I32; init_locals as usize],
            src_body: None,
            locals: Vec::new(),
            scopes: Vec::new(),
            stmt_lines: Vec::new(),
        };
        code_items.extend_from_slice(&code_entry(&init, &import_index));
    }
    let code_sec = section(
        wasm_abi::CORE_SEC_CODE,
        &wasm_vec(n + n_synth + n_init, &code_items),
    );

    // ── Table + Element sections ── one funcref table holding the lambda-lifted closures, so a first-class
    // closure's `call_indirect (table 0)` resolves (mirrors `core_module_impl` ~749). Empty for a
    // closure-free program → no sections (byte-identical to before). The element segment fills slot `k`
    // with `lifted_table[k]` (the lifted body's absolute func index, computed by the caller from
    // `layout.lifted_abs`), the same slot a `Core::Closure { code: k }` selects.
    let (table_sec, elem_sec) = if lifted_table.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let n_lifted = lifted_table.len();
        let mut table_entry = vec![0x70u8]; // funcref element type
        table_entry.push(0x01); // limits flag: has-max
        uleb128(n_lifted as u64, &mut table_entry); // min
        uleb128(n_lifted as u64, &mut table_entry); // max
        let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
        let mut seg = Vec::new();
        seg.push(0x00); // active, table 0, funcref, func-index list
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg); // offset 0
        seg.push(op::END);
        let mut idxs = Vec::new();
        for &abs in lifted_table {
            uleb128(abs as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        let elem_sec = section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg));
        (table_sec, elem_sec)
    };

    // ── Global section (id 6) ── one mutable i32 per STATIC COMPOUND (indices `0..n_compounds`, init 0), the
    // START init overwrites each with its once-built immortal handle. This builder has NO other globals (its
    // `cabi_realloc` is a stub with no cursor), so the static-compound globals ARE globals `0..n_compounds` —
    // exactly the indices `try_emit_static_compound` emits (`static_bytes.len()==0 + pos`). Empty when there
    // are no static compounds → no section, byte-identical.
    let global_sec = if n_compounds > 0 {
        let mut items = Vec::new();
        for _ in 0..n_compounds {
            let mut g = vec![wasm_abi::CORE_I32, 0x01]; // i32, mutable
            g.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(0, &mut g);
            g.push(op::END);
            items.extend_from_slice(&g);
        }
        section(wasm_abi::CORE_SEC_GLOBAL, &wasm_vec(n_compounds, &items))
    } else {
        Vec::new()
    };
    // ── Start section (id 8) ── names the static-compound init func, run once at instantiation to build every
    // static compound into its global. Between EXPORT (7) and ELEMENT (9). Present iff `n_init == 1`.
    let start_sec = if n_init == 1 {
        section(wasm_abi::CORE_SEC_START, &uleb_bytes(init_abs as u64))
    } else {
        Vec::new()
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    // Table (id 4) after func (id 3), before memory (id 5) — the canonical core section order.
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    // Global (id 6) after memory (id 5), before export (id 7).
    core.extend_from_slice(&global_sec);
    core.extend_from_slice(&export_sec);
    // Start (id 8) after export (id 7), before element (id 9).
    core.extend_from_slice(&start_sec);
    // Element (id 9) after start (id 8), before code (id 10).
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    Ok(core)
}

/// The CLOSURE-RESOURCE core module (C-HOST-1): a program whose export RESULT is a closure `(-> A… R)`
/// crosses as a resource whose `call` method invokes the closure. Like `runtime_resource_core_module`
/// but the resource method is `call` (not `encode`): it recovers the closure CELL rep, reads the code
/// slot, and `call_indirect`s the lifted body with the caller's args.
///
///  * `make(export-params…) -> i32` — forward the export's parameters, `call <export>` (which builds the
///    closure cell, closing over those params → its handle), then `resource.new` to register the cell as
///    the resource's rep. A NULLARY export gives `make()`; a PARAMETERIZED export (`(def (adder k) (fn (x)
///    (+ x k)))`) gives `make(k)`, so the host computes a distinct closure per input (C-HOST-2). The
///    captured params ride in the cell, so `call` is unchanged.
///  * `call(self, args…) -> R` — `resource.rep(self)` recovers the cell rep; `arr-get(cell, 0)` +
///    `get-int` + `i32.wrap_i64` reads the funcref-table slot; push the args (locals `1..1+arity`), push
///    the cell as the env (local slot 0 of the lifted fn), then `call_indirect` the lifted functype over
///    table 0. This is the boundary form of `Core::CallClosure`: the closure logic stays in the guest;
///    the host only holds the rep and drives `call`.
///
/// The funcref TABLE + ELEMENT sections come from the selected export body's `layout.lifted` (the lifted
/// closure occupies a table slot), emitted exactly as `core_module` does. `arg_vts`/`ret_vt` are the
/// closure's CORE valtypes (from its `Ty::Fn`), `make_param_vts` the EXPORT's core param valtypes (empty
/// for a nullary export), `lifted_type_idx` the core type index of the lifted functype `(env, args…) -> R`
/// (`layout.lifted_type_index(0, import_count)`).
/// One closure EXPORT's `make` function, for the multi-export path. Each closure export gets its own
/// `make` (which calls that export's body to build the cell, closing over that export's params, then
/// `resource.new`s the handle); all same-signature exports SHARE the one `call` method. A single-export
/// program is the N=1 case ([`closure_resource_core_module`]).
#[derive(Clone)]
pub struct ClosureMake {
    /// The boundary export name — `"make"` for a single closure export, `"make-<def-name>"` for each of
    /// several (so the host picks the one it wants by the source export name).
    pub export_name: String,
    /// The core function index of this export's BODY (which builds the closure cell) — `make` calls it.
    pub export_abs: u32,
    /// This export's parameter core valtypes (empty for a nullary export) — `make` forwards them.
    pub param_vts: Vec<ValType>,
}

/// One PLAIN (non-closure) export riding alongside the closure exports in the SAME program core module
/// (the "closure ALONGSIDE a non-closure export" shape). Its body is already among `funcs` (every reachable
/// def is selected), so the core module needs no new functype or code for it — only an EXPORT entry naming
/// its core-func index, so the outer envelope can alias + lift it as an ORDINARY top-level component func.
#[derive(Clone)]
pub struct PlainExport {
    /// The core-module export name (the envelope aliases the program instance by this) = the source name.
    pub export_name: String,
    /// The core function index of this export's body (already defined in `funcs`).
    pub body_abs: u32,
}

/// How the shared `call` reassembles ONE flattened compound argument into the single value-heap CELL its
/// lifted closure body expects. A FIXED-SHAPE SCALAR tuple/record closure argument crosses the DIRECT-CALL
/// boundary as a native component `tuple<…>`/`record<…>` type, which the canonical ABI FLATTENS into scalar
/// core params — so the core `call` receives the fields as N separate core params, but the lifted body reads
/// the argument as a SINGLE i32 cell handle (`arr-get`/`get-int` projections). This descriptor tells the
/// `call` body to rebuild that cell in-guest (`arr-alloc N`, then per field: index, the flattened param,
/// box, `arr-set`) — the exact `Core::Tuple` build shape (`select.rs`) — and push the resulting handle in
/// place of the raw fields. Proven runnable by the `a_fixed_shape_tuple_closure_arg_crosses_by_native_
/// flattening` oracle. `None` (the common case) is byte-identical to the scalar path.
///
/// One resource-`make` PARAMETER's core-side plan: a SCALAR leaf (forwarded to the export body directly
/// via `local.get`) or a fixed-shape TUPLE/record (rebuilt into its value-heap cell from its contiguous
/// run of flattened leaf params, via [`emit_cell_rebuild`]). `make` iterates these in param order,
/// threading a leaf cursor (across all params) and a cell-local cursor (across the compound slots), so any
/// MIX of scalar + compound params — and multiple compounds — composes.
#[derive(Clone)]
pub enum MakeCoreSlot {
    /// A scalar parameter — one flattened leaf, forwarded as-is.
    Scalar,
    /// A fixed-shape scalar tuple/record parameter — its per-field rebuild; consumes its fields' leaves. The
    /// `bool` = the make wrapper deep-drops this rebuilt cell after the export call (the def only BORROWS it,
    /// per `record_cell_param_droppable`); `false` keeps the cell (a moved-out field / unproven shape) so the
    /// drop never double-frees a live child.
    Tuple(Vec<FieldRebuild>, bool),
    /// A memory-bearing (`String`/`Bytes`/`list<scalar>`) or value-form (`BigInt`/`Rational`/`Symbol`) leaf
    /// parameter — consumes TWO flattened leaves (the canonical `(ptr, len)`) and lifts them in the make body
    /// via the SAME shared emitters the typed-interface wrapper uses (`emit_bytes_leaf_copy_in` /
    /// `emit_list_leaf_lift` / `emit_value_form_lift`, per-byte path). `desc` is the baked shape-descriptor
    /// bytes for a [`MemLeafKind::ValueForm`] (empty otherwise). `drop_after` = the make wrapper reclaims the
    /// lifted handle after the (borrowing) export call.
    MemLeaf {
        kind: MemLeafKind,
        drop_after: bool,
        desc: Vec<u8>,
    },
}

#[derive(Clone)]
pub struct TupleArgRebuild {
    /// The fields of this compound, in cell order. Each is either an aliased-width SCALAR leaf (consumes ONE
    /// flattened core param, boxed with a single op) or a NESTED fixed-shape compound (consumes its own
    /// flattened leaves depth-first, rebuilt into its own sub-cell first). A NESTED field builds an i32 handle
    /// which the parent `arr-set`s directly (no box op). See [`FieldRebuild`].
    pub fields: Vec<FieldRebuild>,
    /// The CORE-PARAM index this compound's flattened LEAVES start at. `1` when the tuple is the SOLE closure
    /// arg (leaves at params `1..1+L`, after `self`=0). When the tuple sits AMONG scalar args, the PREFIX
    /// scalar args occupy params `1..base_param` and the tuple's leaves `base_param..base_param+L`; the SUFFIX
    /// scalars follow. The shared `call` body pushes prefix scalars, the rebuilt tuple, then suffix scalars.
    /// A NESTED compound field's leaves continue in the SAME flat sequence (depth-first), so this base is the
    /// running leaf cursor the recursive [`emit_tuple_rebuild`] threads.
    pub base_param: u32,
}

/// One field of a [`TupleArgRebuild`] cell: an aliased-width SCALAR leaf (a single flattened core param,
/// boxed) or a NESTED fixed-shape compound (its own sub-fields, rebuilt into a sub-cell whose i32 handle the
/// parent stores directly). Mirrors the value-heap `Core::Tuple`/`Core::Record` build shape recursively.
#[derive(Clone)]
pub enum FieldRebuild {
    /// A scalar leaf: `box_op` (`"box-int"`/`"box-bool"`/`"box-float"`/`"box-float32"`) applied to the one
    /// flattened core param at the running leaf cursor; `extend` = `Some(signed)` when a NARROW int (an i32
    /// core param) must be i32→i64 extended before `box-int` (which takes i64), else `None`.
    Scalar {
        box_op: &'static str,
        extend: Option<bool>,
    },
    /// A nested fixed-shape compound field: rebuild its own sub-cell (recursively, consuming the next
    /// contiguous run of flattened leaf params) → an i32 handle the parent `arr-set`s AS-IS (no box op). The
    /// `Vec<u32>` is the sub-cell's per-field SLOTS (each sub-field's name-lex cell position) — so a nested
    /// WIT record whose field order ≠ name-lex (the message's `sender{reducer, host}`, name-lex `host,
    /// reducer`) still lands its fields in the right slots. Identity `(0..n)` for a guest-constructed
    /// tuple/record (the closure-arg path).
    Nested(Vec<FieldRebuild>, Vec<u32>),
    /// A `list<u8>` leaf: the canon lift flattens it to `(ptr: i32, len: i32)` — TWO consecutive
    /// flattened core params. The wrapper allocates a guest `Bytes` of `len` (`bytes-alloc`), copies
    /// `len` bytes out of linear memory 0 starting at `ptr` (`i32.load8_u` + `bytes-set` per byte), and
    /// stores the resulting handle into the record slot AS-IS (no box op — a `Bytes` handle, like
    /// `Nested`). Consumes TWO flattened leaf params. Needs the two scratch locals the wrapper reserves
    /// (a `buf` handle + a copy counter) and the core module owning memory 0 + a `cabi_realloc`
    /// (`wrapper_needs_memory`) so the canon lift can lower the incoming list into that memory.
    BytesLeaf,
    /// A `list<scalar>` leaf (rpp3's `xs: list<s64>`): the canon lift flattens it to `(ptr: i32, len: i32)` —
    /// TWO consecutive flattened core params. The wrapper builds a value-heap vec by reading + boxing each
    /// element out of linear memory 0 per its width (`emit_list_leaf_lift`, exactly like a top-level
    /// [`MemLeafKind::List`] lift), and stores the resulting vec handle into the record slot AS-IS (no box op —
    /// a vec handle, like `Nested`/`BytesLeaf`). Consumes TWO flattened leaf params. Needs the same two scratch
    /// locals the wrapper reserves (a `buf` vec accumulator + an element cursor) and memory 0 (`has_bytes_leaf`
    /// ⇒ true). Only a FLAT list (`nest_lists == 0`) is admitted — a nested `list<list<…>>` field is a later
    /// slice — so the element lift needs no fresh locals and a throwaway `next_local` suffices.
    ListLeaf(ListElem),
    /// A variant/`option`/`result` PARAM field (the response's `answer`): the canon lift flattens it to
    /// `(disc, payload…)`; the wrapper branches on the boundary disc and rebuilds the guest sum cell
    /// (`sum-new`), leaving its handle for the parent `arr-set`. Reuses the closure-arg [`SumArgRebuild`]
    /// (its `base_param` is IGNORED here — the disc is read at the record cursor). Consumes
    /// `flattened_param_count()` leaves.
    Sum(Box<SumArgRebuild>),
}

impl FieldRebuild {
    /// The number of FLATTENED core leaf params this field consumes (1 for a scalar; the recursive sum for a
    /// nested compound). Used to thread the running leaf cursor across sibling fields.
    fn leaf_count(&self) -> u32 {
        match self {
            FieldRebuild::Scalar { .. } => 1,
            FieldRebuild::Nested(sub, _) => sub.iter().map(FieldRebuild::leaf_count).sum(),
            // A `list<u8>` flattens to `(ptr, len)` — two core params.
            FieldRebuild::BytesLeaf => 2,
            // A `list<scalar>` likewise flattens to `(ptr, len)` — two core params.
            FieldRebuild::ListLeaf(_) => 2,
            FieldRebuild::Sum(r) => r.flattened_param_count(),
        }
    }

    /// Collect the runtime ops this field (recursively) references, so the `call` core imports them. A
    /// nested field has no op of its own (its handle is stored as-is) but its leaves do; a scalar leaf its
    /// box op; a `list<u8>` leaf the `bytes-alloc`/`bytes-set` its copy-in loop calls.
    pub fn collect_box_ops(&self, out: &mut impl FnMut(&'static str)) {
        // Default per-byte (closure/tuple/resource paths — no shared allocator at lower-time).
        self.collect_box_ops_gated(false, out);
    }

    /// Like [`collect_box_ops`] but gated: `bulk_bytes` selects the bulk `bytes-new` copy-in (a shared
    /// allocator exists at lower-time — the host-`_mem` typed-interface path) vs the per-byte
    /// `bytes-alloc` + `bytes-set` loop. MUST match `emit_bytes_leaf_copy_in`'s gate exactly.
    pub fn collect_box_ops_gated(&self, bulk_bytes: bool, out: &mut impl FnMut(&'static str)) {
        match self {
            FieldRebuild::Scalar { box_op, .. } => out(box_op),
            FieldRebuild::Nested(sub, _) => {
                for f in sub {
                    f.collect_box_ops_gated(bulk_bytes, out);
                }
            }
            FieldRebuild::BytesLeaf => {
                if bulk_bytes {
                    out("bytes-new");
                } else {
                    out("bytes-alloc");
                    out("bytes-set");
                }
            }
            // The list copy-in loop (`emit_list_leaf_lift`): build the vec (`vec-empty`), then per element
            // read + box (`elem.box_op`) + append (`vec-push`).
            FieldRebuild::ListLeaf(elem) => {
                out("vec-empty");
                out("vec-push");
                out(elem.box_op);
            }
            FieldRebuild::Sum(r) => {
                r.arm_true.collect_ops_gated(bulk_bytes, out);
                r.arm_false.collect_ops_gated(bulk_bytes, out);
                out("sum-new");
            }
        }
    }

    /// Whether this field (recursively) contains a memory-bearing leaf — a `list<u8>`/`Bytes` (`BytesLeaf`)
    /// OR a `list<scalar>` (`ListLeaf`) — whose copy-in loop the wrapper carrying it must reserve the two
    /// scratch locals for, and for which the core module must own memory 0.
    pub fn has_bytes_leaf(&self) -> bool {
        match self {
            FieldRebuild::Scalar { .. } => false,
            FieldRebuild::Nested(sub, _) => sub.iter().any(FieldRebuild::has_bytes_leaf),
            FieldRebuild::BytesLeaf => true,
            // A `list<scalar>` field copies its elements out of memory 0 via the same scratch pair.
            FieldRebuild::ListLeaf(_) => true,
            // A sum arm's `list<u8>` (Bytes) payload, or a Compound payload carrying a bytes leaf, needs the
            // scratch locals + memory 0; a scalar/nullary/enum arm does not.
            FieldRebuild::Sum(r) => [&r.arm_true, &r.arm_false]
                .iter()
                .any(|a| match &a.payload {
                    SumArmPayload::Bytes { .. } => true,
                    SumArmPayload::Compound(fs) => fs.iter().any(FieldRebuild::has_bytes_leaf),
                    _ => false,
                }),
        }
    }
}

/// One arm of a two-arm [`SumArgRebuild`]: how the guest builds ONE variant's sum cell from the flattened
/// payload param. `decl_disc` is the variant's index in Cadenza's decl (what `sum-new` stamps); `payload_box`
/// is `Some((box_op, extend))` when this variant carries one aliased-width scalar payload (boxed into the
/// cell), or `None` for a nullary variant (the inline-unit payload). Both `Option`'s `Some`/`None` and
/// `Result`'s `Ok`/`Err` are two-arm sums; a nullary arm has `payload_box: None`.
#[derive(Clone)]
pub struct SumArgArm {
    /// This variant's discriminant IN CADENZA'S DECL — `sum-new(decl_disc, …)` stamps it (a later guest
    /// `match` on the rebuilt cell dispatches on this). May differ from the boundary disc.
    pub decl_disc: u32,
    /// How this arm builds its payload from the flattened core param(s): a nullary variant (inline unit), a
    /// single boxed scalar leaf, or a compound (tuple/record) cell rebuilt from a run of flattened leaves.
    pub payload: SumArmPayload,
}

impl SumArgArm {
    /// Register (via `out`) every runtime op this arm's payload rebuild emits, so the `call` core imports them:
    /// a scalar arm's box op; a compound arm's `arr-alloc`/`arr-set` + each field's box op (recursively). A
    /// nullary arm emits none. (`sum-new` itself is registered by the caller once per sum.)
    pub fn collect_ops(&self, out: &mut impl FnMut(&'static str)) {
        // Default per-byte (closure/tuple/resource paths — no shared allocator at lower-time).
        self.collect_ops_gated(false, out);
    }

    /// Like [`collect_ops`] but gated: `bulk_bytes` selects `bytes-new` (a shared allocator exists at
    /// lower-time) vs the per-byte `bytes-alloc` + `bytes-set` loop. MUST match `emit_bytes_leaf_copy_in`.
    pub fn collect_ops_gated(&self, bulk_bytes: bool, out: &mut impl FnMut(&'static str)) {
        match &self.payload {
            SumArmPayload::Nullary => {}
            SumArmPayload::Scalar { box_op, .. } => out(box_op),
            SumArmPayload::Compound(fields) => {
                out("arr-alloc");
                out("arr-set");
                for f in fields {
                    f.collect_box_ops_gated(bulk_bytes, out);
                }
            }
            SumArmPayload::Bytes { .. } => {
                if bulk_bytes {
                    out("bytes-new");
                } else {
                    out("bytes-alloc");
                    out("bytes-set");
                }
            }
            // An enum payload builds the inner all-nullary cell via `sum-new`.
            SumArmPayload::Enum => out("sum-new"),
            // A list<scalar> payload builds a value-heap vec (per-element read/box/push).
            SumArmPayload::List(elem) => {
                out("vec-empty");
                out("vec-push");
                out(elem.box_op);
            }
        }
    }
}

/// How one [`SumArgArm`] builds its variant payload from the flattened core param(s) at the sum's payload base.
#[derive(Clone)]
pub enum SumArmPayload {
    /// A nullary variant — no payload param; the cell's payload is the inline unit (`IMM_UNIT`).
    Nullary,
    /// A single aliased-width scalar payload: `box_op` applied to the one flattened leaf at the payload base.
    /// `extend` = `Some(signed)` when a NARROW int leaf (an i32 core param) must be i32→i64 extended before
    /// `box-int`. `wrap_join` = the flattened payload slot is a WIDER core JOIN (i64) than this arm's own
    /// narrow (i32-core) payload — the different-width `result<ok,err>` case, where the canonical ABI joins the
    /// payload slot to the wider side; the guest `i32.wrap_i64`s the joined param FIRST (recovering the narrow
    /// value's low 32 bits) before this arm's `extend`. Always `false` for an Option arm (single payload, no
    /// join) and a same-core-width Result. Proven by the diff-width Result oracle.
    Scalar {
        box_op: &'static str,
        extend: Option<bool>,
        wrap_join: bool,
    },
    /// A COMPOUND (fixed-shape tuple/record) payload: the arm rebuilds a value-heap cell from the payload's
    /// recursively-flattened leaves (consuming a contiguous run of core params starting at the payload base),
    /// exactly as a bare tuple arg rebuilds via [`emit_cell_rebuild`], then `sum-new`s the variant over that
    /// cell handle. The payload crossed the boundary as `option<tuple<…>>` (both formers anonymous-allowed, so
    /// no `variant` naming wall). Proven by the `an_option_tuple_payload_closure_arg_crosses_by_native_
    /// flattening` oracle.
    Compound(Vec<FieldRebuild>),
    /// A `list<u8>` / `string` payload (an `Ok = list<u8>` arm — the reducer response's
    /// `answer: result<payload, error>` — or an `Err = string` arm, e.g. `result<Int64, String>`, erp1). The
    /// payload flattened to `(ptr, len)` — TWO consecutive leaves at the payload base; the arm allocates a
    /// guest `Bytes` and copies the bytes out of memory 0 (exactly like a top-level [`FieldRebuild::BytesLeaf`];
    /// a `String` builds the SAME UTF-8 byte-leaf handle as a `Bytes`, no decode). Needs the wrapper's scratch
    /// locals + memory 0. `ptr_from_i64` = the canonical variant JOIN widened the payload's FIRST slot (the
    /// `ptr`) to `i64` because the OTHER arm carries a wider (`i64`) scalar there — the different-width Result
    /// case (erp1): the guest `i32.wrap_i64`s the joined `ptr` slot to recover the i32 address before the copy.
    /// `false` for a same-width payload slot (both arms i32-core at slot0 — the reducer `result<list<u8>, enum>`).
    Bytes { ptr_from_i64: bool },
    /// An all-nullary variant / WIT `enum` payload (an `Err = error` arm). The enum flattened to ONE i32 disc
    /// leaf at the payload base. Every case is nullary and the guest declares the same enum in the same case
    /// order, so the boundary disc IS the guest decl disc: the arm reads the disc leaf and builds the inner
    /// cell `sum-new(disc, IMM_UNIT)` (no per-case branch) as this arm's payload. One leaf.
    Enum,
    /// A `list<scalar>` payload (an `option<list<Int64>>` Some arm — eop2). The list flattened to `(ptr, len)`
    /// — TWO consecutive leaves at the payload base; the arm builds a value-heap vec by reading + boxing each
    /// element (exactly like a top-level [`MemLeafKind::List`] lift), leaving the vec handle as this arm's
    /// payload. The [`ListElem`] carries the scalar leaf's read/box; only a FLAT list (`nest_lists == 0`) is
    /// admitted here (a nested list-in-option is a later slice). Reuses the wrapper's scratch locals as the
    /// vec accumulator + cursor (like the `Bytes` arm reuses them for the byte copy).
    List(ListElem),
}

/// How a closure `call` reassembles ONE flattened fixed-shape SUM argument (an `Option`/`Result` — a
/// two-variant sum, each variant nullary OR carrying ≤1 aliased-width scalar payload) into the single
/// value-heap CELL its lifted body expects. The sum crosses the DIRECT-CALL boundary as a native component
/// `option<T>` (one payload + one nullary) or `result<ok,err>` (two payloads), which the canonical ABI
/// FLATTENS into `(disc: i32, payload)` core params — the disc at `base_param`, the payload leaf at
/// `base_param + 1` (the payload slot is the JOIN of both cases' scalars). This descriptor tells the `call`
/// body to branch on the flattened boundary disc and build each arm's cell via `sum-new` (the exact
/// `Core::SumNew` shape), pushing the resulting handle in place of the raw params. Proven runnable by the
/// `an_option_scalar_closure_arg_crosses_by_native_flattening` + `a_result_scalar_closure_arg_crosses_by_
/// native_flattening` oracles. TRAP: the disc has TWO conventions: the guest BRANCHES on the boundary disc
/// (`option`/`result` send case-1 for the 2nd case), but BUILDS with each arm's DECL disc.
#[derive(Clone)]
pub struct SumArgRebuild {
    /// The CORE-PARAM index the flattened sum's `disc` (i32) lands at; the payload is at `base_param + 1`.
    /// `1` when the sum is the SOLE closure arg (after `self`=0).
    pub base_param: u32,
    /// The boundary disc value that selects `arm_true` — the value the flattened `disc` carries for that
    /// case. For `option<T>` this is `1` (Some); for `result<ok,err>` `0` (Ok). The guest branches
    /// `disc == boundary_true_disc ? arm_true : arm_false`.
    pub boundary_true_disc: u32,
    /// The arm built when `disc == boundary_true_disc` (Some for option, Ok for result).
    pub arm_true: SumArgArm,
    /// The arm built otherwise (None for option, Err for result).
    pub arm_false: SumArgArm,
}

impl SumArgRebuild {
    /// The number of FLATTENED core params this sum consumes: the `disc` (1) + its payload's leaf count. A
    /// scalar/nullary payload is 1 leaf (the join slot is present even for a nullary arm), so the classic
    /// `option<scalar>`/`result<scalar,scalar>` case is 2. A COMPOUND (Option-of-tuple) payload contributes its
    /// recursively-flattened leaf count, so the sum spans `1 + leaves`. Used to skip the sum's params when
    /// walking the flattened arg list.
    pub fn flattened_param_count(&self) -> u32 {
        // Both arms flatten into the SAME payload slot(s) (the join); take the widest (a nullary arm has 0
        // OWN leaves but shares the 1 scalar join slot). The payload-carrying arm's leaf count is authoritative.
        let arm_leaves = |arm: &SumArgArm| -> u32 {
            match &arm.payload {
                SumArmPayload::Nullary => 0,
                SumArmPayload::Scalar { .. } => 1,
                SumArmPayload::Compound(fields) => {
                    fields.iter().map(FieldRebuild::leaf_count).sum()
                }
                // A `list<u8>` (Bytes) or `list<scalar>` payload flattens to `(ptr, len)` — two leaves; an
                // enum to one disc leaf.
                SumArmPayload::Bytes { .. } | SumArmPayload::List(_) => 2,
                SumArmPayload::Enum => 1,
            }
        };
        1 + arm_leaves(&self.arm_true)
            .max(arm_leaves(&self.arm_false))
            .max(1)
    }
}

/// Emit ONE arm's cell build: `sum-new(decl_disc, payload)` where the payload is the inline unit (nullary), a
/// boxed scalar leaf, or a rebuilt compound cell. Leaves `[sum-handle]` on stack. `payload_param` is the
/// flattened core-param index the payload's leaf/leaves start at (the sum's `base_param + 1`).
fn emit_sum_arm(
    arm: &SumArgArm,
    payload_param: u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    scratch: Option<(u32, u32)>,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(arm.decl_disc as i64, out); // [disc]
    match &arm.payload {
        SumArmPayload::Scalar {
            box_op,
            extend,
            wrap_join,
        } => {
            out.push(op::LOCAL_GET);
            uleb128(payload_param as u64, out); // [disc, payload-leaf]
            if *wrap_join {
                // The joined payload slot is the WIDER core (i64) but THIS arm's payload is narrow (i32-core):
                // the narrow value arrived widened into the join, so recover its low 32 bits before this arm's
                // own (re-)extend. `i32.wrap_i64` keeps the low 32 bits (the raw narrow value, sign/zero
                // irrelevant here since `extend` below re-applies the correct one). See the diff-width oracle.
                out.push(op::I32_WRAP_I64);
            }
            if let Some(signed) = extend {
                out.push(if *signed {
                    op::I64_EXTEND_I32_S
                } else {
                    op::I64_EXTEND_I32_U
                });
            }
            out.push(op::CALL);
            uleb128(imp(box_op), out); // [disc, payload-handle]
        }
        SumArmPayload::Compound(fields) => {
            // Rebuild the payload's value-heap cell from its recursively-flattened leaves, starting at the
            // payload base — exactly as a bare tuple arg rebuilds. Leaves the cell handle on the stack.
            let mut cursor = payload_param;
            emit_cell_rebuild(fields, &mut cursor, bulk_bytes, imp, None, None, out); // [disc, payload-cell-handle]
        }
        SumArmPayload::Nullary => {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(
                crate::backend::wasm::runtime_abi::IMM_UNIT as i64,
                out,
            ); // [disc, unit]
        }
        SumArmPayload::Bytes { ptr_from_i64 } => {
            // The `list<u8>`/`string` payload crossed as `(ptr, len)` at the payload base; copy it into a guest
            // `Bytes` (exactly like a top-level `BytesLeaf`; a `String` IS the same UTF-8 byte-leaf), leaving the
            // handle as this arm's payload. `ptr_from_i64` wraps the joined `i64` ptr slot to i32 first (the
            // different-width Result join — erp1).
            let (buf, ctr) = scratch.expect("a Bytes sum arm needs the wrapper's scratch locals");
            emit_bytes_leaf_copy_in(payload_param, *ptr_from_i64, buf, ctr, bulk_bytes, imp, out); // [disc, bytes-handle]
        }
        SumArmPayload::Enum => {
            // The enum payload crossed as ONE i32 disc leaf; build the inner all-nullary cell
            // `sum-new(disc, IMM_UNIT)` (boundary disc == guest decl disc — same case order) as this arm's
            // payload.
            out.push(op::LOCAL_GET);
            uleb128(payload_param as u64, out); // [disc, enum-disc]
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(
                crate::backend::wasm::runtime_abi::IMM_UNIT as i64,
                out,
            ); // [disc, enum-disc, unit]
            out.push(op::CALL);
            uleb128(imp("sum-new"), out); // [disc, enum-cell]
        }
        SumArmPayload::List(elem) => {
            // The `list<scalar>` payload crossed as `(ptr, len)` at the payload base; build a value-heap vec
            // (exactly like a top-level `MemLeafKind::List` lift), leaving the vec handle as this arm's payload.
            // Reuse the wrapper's scratch pair as the vec accumulator + element cursor (like the Bytes arm).
            let (buf, ctr) = scratch.expect("a list sum arm needs the wrapper's scratch locals");
            // A FLAT list (`nest_lists == 0`) allocates no extra locals, so a throwaway `next_local` suffices;
            // the classifier admits only flat lists here (a nested list-in-option is a later slice).
            debug_assert_eq!(
                elem.nest_lists, 0,
                "only a flat list payload is admitted in a sum arm"
            );
            let mut nl = 0u32;
            emit_list_leaf_lift(elem, payload_param, buf, ctr, &mut nl, imp, out); // [disc, vec-handle]
        }
    }
    out.push(op::CALL);
    uleb128(imp("sum-new"), out); // [sum-handle]
}

/// Emit the SUM-arg CELL REBUILD for a closure `call` body: reassemble the single fixed-shape sum argument
/// (which crossed FLATTENED as `(disc, payload)` core params) into the one i32 sum-cell handle the lifted
/// body expects, leaving the handle on the stack AND stashed in `sum_local` (dropped after `call_indirect`).
/// `if disc == boundary_true_disc { arm_true } else { arm_false }`, each arm via [`emit_sum_arm`].
fn emit_sum_arg_rebuild(
    rebuild: &SumArgRebuild,
    sum_local: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let disc_param = rebuild.base_param;
    let payload_param = rebuild.base_param + 1;
    // Branch on the BOUNDARY disc (the component-model convention), NOT the decl disc.
    out.push(op::LOCAL_GET);
    uleb128(disc_param as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(rebuild.boundary_true_disc as i64, out);
    out.push(op::I32_EQ);
    out.push(op::IF);
    out.push(wasm_abi::CORE_I32); // block type: → i32 (the sum handle)
    // Closure sum args carry scalar/nullary/compound payloads only (no `list<u8>`/enum arm) — no scratch.
    // Closure sum-arg rebuild: no shared allocator at lower-time → per-byte (bulk_bytes=false).
    emit_sum_arm(&rebuild.arm_true, payload_param, false, imp, None, out);
    out.push(op::ELSE);
    emit_sum_arm(&rebuild.arm_false, payload_param, false, imp, None, out);
    out.push(op::END);
    // stash for the post-dispatch drop; leaves [sum-handle] on the stack.
    out.push(op::LOCAL_TEE);
    uleb128(sum_local as u64, out);
}

/// Emit the sum rebuild for a record-cell FIELD ([`FieldRebuild::Sum`]): like [`emit_sum_arg_rebuild`] but the
/// disc/payload are read at the record's running cursor (NOT the closure `base_param`), and the handle is left
/// on the stack for the parent `arr-set` (no drop-stash — the parent record owns it). Advances `*cursor` past
/// the sum's flattened `(disc, payload…)`.
fn emit_sum_field(
    rebuild: &SumArgRebuild,
    cursor: &mut u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    scratch: Option<(u32, u32)>,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let disc_param = *cursor;
    let payload_param = *cursor + 1;
    out.push(op::LOCAL_GET);
    uleb128(disc_param as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(rebuild.boundary_true_disc as i64, out);
    out.push(op::I32_EQ);
    out.push(op::IF);
    out.push(wasm_abi::CORE_I32); // block type: → i32 (the sum handle)
    emit_sum_arm(
        &rebuild.arm_true,
        payload_param,
        bulk_bytes,
        imp,
        scratch,
        out,
    );
    out.push(op::ELSE);
    emit_sum_arm(
        &rebuild.arm_false,
        payload_param,
        bulk_bytes,
        imp,
        scratch,
        out,
    );
    out.push(op::END); // → [sum-handle]
    *cursor += rebuild.flattened_param_count();
}

/// Emit the tuple-arg CELL REBUILD for a closure `call` body: reassemble the single fixed-shape tuple/record
/// argument (which crossed the boundary FLATTENED into its N scalar fields at core params `1..1+N`) into the
/// one i32 cell handle the lifted body expects — `arr-alloc N` + per field (index, the flattened param, box,
/// `arr-set`; the FBIP array threaded on the stack) — leaving the handle on the stack AND stashed in
/// `tuple_local`. Caller must have pushed nothing between (the array threads from `arr-alloc`), and drops the
/// cell after `call_indirect` via [`emit_tuple_rebuilt_drop`]. Shared by every list-result `call` body (bytes/
/// value-form/value-encode) + the scalar body; `imp(name) -> import index`. See [`TupleArgRebuild`].
fn emit_tuple_rebuild(
    rebuild: &TupleArgRebuild,
    tuple_local: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    // Rebuild the TOP-LEVEL cell, threading the leaf cursor from `base_param`; `local.tee` the resulting
    // handle into `tuple_local` for the post-dispatch drop (only the OUTER cell is dropped — its nested
    // sub-cells are its elements, reclaimed with it).
    let mut cursor = rebuild.base_param;
    // Closure tuple-arg rebuild: no shared allocator at lower-time → per-byte (bulk_bytes=false).
    emit_cell_rebuild(&rebuild.fields, &mut cursor, false, imp, None, None, out);
    out.push(crate::backend::wasm::wasm_abi::op::LOCAL_TEE);
    uleb128(tuple_local as u64, out); // stash for the post-dispatch drop; leaves [arr] on the stack
}

/// Emit ONE value-heap cell for a run of [`FieldRebuild`] fields, consuming flattened leaf params from
/// `*cursor` (advanced past each leaf, depth-first). `arr-alloc N` + per field: index, then either the boxed
/// scalar leaf OR a recursively-rebuilt nested sub-cell handle OR a `list<u8>` copied out of memory, then
/// `arr-set` (FBIP array threaded on the stack). Leaves the cell handle on the stack. Recursion mirrors
/// `Core::Tuple`/`Core::Record`. `scratch` = `Some((buf_local, i_local))` when this cell (or a nested one)
/// carries a `BytesLeaf` — the two reusable scratch locals its copy-in loop threads (`buf` handle + counter);
/// `None` for any rebuild with no bytes leaf (every non-wrapper caller — closure/tuple/sum rebuilds — which
/// never carry one). A `BytesLeaf` reached with `scratch == None` is a caller bug (`.expect`).
fn emit_cell_rebuild(
    fields: &[FieldRebuild],
    cursor: &mut u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    scratch: Option<(u32, u32)>,
    slots: Option<&[u32]>,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(fields.len() as i64, out);
    out.push(op::CALL);
    uleb128(imp("arr-alloc"), out); // [arr]
    // `fields` are consumed in WIT/flattened-param order (the cursor advances sequentially); each is stored
    // at its cell SLOT — `slots[i]` when the record permutes (a WIT field whose name-lex slot ≠ its WIT
    // position), else the position `i` (a name-lex-ordered record / a nested rebuild, identity). This is how
    // a declaration-ordered WIT record param (the real `message`) lands its fields in the value-heap cell's
    // name-lex slots the def reads.
    for (i, field) in fields.iter().enumerate() {
        let slot = slots.map(|s| s[i]).unwrap_or(i as u32);
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(slot as i64, out); // [arr, slot]
        match field {
            FieldRebuild::Scalar { box_op, extend } => {
                out.push(op::LOCAL_GET);
                uleb128(*cursor as u64, out); // the flattened leaf param → [arr, i, leaf]
                *cursor += 1;
                if let Some(signed) = extend {
                    out.push(if *signed {
                        op::I64_EXTEND_I32_S
                    } else {
                        op::I64_EXTEND_I32_U
                    });
                }
                out.push(op::CALL);
                uleb128(imp(box_op), out);
            }
            FieldRebuild::Nested(sub, sub_slots) => {
                // Rebuild the nested sub-cell (consumes its own leaves in WIT order) → an i32 handle stored
                // AS-IS. Its own fields permute into their name-lex slots via `sub_slots`.
                emit_cell_rebuild(sub, cursor, bulk_bytes, imp, scratch, Some(sub_slots), out); // → [arr, i, sub-handle]
            }
            FieldRebuild::BytesLeaf => {
                let (buf, ctr) = scratch.expect("a BytesLeaf needs the wrapper's scratch locals");
                emit_bytes_leaf_copy_in(*cursor, false, buf, ctr, bulk_bytes, imp, out); // → [arr, i, buf]
                *cursor += 2; // the list flattened to (ptr, len)
            }
            FieldRebuild::ListLeaf(elem) => {
                // The `list<scalar>` field crossed as `(ptr, len)` at `*cursor`; build a value-heap vec into the
                // wrapper's scratch pair (exactly like the sum-arm `SumArmPayload::List` lift), leaving the vec
                // handle for the parent `arr-set`. A FLAT list allocates no fresh locals, so a throwaway
                // `next_local` suffices (the classifier admits only `nest_lists == 0` here).
                let (buf, ctr) = scratch.expect("a ListLeaf needs the wrapper's scratch locals");
                debug_assert_eq!(elem.nest_lists, 0, "only a flat list field is admitted");
                let mut nl = 0u32;
                emit_list_leaf_lift(elem, *cursor, buf, ctr, &mut nl, imp, out); // → [arr, i, vec]
                *cursor += 2; // the list flattened to (ptr, len)
            }
            FieldRebuild::Sum(rebuild) => {
                emit_sum_field(rebuild, cursor, bulk_bytes, imp, scratch, out); // → [arr, i, sum-handle]
            }
        }
        out.push(op::CALL);
        uleb128(imp("arr-set"), out); // → [arr]
    }
}

/// Emit the copy-in for one `BytesLeaf`: the list crossed the boundary as `(ptr, len)` at flattened core
/// params `ptr_leaf` (= `*cursor`) and `ptr_leaf + 1`. Build the guest `Bytes` in ONE bulk call —
/// `bytes-new((ptr, len))` — instead of `bytes-alloc` + a per-byte `bytes-set` loop (the ~1.9us/byte
/// reducer-fold marshaling cost, operator seq 916). The marshaling-boundary bytes are already CONTIGUOUS in
/// linear memory 0 (the core module owns it under `wrapper_needs_memory`), so the `list<u8>` arg lowers to
/// exactly the `(ptr_leaf, len_leaf)` core params the canon adapter copies across into the runtime heap.
/// Leaves the fresh `Bytes` handle on the stack (the caller `arr-set`s it AS-IS); the surrounding stack
/// (`[arr, i]`) is untouched — this is a balanced `[] -> [handle]`. An empty list (`len == 0`) yields the
/// shared immortal empty-`Bytes` singleton. The `buf`/`ctr` scratch locals are no longer needed (the bulk
/// call carries no loop), kept in the signature so callers need not renumber their reserved scratch.
fn emit_bytes_leaf_copy_in(
    ptr_leaf: u32,
    ptr_from_i64: bool,
    buf: u32,
    ctr: u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let len_leaf = ptr_leaf + 1;
    // Read the `ptr` core-param slot, wrapping i64→i32 when the variant JOIN widened it (a different-width
    // Result whose OTHER arm carries an i64 scalar at slot0 — erp1). The `len` slot is always i32.
    let get_ptr = |out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(ptr_leaf as u64, out);
        if ptr_from_i64 {
            out.push(op::I32_WRAP_I64);
        }
    };
    if bulk_bytes {
        // handle = bytes-new(ptr, len) — the `list<u8>` arg is canon-lowered to the `(ptr, len)` pair, read
        // straight out of linear memory 0; one cross-component call replaces the alloc + per-byte-set loop.
        // Only valid where the envelope provides a shared allocator at lower-time (the host-`_mem` assembler).
        get_ptr(out);
        out.push(op::LOCAL_GET);
        uleb128(len_leaf as u64, out);
        out.push(op::CALL);
        uleb128(imp("bytes-new"), out);
        // leaves the Bytes handle on the stack for the caller's arr-set ([] -> [handle]).
        return;
    }
    // PER-BYTE FALLBACK (no shared allocator at lower-time — e.g. a non-host typed interface, whose
    // runtime-op canon-lower cannot carry the Memory option `bytes-new` needs): `buf = bytes-alloc(len)`,
    // then loop `j in 0..len` copying `bytes-set(buf, j, i32.load8_u(ptr + j))` out of linear memory 0.
    // buf = bytes-alloc(len)
    out.push(op::LOCAL_GET);
    uleb128(len_leaf as u64, out);
    out.push(op::CALL);
    uleb128(imp("bytes-alloc"), out);
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out);
    // ctr = 0
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(0, out);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    // block { loop { if ctr >= len br 1; buf = bytes-set(buf, ctr, load8(ptr + ctr)); ctr += 1; br 0 } }
    out.push(op::BLOCK);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    out.push(op::LOOP);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(len_leaf as u64, out);
    out.push(op::I32_GE_U);
    out.push(op::BR_IF);
    uleb128(1, out);
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    get_ptr(out);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::I32_ADD);
    out.push(op::I32_LOAD8_U);
    out.push(0x00); // align 2^0
    out.push(0x00); // offset 0
    out.push(op::CALL);
    uleb128(imp("bytes-set"), out);
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(1, out);
    out.push(op::I32_ADD);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    out.push(op::BR);
    uleb128(0, out);
    out.push(op::END); // end loop
    out.push(op::END); // end block
    // leave buf on the stack for the caller's arr-set
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
}

/// Emit the lift for one top-level VALUE-FORM leaf param (`BigInt`/`Rational`/`Symbol`) — a type with no
/// scalar boundary rep that crosses as the canonical `list<u8>` value-form at flattened core params
/// `ptr_leaf` / `ptr_leaf + 1`. Copies those bytes out of linear memory 0 into a value-heap byte-leaf, bakes
/// the type's shape `desc` into a second byte-leaf, then `value-decode(bytes, desc)` reconstructs the
/// value-heap handle (the PARAM twin of `Core::ValueDecode`, R2). `value-decode` BORROWS both leaves, so the
/// wrapper (their owner) drops both here; the decoded handle is left on the stack as the def arg (`[]->[h]`).
/// `buf`/`ctr` are the two reusable byte-copy scratch locals; `desc_local`/`res_local` are two fresh i32
/// locals (the baked descriptor handle + the decoded-result stash). A NULL decode (a host contract
/// violation — a non-`Option` param's bytes are a valid encoding by contract) is left as-is; the borrowed
/// slice's post-call reclaim drops it like any handle.
#[allow(clippy::too_many_arguments)]
fn emit_value_form_lift(
    desc: &[u8],
    ptr_leaf: u32,
    buf: u32,
    ctr: u32,
    desc_local: u32,
    res_local: u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    // 1. Bake the descriptor into `desc_local` first (clean stack for the operand copy). `desc-buf =
    //    bytes-alloc(len)`, then thread it through one `bytes-set(buf, j, byte)` per descriptor byte (each
    //    returns the — possibly reallocated — handle, left on the stack for the next).
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(desc.len() as i64, out);
    out.push(op::CALL);
    uleb128(imp("bytes-alloc"), out); // [desc-buf]
    for (j, &byte) in desc.iter().enumerate() {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(j as i64, out);
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(byte as i64, out);
        out.push(op::CALL);
        uleb128(imp("bytes-set"), out); // [desc-buf]
    }
    out.push(op::LOCAL_SET);
    uleb128(desc_local as u64, out); // [] descriptor stored
    // 2. Copy the boundary `(ptr, len)` value-form bytes into a value-heap byte-leaf; normalize into `buf`
    //    (the per-byte path already leaves it there, the bulk `bytes-new` path leaves it only on the stack).
    emit_bytes_leaf_copy_in(ptr_leaf, false, buf, ctr, bulk_bytes, imp, out); // → [bytes]
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out); // [] bytes stored in buf
    // 3. handle = value-decode(bytes, desc) — borrows both; stash the result (NULL on a mismatch).
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out); // [bytes]
    out.push(op::LOCAL_GET);
    uleb128(desc_local as u64, out); // [bytes, desc]
    out.push(op::CALL);
    uleb128(imp("value-decode"), out); // [handle-or-null]
    out.push(op::LOCAL_SET);
    uleb128(res_local as u64, out); // [] handle stashed
    // 4. Drop the borrowed-only temporaries (the copied bytes + the baked descriptor). The decoded value in
    //    `res_local` is independent of both, so this is safe before leaving it as the def arg.
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
    out.push(op::LOCAL_GET);
    uleb128(desc_local as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
    // 5. Leave the decoded handle on the stack as the def arg.
    out.push(op::LOCAL_GET);
    uleb128(res_local as u64, out); // [handle]
}

/// Emit the lift for one top-level `list<scalar>` param: the list crossed the boundary as `(ptr, len)` at
/// flattened core params `ptr_leaf` / `ptr_leaf + 1` (len = the ELEMENT count). Build a value-heap vec
/// (`vec-empty`) and loop `j in 0..len` pushing `box(load(ptr + j*stride))` per the [`ListElem`] — each
/// element read at its natural canonical stride, optionally i32→i64 extended (a narrow int), boxed, and
/// `vec-push`ed. Leaves the vec handle on the stack (the caller passes it as the def arg directly).
/// `buf`/`ctr` are the two reusable scratch locals; stack-balanced (`[]->[handle]`). An empty list yields
/// the fresh empty vec.
fn emit_list_leaf_lift(
    elem: &ListElem,
    ptr_leaf: u32,
    buf: u32,
    ctr: u32,
    next_local: &mut u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    // Build the outer list at boundary leaves (ptr_leaf, ptr_leaf+1) into `buf`, descending `nest_lists`
    // sub-list levels; then leave the outer vec handle on the stack for the def call.
    emit_list_level(
        elem,
        elem.nest_lists,
        ptr_leaf,
        ptr_leaf + 1,
        buf,
        ctr,
        next_local,
        imp,
        out,
    );
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
}

/// Build ONE list level into `buf` from the `(ptr_local, len_local)` descriptor, leaving the stack balanced
/// (the result is left in `buf`, NOT on the stack — the caller reads `buf`). `levels` = remaining nested
/// list depth: `0` → each element is the scalar leaf (load + box + push); `k>0` → each element is a
/// `(ptr, len)` sub-list at canonical stride 8 (load its ptr/len into fresh locals, recursively build it into
/// a fresh inner `buf`, then push that vec handle). `next_local` hands each nested level its own scratch
/// locals so no level clobbers another's cursor/accumulator.
#[allow(clippy::too_many_arguments)]
fn emit_list_level(
    elem: &ListElem,
    levels: u32,
    ptr_local: u32,
    len_local: u32,
    buf: u32,
    ctr: u32,
    next_local: &mut u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    // A nested element is a `(ptr,len)` sub-list = 8 canonical bytes; a scalar leaf uses its own stride.
    let stride: u32 = if levels > 0 { 8 } else { elem.stride };
    // For a nested level, pre-allocate this level's per-element scratch (inner ptr/len + inner vec/cursor).
    let (inner_ptr, inner_len, inner_buf, inner_ctr) = if levels > 0 {
        let base = *next_local;
        *next_local += 4;
        (base, base + 1, base + 2, base + 3)
    } else {
        (0, 0, 0, 0)
    };
    // buf = vec-empty()
    out.push(op::CALL);
    uleb128(imp("vec-empty"), out);
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out);
    // ctr = 0
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(0, out);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    // block { loop { if ctr >= len br 1; <build+push element>; ctr += 1; br 0 } }
    out.push(op::BLOCK);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    out.push(op::LOOP);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    // if ctr >= len -> br 1 (exit)
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(len_local as u64, out);
    out.push(op::I32_GE_U);
    out.push(op::BR_IF);
    uleb128(1, out);
    // addr = ptr + ctr*stride  (helper: pushes the element address onto the stack)
    let emit_addr = |out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(ptr_local as u64, out);
        out.push(op::LOCAL_GET);
        uleb128(ctr as u64, out);
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(stride as i64, out);
        out.push(op::I32_MUL);
        out.push(op::I32_ADD);
    };
    if levels == 0 {
        // buf = vec-push(buf, box(load(addr)))
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        emit_addr(out); // [buf, addr]
        out.push(elem.load_op);
        uleb128(elem.load_align as u64, out);
        uleb128(0, out); // offset 0 → [buf, elem]
        if let Some(signed) = elem.extend {
            out.push(if signed {
                op::I64_EXTEND_I32_S
            } else {
                op::I64_EXTEND_I32_U
            });
        }
        out.push(op::CALL);
        uleb128(imp(elem.box_op), out); // [buf, boxed]
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    } else {
        // inner_ptr = i32.load(addr + 0); inner_len = i32.load(addr + 4) — the canonical list descriptor.
        emit_addr(out);
        out.push(op::I32_LOAD);
        uleb128(2, out); // align log2(4)
        uleb128(0, out); // offset 0 (ptr)
        out.push(op::LOCAL_SET);
        uleb128(inner_ptr as u64, out);
        emit_addr(out);
        out.push(op::I32_LOAD);
        uleb128(2, out);
        uleb128(4, out); // offset 4 (len)
        out.push(op::LOCAL_SET);
        uleb128(inner_len as u64, out);
        // Recursively build the inner list into inner_buf (leaves the stack balanced), then push its handle.
        emit_list_level(
            elem,
            levels - 1,
            inner_ptr,
            inner_len,
            inner_buf,
            inner_ctr,
            next_local,
            imp,
            out,
        );
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        out.push(op::LOCAL_GET);
        uleb128(inner_buf as u64, out); // [buf, inner-vec]
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    }
    // ctr += 1
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(1, out);
    out.push(op::I32_ADD);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    out.push(op::BR);
    uleb128(0, out);
    out.push(op::END); // end loop
    out.push(op::END); // end block
}

/// Emit the RESULT-SPILL for a wrapper whose def returns a value-heap compound HANDLE: store the handle
/// (on the stack from the def `call`) to `rec`, allocate a `size`-byte return area (`cabi_realloc(0, 0,
/// align, size)`) into `retptr`, write the value's canonical form there via [`emit_canon_write`], and leave
/// `retptr` on the stack as the boundary result. `next_local` hands the writer fresh scratch locals. The canon
/// lift reads the value back out of memory from that pointer.
#[allow(clippy::too_many_arguments)]
fn emit_result_spill(
    rec: u32,
    retptr: u32,
    next_local: &mut u32,
    realloc_abs: u64,
    size: u32,
    align: u32,
    write: &CanonWrite,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    // rec = def result handle (currently on the stack).
    out.push(op::LOCAL_SET);
    uleb128(rec as u64, out);
    // retptr = cabi_realloc(old_ptr=0, old_size=0, align, size)
    const_i32(0, out);
    const_i32(0, out);
    const_i32(align as i64, out);
    const_i32(size as i64, out);
    out.push(op::CALL);
    uleb128(realloc_abs, out);
    out.push(op::LOCAL_SET);
    uleb128(retptr as u64, out);
    // Write the value's canonical form at retptr + 0.
    emit_canon_write(
        write,
        rec,
        retptr,
        0,
        next_local,
        realloc_abs,
        bulk_bytes,
        imp,
        out,
    );
    // Reclaim the def's RESULT handle: the def returned an OWNED compound (callee-owns-args → the caller, this
    // wrapper, owns the result), and the canonical writer only BORROWED it (arr-get/vec-get/sum-disc/bytes-len
    // are borrowing reads that retain nothing), so after the write `rec` holds the sole reference to the whole
    // value tree — `drop` deep-reclaims it (the tree's children too). Without this the spilled result cell
    // (+ its boxed children) LEAKED one per call (the SpillRecord-result known-leak class, SHAPE 60/62/63).
    out.push(op::LOCAL_GET);
    uleb128(rec as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
    // Return the area pointer.
    out.push(op::LOCAL_GET);
    uleb128(retptr as u64, out);
}

/// Lower a `list<u8>`/`Bytes` RESULT member (a def returning a value-heap Bytes handle) to the canonical
/// `list<u8>` return in ONE bulk call — `bytes-read(rec)` — instead of `bytes-len` + a per-byte `bytes-get`
/// copy loop (the ~1.9us/byte reducer-fold marshaling cost, operator seq 916). `bytes-read` is canon-lowered
/// with Memory+Realloc options, so its `list<u8>` result (2 flats > `MAX_FLAT_RESULTS`) is written by the
/// adapter — allocating the buffer via the guest's OWN `cabi_realloc` in guest memory 0 — into a
/// caller-provided 8-byte return area as `(ptr, len)` at `[+0]`/`[+4]`. That area IS the member's canonical
/// `list<u8>` return, so we allocate it, hand it to `bytes-read` as the trailing retptr arg, and return it
/// directly — no second buffer, no copy loop. `bytes-read` BORROWS `rec` (rc unchanged), so we still DROP the
/// owned def-result handle afterwards, exactly as the old `bytes-get` path did. `rec`/`retptr` are the two
/// scratch i32 locals the caller reserved; `next_local` is no longer drawn from (the bulk call has no loop
/// scratch), kept in the signature so callers need not change.
fn emit_result_copy_bytes(
    rec: u32,
    retptr: u32,
    next_local: &mut u32,
    realloc_abs: u64,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };
    let call = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(imp(name), out);
    };
    if bulk_bytes {
        // rec = the def's result Bytes handle (currently on the stack).
        set(rec, out);
        // retptr = cabi_realloc(0, 0, align=4, size=8) — the (ptr,len) return area bytes-read fills. Only
        // valid where the envelope provides a shared allocator at lower-time (the host-`_mem` assembler).
        const_i32(0, out);
        const_i32(0, out);
        const_i32(4, out);
        const_i32(8, out);
        out.push(op::CALL);
        uleb128(realloc_abs, out);
        set(retptr, out);
        // bytes-read(rec, retptr) -> () : one bulk call writes retptr[0]=guest ptr, retptr[4]=len (the buffer
        // allocated in guest memory 0 by the canon adapter's realloc). Args: buf first, then trailing retptr.
        get(rec, out);
        get(retptr, out);
        call("bytes-read", out);
        // Drop the owned def result handle (bytes-read only BORROWED it).
        get(rec, out);
        call("drop", out);
        // Return the area pointer (the member's canonical (ptr,len) list<u8> return).
        get(retptr, out);
        return;
    }
    // PER-BYTE FALLBACK (no shared allocator at lower-time): `bytes-len` + a `bytes-get` per-byte copy loop
    // into a `cabi_realloc`'d buffer, then write `(ptr, len)` into a `cabi_realloc`'d 8-byte return area.
    let (n, buf, i) = (*next_local, *next_local + 1, *next_local + 2);
    *next_local += 3;
    // rec = the def's result Bytes handle (currently on the stack).
    set(rec, out);
    // n = bytes-len(rec)
    get(rec, out);
    call("bytes-len", out);
    set(n, out);
    // buf = cabi_realloc(orig=0, orig_size=0, align=1, size=n)
    const_i32(0, out);
    const_i32(0, out);
    const_i32(1, out);
    get(n, out);
    out.push(op::CALL);
    uleb128(realloc_abs, out);
    set(buf, out);
    // COPY LOOP: i = 0; while i < n { store8(buf + i, bytes-get(rec, i)); i++ }
    const_i32(0, out);
    set(i, out);
    out.push(op::BLOCK);
    out.push(wasm_abi::BLOCK_EMPTY);
    out.push(op::LOOP);
    out.push(wasm_abi::BLOCK_EMPTY);
    {
        get(i, out);
        get(n, out);
        out.push(op::I32_GE_U);
        out.push(op::BR_IF);
        uleb128(1, out);
        get(buf, out);
        get(i, out);
        out.push(op::I32_ADD);
        get(rec, out);
        get(i, out);
        call("bytes-get", out);
        out.push(op::I32_STORE8);
        out.push(0x00);
        out.push(0x00);
        get(i, out);
        const_i32(1, out);
        out.push(op::I32_ADD);
        set(i, out);
        out.push(op::BR);
        uleb128(0, out);
    }
    out.push(op::END);
    out.push(op::END);
    // retptr = cabi_realloc(0, 0, align=4, size=8) — the (ptr,len) return area.
    const_i32(0, out);
    const_i32(0, out);
    const_i32(4, out);
    const_i32(8, out);
    out.push(op::CALL);
    uleb128(realloc_abs, out);
    set(retptr, out);
    // retptr[0] = buf (ptr), retptr[4] = n (len) — i32 stores, 4-byte aligned.
    get(retptr, out);
    get(buf, out);
    out.push(op::I32_STORE);
    out.push(0x02);
    out.push(0x00);
    get(retptr, out);
    get(n, out);
    out.push(op::I32_STORE);
    out.push(0x02);
    out.push(0x04);
    // Drop the def result handle (the wrapper consumed it into the buffer).
    get(rec, out);
    call("drop", out);
    // Return the area pointer.
    get(retptr, out);
}

/// Recursively write ONE value-heap value's canonical-ABI form into linear memory at `dst_base + offset`.
/// `handle` is the local holding the value's runtime handle; `dst_base` a local holding the base address;
/// `offset` a static byte offset. `next_local` hands out fresh i32 scratch locals (per-level handles, list
/// loop counters). See [`CanonWrite`] for the per-kind plan.
#[allow(clippy::too_many_arguments)]
fn emit_canon_write(
    cw: &CanonWrite,
    handle: u32,
    dst_base: u32,
    offset: u32,
    next_local: &mut u32,
    realloc_abs: u64,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };
    let call = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(imp(name), out);
    };
    // Emit an `i32.store` (align hint 4, offset `off`) — the caller pushed [addr, value] first.
    let store_i32_at = |off: u32, out: &mut Vec<u8>| {
        out.push(op::I32_STORE);
        out.push(0x02);
        uleb128(off as u64, out);
    };
    match cw {
        CanonWrite::Scalar {
            read,
            wrap_i64,
            store,
        } => {
            // store(dst_base + offset) = [wrap] read(handle)
            get(dst_base, out);
            get(handle, out);
            call(read, out);
            if *wrap_i64 {
                out.push(op::I32_WRAP_I64);
            }
            out.push(*store);
            out.push(0x00); // align hint (conservative)
            uleb128(offset as u64, out);
        }
        CanonWrite::EnumDisc { store } => {
            // store(dst_base + offset) = handle (the BARE i32 discriminant, stored directly — no unbox).
            get(dst_base, out);
            get(handle, out);
            out.push(*store);
            out.push(0x00); // align hint (conservative)
            uleb128(offset as u64, out);
        }
        CanonWrite::Record { fields } => {
            for f in fields {
                let fh = *next_local;
                *next_local += 1;
                get(handle, out);
                const_i32(f.index as i64, out);
                call("arr-get", out);
                set(fh, out);
                emit_canon_write(
                    &f.write,
                    fh,
                    dst_base,
                    offset + f.offset,
                    next_local,
                    realloc_abs,
                    bulk_bytes,
                    imp,
                    out,
                );
            }
        }
        CanonWrite::Bytes if bulk_bytes => {
            // BULK (seq-916 loop-ii): `bytes-read(handle, dst_base+offset)` writes the (ptr,len) pair straight
            // into this field's canonical `list<u8>` slot — retptr[0]=guest ptr, retptr[4]=len — in ONE call,
            // replacing `bytes-len` + `cabi_realloc` + the per-byte `bytes-get` copy loop the fallback below
            // does (the nested-Bytes LOWER twin of `emit_result_copy_bytes`'s bare-result bulk). The guest's
            // canon adapter allocates the buffer in memory 0 via its own realloc, exactly like the fallback's
            // `cabi_realloc(0,0,1,count)`, so the resulting (ptr,len) slot is identical downstream. `bytes-read`
            // BORROWS `handle` (rc unchanged) and `handle` is itself a borrowing `arr-get` read the enclosing
            // `emit_result_spill` deep-drops with the whole tree — so, exactly like the per-byte `bytes-get`
            // path, NO drop is emitted here. Only valid where the assembler canon-lowers `bytes-read` + provides
            // the shared allocator (the `import_realloc`/`_mem` mode the caller gates on).
            get(handle, out); // buf
            get(dst_base, out);
            const_i32(offset as i64, out);
            out.push(op::I32_ADD); // retptr = dst_base + offset (this field's (ptr,len) slot)
            call("bytes-read", out);
        }
        CanonWrite::Bytes => {
            // PER-BYTE FALLBACK (no shared allocator / no bytes-read canon-lower at lower-time):
            // count = bytes-len(handle); ptr = cabi_realloc(0,0,1,count); copy loop; store (ptr, count).
            let count = *next_local;
            let ptr = *next_local + 1;
            let i = *next_local + 2;
            *next_local += 3;
            get(handle, out);
            call("bytes-len", out);
            set(count, out);
            const_i32(0, out);
            const_i32(0, out);
            const_i32(1, out); // align 1 for bytes
            get(count, out);
            out.push(op::CALL);
            uleb128(realloc_abs, out);
            set(ptr, out);
            // store (ptr, count) at (dst_base+offset, dst_base+offset+4)
            get(dst_base, out);
            get(ptr, out);
            store_i32_at(offset, out);
            get(dst_base, out);
            get(count, out);
            store_i32_at(offset + 4, out);
            // copy loop: i=0; while i<count: store8(ptr+i, bytes-get(handle,i)); i++
            const_i32(0, out);
            set(i, out);
            out.push(op::BLOCK);
            out.push(wasm_abi::BLOCK_EMPTY);
            out.push(op::LOOP);
            out.push(wasm_abi::BLOCK_EMPTY);
            get(i, out);
            get(count, out);
            out.push(op::I32_GE_U);
            out.push(op::BR_IF);
            uleb128(1, out);
            // store8(ptr + i, bytes-get(handle, i))
            get(ptr, out);
            get(i, out);
            out.push(op::I32_ADD);
            get(handle, out);
            get(i, out);
            call("bytes-get", out);
            out.push(op::I32_STORE8);
            out.push(0x00);
            uleb128(0, out);
            get(i, out);
            const_i32(1, out);
            out.push(op::I32_ADD);
            set(i, out);
            out.push(op::BR);
            uleb128(0, out);
            out.push(op::END); // loop
            out.push(op::END); // block
        }
        CanonWrite::List {
            elem_size,
            elem_align,
            elem,
        } => {
            // count = vec-len(handle); base = cabi_realloc(0,0,elem_align, count*elem_size);
            let count = *next_local;
            let base = *next_local + 1;
            let i = *next_local + 2;
            let eh = *next_local + 3;
            let edst = *next_local + 4;
            *next_local += 5;
            get(handle, out);
            call("vec-len", out);
            set(count, out);
            const_i32(0, out);
            const_i32(0, out);
            const_i32(*elem_align as i64, out);
            get(count, out);
            const_i32(*elem_size as i64, out);
            out.push(op::I32_MUL);
            out.push(op::CALL);
            uleb128(realloc_abs, out);
            set(base, out);
            // store (base, count) at (dst_base+offset, +4)
            get(dst_base, out);
            get(base, out);
            store_i32_at(offset, out);
            get(dst_base, out);
            get(count, out);
            store_i32_at(offset + 4, out);
            // loop i in 0..count: eh = vec-get(handle,i); edst = base + i*elem_size; write elem at (edst,0)
            const_i32(0, out);
            set(i, out);
            out.push(op::BLOCK);
            out.push(wasm_abi::BLOCK_EMPTY);
            out.push(op::LOOP);
            out.push(wasm_abi::BLOCK_EMPTY);
            get(i, out);
            get(count, out);
            out.push(op::I32_GE_U);
            out.push(op::BR_IF);
            uleb128(1, out);
            get(handle, out);
            get(i, out);
            call("vec-get", out);
            set(eh, out);
            get(base, out);
            get(i, out);
            const_i32(*elem_size as i64, out);
            out.push(op::I32_MUL);
            out.push(op::I32_ADD);
            set(edst, out);
            emit_canon_write(
                elem,
                eh,
                edst,
                0,
                next_local,
                realloc_abs,
                bulk_bytes,
                imp,
                out,
            );
            get(i, out);
            const_i32(1, out);
            out.push(op::I32_ADD);
            set(i, out);
            out.push(op::BR);
            uleb128(0, out);
            out.push(op::END); // loop
            out.push(op::END); // block
        }
        CanonWrite::Variant {
            disc_store,
            payload_offset,
            arms,
        } => {
            // d = sum-disc(handle) — the guest's DECL disc.
            let d = *next_local;
            *next_local += 1;
            get(handle, out);
            call("sum-disc", out);
            set(d, out);
            // Per arm k: if d == k, store its boundary disc at dst_base+offset, then (if payload) write it.
            for (k, arm) in arms.iter().enumerate() {
                get(d, out);
                const_i32(k as i64, out);
                out.push(op::I32_EQ);
                out.push(op::IF);
                out.push(wasm_abi::BLOCK_EMPTY);
                // store boundary disc
                get(dst_base, out);
                const_i32(arm.boundary_disc as i64, out);
                out.push(*disc_store);
                out.push(0x00); // align hint
                uleb128(offset as u64, out);
                if let Some(pw) = &arm.payload {
                    let ph = *next_local;
                    *next_local += 1;
                    get(handle, out);
                    call("sum-payload", out);
                    set(ph, out);
                    emit_canon_write(
                        pw,
                        ph,
                        dst_base,
                        offset + payload_offset,
                        next_local,
                        realloc_abs,
                        bulk_bytes,
                        imp,
                        out,
                    );
                }
                out.push(op::END); // if
            }
        }
    }
}

/// Push a closure `call`'s arguments onto the stack in the lifted body's order, threading ZERO OR MORE
/// fixed-shape tuple-arg REBUILDS among the scalar core params. `tuples` are in ascending `base_param` order
/// (each rebuild carries the core-param index its flattened leaves start at); the flattened core params run
/// `1..1+arity` (after `self`=0). Walks the core-param range: a param that starts a tuple's leaves emits that
/// tuple's rebuilt cell (via [`emit_tuple_rebuild`], stashed at `tuple_local + tuple_index` for the
/// post-dispatch drop) and skips its leaves; any other param is a plain scalar `local.get`. With `tuples`
/// empty, byte-identical to the raw scalar push; with one tuple, byte-identical to the prior single-tuple
/// interleave. Shared by the scalar `call` body + every list-result `call` body.
fn emit_closure_call_args(
    tuples: &[TupleArgRebuild],
    tuple_local: u32,
    arity: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    emit_closure_call_args_with_sums(tuples, tuple_local, &[], 0, arity, imp, out)
}

/// [`emit_closure_call_args`] with ZERO OR MORE fixed-shape SUM-arg rebuilds interleaved among the scalars +
/// tuples. Each sum consumes `1` (nullary payload variant) or `2` (disc + one scalar payload) flattened core
/// params from its `base_param`; at a sum's `base_param` the walk emits its rebuilt cell (stashed at
/// `sum_local + i`) and skips its params. Sums + tuples are non-overlapping. With `sums` empty, byte-identical
/// to [`emit_closure_call_args`]. (This increment wires only sums; a tuple + sum together is a later widening.)
fn emit_closure_call_args_with_sums(
    tuples: &[TupleArgRebuild],
    tuple_local: u32,
    sums: &[SumArgRebuild],
    sum_local: u32,
    arity: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    // Walk the flattened core params `1..1+arity`; at each tuple's/sum's `base_param` emit its rebuild + skip
    // its params, else push the scalar. `tuples`/`sums` are ascending by `base_param` + non-overlapping.
    let mut a = 1u32;
    while a < 1 + arity {
        if let Some((ti, rebuild)) = tuples.iter().enumerate().find(|(_, t)| t.base_param == a) {
            emit_tuple_rebuild(rebuild, tuple_local + ti as u32, imp, out);
            a += rebuild
                .fields
                .iter()
                .map(FieldRebuild::leaf_count)
                .sum::<u32>();
        } else if let Some((si, rebuild)) = sums.iter().enumerate().find(|(_, s)| s.base_param == a)
        {
            emit_sum_arg_rebuild(rebuild, sum_local + si as u32, imp, out);
            // Skip the sum's flattened params: disc (1) + the payload's leaves. A scalar `option`/`result`
            // flattens to `(disc, payload)` = 2; a COMPOUND (Option-of-tuple) payload spans `1 + its leaves`.
            a += rebuild.flattened_param_count();
        } else {
            get(a, out);
            a += 1;
        }
    }
}

/// Drop the REBUILT tuple-arg cell (an owned per-call temporary the `call` fabricated) after `call_indirect`.
/// Unconditional (both own + borrow — the host owns only the closure handle, never this fabricated arg cell),
/// balancing the `arr-alloc` in [`emit_tuple_rebuild`]. Leaves the stack unchanged (drop returns nothing).
fn emit_tuple_rebuilt_drop(tuple_local: u32, imp: &dyn Fn(&str) -> u64, out: &mut Vec<u8>) {
    use crate::backend::wasm::wasm_abi::op;
    out.push(op::LOCAL_GET);
    uleb128(tuple_local as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
}

/// Single-export closure-resource core module — the N=1 case of [`multi_closure_resource_core_module`],
/// preserved for the single-closure-export path (`emit_closure_resource`) and its serializer unit test.
#[allow(clippy::too_many_arguments)]
pub fn closure_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    ret_vt: ValType,
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    closure_resource_core_module_borrow(
        funcs,
        imports,
        export_abs,
        arg_vts,
        ret_vt,
        make_param_vts,
        lifted_type_idx,
        layout,
        false,
    )
}

/// [`closure_resource_core_module`] with a `call_borrow` switch — the single-export front to
/// [`multi_closure_resource_core_module_with_host_borrow`]. `call_borrow = true` gives the REPEATABLE
/// `borrow<t>` `call` (host keeps the handle across calls; `t-dtor` reclaims); `false` the shipped
/// own/self-drop single-use `call`.
#[allow(clippy::too_many_arguments)]
pub fn closure_resource_core_module_borrow(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    ret_vt: ValType,
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    layout: &Layout,
    call_borrow: bool,
) -> Result<Vec<u8>, String> {
    multi_closure_resource_core_module_with_host_borrow(
        funcs,
        imports,
        &[],
        &[ClosureMake {
            export_name: "make".to_string(),
            export_abs,
            param_vts: make_param_vts.to_vec(),
        }],
        &[],
        arg_vts,
        ret_vt,
        lifted_type_idx,
        layout,
        call_borrow,
        &[],
        &[],
    )
}

/// The COMPOUND-RESULT (`Bytes`) closure-resource core module: a closure whose result is a runtime `Bytes`
/// crosses the `call` boundary as `list<u8>` (the raw payload), not a scalar. Structurally the single-export
/// scalar core ([`closure_resource_core_module`]) but `call` returns an `i32` retptr instead of the scalar,
/// and the core carries a MEMORY + `cabi_realloc` (a scalar `call` needs neither) so the canonical ABI can
/// read the `(ptr, len)` return area. `call(self, args…)`:
///   1. recover the cell rep (`resource.rep`), dispatch the lifted closure via `call_indirect` — it returns
///      a runtime `Bytes` HANDLE (i32) on the stack;
///   2. store that handle in a local `bh`, DROP the closure cell (`heap.drop(cell)` — own<t> release, as the
///      scalar `call` does), then run the `bytes-len`/`bytes-get` copy loop writing the payload to `OUT=8`
///      and the `(ptr=OUT, len=n)` return area to `[0..8]`;
///   3. DROP the Bytes handle (`heap.drop(bh)` — the copy is done, the guest owns the transient Bytes) and
///      return the retptr `0`.
///
/// The `oracle_closure_list_component` proved this `call` shape (a `list<u8>`-returning method lifted with
/// Memory/Realloc) runs under wasmtime; this emits the PRODUCTION core. `arg_vts` are the closure's arg
/// core valtypes; the closure body's `Bytes` result is an i32 handle, so `call`'s core result is i32.
/// The imports must include `bytes-len`, `bytes-get`, `drop`, `arr-get`, `get-int` (the caller's used-set).
pub fn closure_bytes_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    closure_bytes_resource_core_module_borrow(
        funcs,
        imports,
        export_abs,
        arg_vts,
        make_param_vts,
        lifted_type_idx,
        layout,
        false,
        &[],
    )
}

/// [`closure_bytes_resource_core_module`] with a `call_borrow` switch (C-HOST-6, byte-rope result). When
/// TRUE the `call` uses the borrow-lift's directly-passed rep as the cell (no `resource.rep`) and does NOT
/// drop the cell (the host keeps the handle → repeatable; the `t-dtor` reclaims on drop). The transient
/// `Bytes` handle the closure returns is STILL dropped after the copy (the guest owns that scratch value
/// either way — it is separate from the cell). `false` reproduces the own/self-drop body byte-for-byte.
///
/// `tuples`: ZERO OR MORE fixed-shape tuple/record args that crossed FLATTENED — the `call` rebuilds each cell
/// from its flattened fields ([`emit_tuple_rebuild`], stashed at `tuple_local + i`) before `call_indirect` and
/// drops each after ([`emit_tuple_rebuilt_drop`]). `arg_vts` is the FULL flattened field vts of every arg.
/// `&[]` = the scalar-arg path (byte-identical); a single rebuild reproduces the one-tuple path byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn closure_bytes_resource_core_module_borrow(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    layout: &Layout,
    call_borrow: bool,
    tuples: &[TupleArgRebuild],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── import functypes 0..k; resource-new/rep (k, k+1); one per defined body; then make
    // `(make-params…)->i32`; call `(i32 self, args…)->i32 retptr`; cabi_realloc `(i32×4)->i32`.
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
    type_items.extend_from_slice(&i32_to_i32); // resource-new (k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    // make `(make-params…)->i32`.
    let make_type_idx = defined_type_base + n;
    {
        let params: Vec<u8> = make_param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // call `(i32 self, args…)->i32 retptr`.
    let call_type_idx = make_type_idx + 1;
    {
        let mut params = vec![wasm_abi::CORE_I32];
        params.extend(arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // cabi_realloc `(i32×4)->i32`.
    let realloc_type_idx = call_type_idx + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + 3;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── k ops + resource-new + resource-rep.
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

    // ── Function section ── defined bodies, then make, call, cabi_realloc.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    uleb128(make_type_idx as u64, &mut func_items);
    uleb128(call_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(n + 3, &func_items));
    let make_abs = (defined_type_base + n) as u32;
    let call_abs = make_abs + 1;
    let realloc_abs = call_abs + 1;

    // ── Table + Element ── the funcref table for the lifted closure(s). ── Memory ── one page.
    let n_lifted = layout.lifted.len();
    let mut table_entry = vec![0x70u8, 0x01];
    uleb128(n_lifted as u64, &mut table_entry);
    uleb128(n_lifted as u64, &mut table_entry);
    let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Export section ── memory, make, call, cabi_realloc (core section order: table(4) precedes mem(5)).
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
        items.extend_from_slice(&export("call", wasm_abi::EXPORT_KIND_FUNC, call_abs));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(4, &items))
    };

    // ── Element ── active segment table 0, offset 0, [lifted…].
    let elem_sec = {
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg))
    };

    // ── Code section ── defined bodies, then make, call, cabi_realloc.
    let imp = |name: &str| {
        *import_index
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` imported")) as u64
    };
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // make: forward the export params, `call <export body>` (builds the cell), `resource.new`.
    {
        let mut inner = uleb_bytes(0);
        for p in 0..make_param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // call(self, args…): dispatch the lifted closure → a Bytes HANDLE, copy it to the return area, drop
    // both the cell and the Bytes handle, return the retptr.
    {
        const OUT: i64 = 8;
        // Params: 0 = self, 1..1+arity = args. Locals: cell(i32), bh(i32 bytes handle), n(i32), i(i32), and
        // (for each flattened tuple arg) tuple(i32 = the rebuilt arg cell, at tuple_local + i).
        let arity = arg_vts.len() as u32;
        let cell = 1 + arity;
        let bh = cell + 1;
        let nlen = bh + 1;
        let iv = nlen + 1;
        let tuple_local = iv + 1;
        let n_locals = 4 + tuples.len() as u32; // cell/bh/n/i + one i32 per rebuilt tuple-arg cell
        let mut inner = Vec::new();
        // one local group: n_locals × i32.
        inner.extend_from_slice(&wasm_vec(1, &{
            let mut g = uleb_bytes(n_locals as u64);
            g.push(wasm_abi::CORE_I32);
            g
        }));
        let get = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_GET);
            uleb128(l as u64, out);
        };
        let set = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_SET);
            uleb128(l as u64, out);
        };
        let ci32 = |v: i64, out: &mut Vec<u8>| {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(v, out);
        };
        // cell = self (borrow: the rep passed directly) or resource.rep(self) (own).
        get(0, &mut inner);
        if !call_borrow {
            inner.push(op::CALL);
            uleb128(f_rrep as u64, &mut inner);
        }
        set(cell, &mut inner);
        // dispatch: push env(cell) + args (or the REBUILT tuple-arg cell), read the code slot
        // (arr-get(cell,0)→get-int→wrap), call_indirect.
        get(cell, &mut inner);
        emit_closure_call_args(tuples, tuple_local, arity, &imp, &mut inner);
        get(cell, &mut inner);
        ci32(0, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("arr-get"), &mut inner);
        inner.push(op::CALL);
        uleb128(imp("get-int"), &mut inner);
        inner.push(op::I32_WRAP_I64);
        inner.push(op::CALL_INDIRECT);
        uleb128(lifted_type_idx as u64, &mut inner);
        uleb128(0, &mut inner); // table 0
        set(bh, &mut inner); // the closure's Bytes-handle result
        // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (the lifted body finished
        // borrowing it), unconditionally (own + borrow). Separate from the closure cell + the Bytes handle.
        for ti in 0..tuples.len() as u32 {
            emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
        }
        // OWN: drop the closure cell now (release — balances make's alloc). BORROW: the host keeps the cell
        // (repeatable), the `t-dtor` reclaims on drop — do NOT drop here. The transient Bytes handle `bh` is
        // separate and is dropped after the copy either way.
        if !call_borrow {
            get(cell, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
        }
        // n = bytes-len(bh)
        get(bh, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-len"), &mut inner);
        set(nlen, &mut inner);
        // GROW to cover [OUT, OUT+n) before the copy — a >64-KiB Bytes closure result would otherwise
        // write OOB past the single initial page (the #7793 page-boundary class, on the closure escape
        // copy-out). No-op when memory already suffices.
        emit_grow_to_cover_out(nlen, OUT, &mut inner);
        // copy loop: i=0; block{ loop{ if i>=n br 1; store8(OUT+i, bytes-get(bh,i)); i++; br 0 } }
        ci32(0, &mut inner);
        set(iv, &mut inner);
        inner.push(op::BLOCK);
        inner.push(wasm_abi::BLOCK_EMPTY);
        inner.push(op::LOOP);
        inner.push(wasm_abi::BLOCK_EMPTY);
        get(iv, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_GE_U);
        inner.push(op::BR_IF);
        uleb128(1, &mut inner);
        ci32(OUT, &mut inner);
        get(iv, &mut inner);
        inner.push(op::I32_ADD);
        get(bh, &mut inner);
        get(iv, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-get"), &mut inner);
        inner.push(op::I32_STORE8);
        inner.push(0x00);
        inner.push(0x00);
        get(iv, &mut inner);
        ci32(1, &mut inner);
        inner.push(op::I32_ADD);
        set(iv, &mut inner);
        inner.push(op::BR);
        uleb128(0, &mut inner);
        inner.push(op::END); // loop
        inner.push(op::END); // block
        // retarea [0..8]: ptr = OUT, len = n.
        ci32(0, &mut inner);
        ci32(OUT, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        ci32(4, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        // DROP the Bytes handle (copy done — the guest owns this transient result).
        get(bh, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        ci32(0, &mut inner); // return the retarea pointer (0)
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // cabi_realloc stub (never called for a fixed-size return area).
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
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// The single-export COMPOUND-VALUE-result closure core module: the closure's `call` returns a
/// tuple/record/sum whose CANONICAL VALUE FORM crosses as `list<u8>` (the host decodes + pretty-prints the
/// `(: value T)` document). Structurally [`closure_bytes_resource_core_module`], but the `call` body writes
/// the value-form TEMPLATE into linear memory + walks each runtime leaf hole from the closure's returned
/// heap handle (the escape's [`encode_walk_body`] machinery), instead of the raw `bytes-*` copy loop. The
/// template's static bytes (structure, names, type node, leaf framing) are laid in the DATA section at
/// `byte_off`; its `(ptr,len)` return area at `ret_off`; only the leaf VALUES are filled at run time by
/// walking `arr-get`/`get-int` paths from the dispatched compound handle. The closure's compound result is
/// a plain heap rep (the `call_indirect` result), so `emit_hole_fill` walks it exactly as the escape walks
/// a resource rep — the ONLY difference is the rep is a local (the dispatch result), not `resource.rep`'d.
#[allow(clippy::too_many_arguments)]
pub fn closure_value_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    template: &crate::lower::ValueFormTemplate,
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    closure_value_resource_core_module_borrow(
        funcs,
        imports,
        export_abs,
        arg_vts,
        make_param_vts,
        lifted_type_idx,
        template,
        layout,
        false,
        &[],
    )
}

/// [`closure_value_resource_core_module`] with a `call_borrow` switch (C-HOST-6, compound value-form
/// result). When TRUE the `call` uses the borrow-lift's rep directly (no `resource.rep`) and does NOT drop
/// the cell (repeatable; the `t-dtor` reclaims). The transient COMPOUND handle the closure returns is still
/// dropped after the walk (guest-owned scratch, separate from the cell). `false` = own/self-drop, byte-identical.
///
/// `tuples`: ZERO OR MORE fixed-shape tuple/record args that crossed FLATTENED — the `call` rebuilds each cell
/// from its flattened fields ([`emit_tuple_rebuild`], at `tuple_local + i`) before `call_indirect` and drops
/// each after ([`emit_tuple_rebuilt_drop`]). `arg_vts` is the FULL flattened field vts of every arg. `&[]` =
/// the scalar-arg path (byte-identical); a single rebuild reproduces the one-tuple path byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn closure_value_resource_core_module_borrow(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    template: &crate::lower::ValueFormTemplate,
    layout: &Layout,
    call_borrow: bool,
    tuples: &[TupleArgRebuild],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── identical shape to the bytes core: imports 0..k; resource-new/rep; defined bodies;
    // make `(make-params…)->i32`; call `(i32 self, args…)->i32 retptr`; cabi_realloc `(i32×4)->i32`.
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
    type_items.extend_from_slice(&i32_to_i32); // resource-new (k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let make_type_idx = defined_type_base + n;
    {
        let params: Vec<u8> = make_param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let call_type_idx = make_type_idx + 1;
    {
        let mut params = vec![wasm_abi::CORE_I32];
        params.extend(arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let realloc_type_idx = call_type_idx + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + 3;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ──
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

    // ── Function section ──
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    uleb128(make_type_idx as u64, &mut func_items);
    uleb128(call_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(n + 3, &func_items));
    let make_abs = (defined_type_base + n) as u32;
    let call_abs = make_abs + 1;
    let realloc_abs = call_abs + 1;

    // ── Table + Memory ──
    let n_lifted = layout.lifted.len();
    let mut table_entry = vec![0x70u8, 0x01];
    uleb128(n_lifted as u64, &mut table_entry);
    uleb128(n_lifted as u64, &mut table_entry);
    let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Export section ── memory, make, call, cabi_realloc.
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
        items.extend_from_slice(&export("call", wasm_abi::EXPORT_KIND_FUNC, call_abs));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(4, &items))
    };

    // ── Element ──
    let elem_sec = {
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg))
    };

    // ── Data section ── the value-form template bytes at `byte_off`, its `(ptr,len)` return area at
    // `ret_off` (both 4-aligned, mirroring the escape's runtime resource layout). The template holes are
    // filled at run time by the `call` body walking the dispatched compound handle.
    let byte_off = 0usize;
    let mut data_bytes: Vec<u8> = template.bytes.clone();
    let ret_off = (data_bytes.len() + 3) & !3;
    data_bytes.resize(ret_off, 0);
    data_bytes.extend_from_slice(&(byte_off as u32).to_le_bytes());
    data_bytes.extend_from_slice(&(template.bytes.len() as u32).to_le_bytes());
    let data_sec = {
        let mut item = vec![0x00];
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut item);
        item.push(op::END);
        item.extend_from_slice(&uleb_bytes(data_bytes.len() as u64));
        item.extend_from_slice(&data_bytes);
        section(wasm_abi::CORE_SEC_DATA, &wasm_vec(1, &item))
    };

    // ── Code section ── defined bodies, then make, call, cabi_realloc.
    let imp = |name: &str| {
        *import_index
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` imported")) as u64
    };
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // make: forward the export params, `call <export body>` (builds the cell), `resource.new`.
    {
        let mut inner = uleb_bytes(0);
        for p in 0..make_param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // call(self, args…): dispatch the lifted closure → a COMPOUND heap handle, drop the cell, then walk the
    // handle to fill the value-form template holes in memory, drop the handle, return the retptr.
    {
        // Params: 0 = self, 1..1+arity = args. Locals — the i32 group FIRST (cell, rep, and one rebuilt-arg
        // cell per flattened tuple arg at `tuple_local + i`), THEN the i64 `scratch`. So `scratch`'s index
        // depends on how many i32 locals precede it: cell=1+arity, rep=cell+1, [tuples…], scratch=(1+arity)+n_i32.
        let arity = arg_vts.len() as u32;
        let cell = 1 + arity;
        let rep = cell + 1;
        let n_i32: u32 = 2 + tuples.len() as u32; // cell, rep, + one i32 per rebuilt tuple-arg cell
        let tuple_local = rep + 1; // the first rebuilt tuple cell (only valid when !tuples.is_empty())
        let scratch = cell + n_i32; // the i64, after all i32 locals
        let mut inner = Vec::new();
        // two local groups: n_i32 × i32 (cell, rep, [tuple]) then 1 × i64 (scratch).
        inner.extend_from_slice(&wasm_vec(2, &{
            let mut g = uleb_bytes(n_i32 as u64);
            g.push(wasm_abi::CORE_I32);
            let mut g2 = uleb_bytes(1);
            g2.push(wasm_abi::CORE_I64);
            g.extend_from_slice(&g2);
            g
        }));
        let get = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_GET);
            uleb128(l as u64, out);
        };
        let set = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_SET);
            uleb128(l as u64, out);
        };
        // cell = self (borrow: rep passed directly) or resource.rep(self) (own).
        get(0, &mut inner);
        if !call_borrow {
            inner.push(op::CALL);
            uleb128(f_rrep as u64, &mut inner);
        }
        set(cell, &mut inner);
        // dispatch: push env(cell) + args (or the REBUILT tuple-arg cell), read the code slot, call_indirect →
        // the compound handle.
        get(cell, &mut inner);
        emit_closure_call_args(tuples, tuple_local, arity, &imp, &mut inner);
        get(cell, &mut inner);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("arr-get"), &mut inner);
        inner.push(op::CALL);
        uleb128(imp("get-int"), &mut inner);
        inner.push(op::I32_WRAP_I64);
        inner.push(op::CALL_INDIRECT);
        uleb128(lifted_type_idx as u64, &mut inner);
        uleb128(0, &mut inner); // table 0
        set(rep, &mut inner); // the closure's compound-handle result
        // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally), before
        // walking the result. Separate from the closure cell + the compound result handle.
        for ti in 0..tuples.len() as u32 {
            emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
        }
        // OWN: drop the closure cell now (release). BORROW: host keeps the cell (repeatable), dtor reclaims —
        // do NOT drop here. The transient compound handle `rep` is separate and dropped after the walk.
        if !call_borrow {
            get(cell, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
        }
        // Fill each template hole by walking the compound handle (`rep`). `emit_hole_fill` reads
        // `arr-get`/`get-int` from `rep` and stores each leaf's bytes into the template at `byte_off`.
        for hole in &template.leaves {
            emit_hole_fill(hole, byte_off, rep, scratch, &import_index, &mut inner);
        }
        // DROP the compound handle (the walk is done — the guest owns this transient result).
        get(rep, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        // return the (ptr,len) retarea pointer.
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(ret_off as i64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // cabi_realloc stub.
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
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    // Core section order: elem(9), code(10), data(11) — the data section comes AFTER code.
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    Ok(core)
}

/// The single-export VARIABLE-LENGTH-collection-result closure core module: the closure's `call` returns a
/// `List`/`Map`/`Set` whose canonical VALUE FORM crosses as `list<u8>`, rendered by the runtime
/// `value-encode(rep, desc)` op (the recursive-sum escape's "approach C") instead of a fixed template — a
/// variable-length collection has no static template. Structurally [`closure_value_resource_core_module`],
/// but the `call` body: dispatch the lifted closure → the collection HANDLE (`rep`), drop the cell, build
/// the compiler-baked shape `descriptor` as a heap `Bytes` (`bytes-alloc` + literal `bytes-set`s), call
/// `value-encode(rep, desc)` → a Bytes document, copy that document out as the `(ptr,len)` return area, and
/// release `rep`/`desc`/`doc`. No data section (the descriptor bytes are baked into the code as constants).
/// The imports must include `value-encode`/`bytes-alloc`/`bytes-set`/`bytes-len`/`bytes-get`/`drop`/
/// `arr-get`/`get-int`.
#[allow(clippy::too_many_arguments)]
pub fn closure_value_encode_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    descriptor: &[u8],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    closure_value_encode_resource_core_module_borrow(
        funcs,
        imports,
        export_abs,
        arg_vts,
        make_param_vts,
        lifted_type_idx,
        descriptor,
        layout,
        false,
        &[],
    )
}

/// [`closure_value_encode_resource_core_module`] with a `call_borrow` switch (C-HOST-6, collection
/// value-encode result). When TRUE the `call` uses the borrow-lift's rep directly (no `resource.rep`) and
/// does NOT drop the cell (repeatable; the `t-dtor` reclaims). The transient collection handle `rep` (and
/// `desc`/`doc`) are still released after the value-encode — guest-owned scratch, separate from the cell.
/// `false` = own/self-drop, byte-identical.
///
/// `tuples`: ZERO OR MORE fixed-shape tuple/record args that crossed FLATTENED — the `call` rebuilds each cell
/// from its flattened fields ([`emit_tuple_rebuild`], at `tuple_local + i`) before `call_indirect` and drops
/// each after ([`emit_tuple_rebuilt_drop`]). `arg_vts` is the FULL flattened field vts of every arg. `&[]` =
/// the scalar-arg path (byte-identical); a single rebuild reproduces the one-tuple path byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn closure_value_encode_resource_core_module_borrow(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    export_abs: u32,
    arg_vts: &[ValType],
    make_param_vts: &[ValType],
    lifted_type_idx: u32,
    descriptor: &[u8],
    layout: &Layout,
    call_borrow: bool,
    tuples: &[TupleArgRebuild],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── identical shape to the bytes/value core: imports 0..k; resource-new/rep; defined
    // bodies; make `(make-params…)->i32`; call `(i32 self, args…)->i32 retptr`; cabi_realloc `(i32×4)->i32`.
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
    type_items.extend_from_slice(&i32_to_i32); // resource-new (k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let make_type_idx = defined_type_base + n;
    {
        let params: Vec<u8> = make_param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let call_type_idx = make_type_idx + 1;
    {
        let mut params = vec![wasm_abi::CORE_I32];
        params.extend(arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let realloc_type_idx = call_type_idx + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + 3;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ──
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

    // ── Function section ──
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    uleb128(make_type_idx as u64, &mut func_items);
    uleb128(call_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    let func_sec = section(wasm_abi::CORE_SEC_FUNCTION, &wasm_vec(n + 3, &func_items));
    let make_abs = (defined_type_base + n) as u32;
    let call_abs = make_abs + 1;
    let realloc_abs = call_abs + 1;

    // ── Table + Memory ──
    let n_lifted = layout.lifted.len();
    let mut table_entry = vec![0x70u8, 0x01];
    uleb128(n_lifted as u64, &mut table_entry);
    uleb128(n_lifted as u64, &mut table_entry);
    let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Export section ── memory, make, call, cabi_realloc.
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
        items.extend_from_slice(&export("call", wasm_abi::EXPORT_KIND_FUNC, call_abs));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        section(wasm_abi::CORE_SEC_EXPORT, &wasm_vec(4, &items))
    };

    // ── Element ──
    let elem_sec = {
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg))
    };

    // ── Code section ── defined bodies, then make, call, cabi_realloc.
    let imp = |name: &str| {
        *import_index
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` imported")) as u64
    };
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // make: forward the export params, `call <export body>` (builds the cell), `resource.new`.
    {
        let mut inner = uleb_bytes(0);
        for p in 0..make_param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // call(self, args…): dispatch → the collection HANDLE, drop the cell, build the descriptor Bytes,
    // value-encode(rep, desc) → the document, copy it to the retarea, drop rep/desc/doc, return the retptr.
    {
        const OUT: i64 = 8;
        // Params: 0 = self, 1..1+arity = args. Locals: cell, rep, desc, doc, n, i — 6 × i32; plus one i32
        // per flattened tuple arg (the rebuilt arg cell, at tuple_local + i).
        let arity = arg_vts.len() as u32;
        let cell = 1 + arity;
        let rep = cell + 1;
        let desc = rep + 1;
        let doc = desc + 1;
        let nlen = doc + 1;
        let iv = nlen + 1;
        let tuple_local = iv + 1;
        let n_locals = 6 + tuples.len() as u32; // cell/rep/desc/doc/n/i + one i32 per rebuilt tuple-arg cell
        let mut inner = Vec::new();
        inner.extend_from_slice(&wasm_vec(1, &{
            let mut g = uleb_bytes(n_locals as u64);
            g.push(wasm_abi::CORE_I32);
            g
        }));
        let get = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_GET);
            uleb128(l as u64, out);
        };
        let set = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_SET);
            uleb128(l as u64, out);
        };
        let ci32 = |v: i64, out: &mut Vec<u8>| {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(v, out);
        };
        // cell = self (borrow: rep passed directly) or resource.rep(self) (own); dispatch → the collection
        // handle into `rep`.
        get(0, &mut inner);
        if !call_borrow {
            inner.push(op::CALL);
            uleb128(f_rrep as u64, &mut inner);
        }
        set(cell, &mut inner);
        get(cell, &mut inner);
        emit_closure_call_args(tuples, tuple_local, arity, &imp, &mut inner);
        get(cell, &mut inner);
        ci32(0, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("arr-get"), &mut inner);
        inner.push(op::CALL);
        uleb128(imp("get-int"), &mut inner);
        inner.push(op::I32_WRAP_I64);
        inner.push(op::CALL_INDIRECT);
        uleb128(lifted_type_idx as u64, &mut inner);
        uleb128(0, &mut inner);
        set(rep, &mut inner);
        // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally), before
        // the value-encode of the result. Separate from the closure cell + the collection result handle.
        for ti in 0..tuples.len() as u32 {
            emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
        }
        // OWN: drop the closure cell now. BORROW: host keeps the cell (repeatable), dtor reclaims — do NOT
        // drop here. The transient collection handle `rep` is separate and released after value-encode.
        if !call_borrow {
            get(cell, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
        }
        // desc = bytes-alloc(len); bytes-set each constant descriptor byte.
        ci32(descriptor.len() as i64, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-alloc"), &mut inner);
        set(desc, &mut inner);
        for (j, &byte) in descriptor.iter().enumerate() {
            get(desc, &mut inner);
            ci32(j as i64, &mut inner);
            ci32(byte as i64, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-set"), &mut inner);
            set(desc, &mut inner);
        }
        // doc = value-encode(rep, desc); n = bytes-len(doc).
        get(rep, &mut inner);
        get(desc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("value-encode"), &mut inner);
        set(doc, &mut inner);
        get(doc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-len"), &mut inner);
        set(nlen, &mut inner);
        // GROW to cover [OUT, OUT+n) before the copy — a >64-KiB value-form document would otherwise write
        // OOB past the single initial page (the #7793 page-boundary class, on the closure value-encode
        // escape copy-out). No-op when memory already suffices.
        emit_grow_to_cover_out(nlen, OUT, &mut inner);
        // copy loop: for i in 0..n { store8(OUT+i, bytes-get(doc, i)) }.
        ci32(0, &mut inner);
        set(iv, &mut inner);
        inner.push(op::BLOCK);
        inner.push(wasm_abi::BLOCK_EMPTY);
        inner.push(op::LOOP);
        inner.push(wasm_abi::BLOCK_EMPTY);
        get(iv, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_GE_U);
        inner.push(op::BR_IF);
        uleb128(1, &mut inner);
        ci32(OUT, &mut inner);
        get(iv, &mut inner);
        inner.push(op::I32_ADD);
        get(doc, &mut inner);
        get(iv, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-get"), &mut inner);
        inner.push(op::I32_STORE8);
        inner.push(0x00);
        inner.push(0x00);
        get(iv, &mut inner);
        ci32(1, &mut inner);
        inner.push(op::I32_ADD);
        set(iv, &mut inner);
        inner.push(op::BR);
        uleb128(0, &mut inner);
        inner.push(op::END);
        inner.push(op::END);
        // retarea [0..8]: ptr = OUT, len = n.
        ci32(0, &mut inner);
        ci32(OUT, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        ci32(4, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        // release rep, desc, doc.
        get(rep, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        get(desc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        get(doc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        ci32(0, &mut inner); // return the retptr (0)
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // cabi_realloc stub.
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
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// The MULTI-EXPORT BYTE-ROPE-result closure core module: N `make-<name>` functions sharing ONE `call`
/// that returns `list<u8>` (a `Bytes`/`String` closure result). Combines [`multi_closure_resource_core_module`]
/// (N makes + shared `call`) with [`closure_bytes_resource_core_module`] (the memory + `cabi_realloc` + the
/// `bytes-len`/`bytes-get` copy loop). Every export shares the closure SIGNATURE, so ONE shared `call`
/// dispatches whichever closure a handle names (the code slot travels in the rep), then copies the returned
/// Bytes handle to the `(ptr, len)` return area. Func/type layout mirrors the scalar multi-export core with
/// `call`'s result changed to i32-retptr + a trailing `cabi_realloc`. `arg_vts` are the closure's arg core
/// valtypes; the imports must include `bytes-len`/`bytes-get`/`drop`/`arr-get`/`get-int`.
#[allow(clippy::too_many_arguments)]
pub fn multi_closure_bytes_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    makes: &[ClosureMake],
    plain: &[PlainExport],
    arg_vts: &[ValType],
    lifted_type_idx: u32,
    layout: &Layout,
    call_borrow: bool,
    tuples: &[TupleArgRebuild],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let nmk = makes.len();
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── import functypes 0..k; resource-new/rep (k, k+1); one per defined body; then N make
    // functypes `(make-params…)->i32`; call `(i32 self, args…)->i32 retptr`; cabi_realloc `(i32×4)->i32`.
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
    type_items.extend_from_slice(&i32_to_i32); // resource-new (k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let make_type_base = defined_type_base + n;
    for mk in makes {
        let params: Vec<u8> = mk.param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // call `(i32 self, args…)->i32 retptr` — shared across all makes.
    let call_type_idx = make_type_base + nmk;
    {
        let mut params = vec![wasm_abi::CORE_I32];
        params.extend(arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let realloc_type_idx = call_type_idx + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + nmk + 2;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ──
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

    // ── Function section ── defined bodies, then N makes, then call, then cabi_realloc.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    for i in 0..nmk {
        uleb128((make_type_base + i) as u64, &mut func_items);
    }
    uleb128(call_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + nmk + 2, &func_items),
    );
    let make_abs_base = (defined_type_base + n) as u32;
    let call_abs = make_abs_base + nmk as u32;
    let realloc_abs = call_abs + 1;

    // ── Table + Memory ──
    let n_lifted = layout.lifted.len();
    let mut table_entry = vec![0x70u8, 0x01];
    uleb128(n_lifted as u64, &mut table_entry);
    uleb128(n_lifted as u64, &mut table_entry);
    let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Export section ── memory, N make-<name>, call, cabi_realloc.
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
        for (i, mk) in makes.iter().enumerate() {
            items.extend_from_slice(&export(
                &mk.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                make_abs_base + i as u32,
            ));
        }
        items.extend_from_slice(&export("call", wasm_abi::EXPORT_KIND_FUNC, call_abs));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        // PLAIN (non-closure) exports ride along: their bodies are already defined funcs, exported by index.
        for p in plain {
            items.extend_from_slice(&export(
                &p.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                p.body_abs,
            ));
        }
        section(
            wasm_abi::CORE_SEC_EXPORT,
            &wasm_vec(nmk + 3 + plain.len(), &items),
        )
    };

    // ── Element ──
    let elem_sec = {
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg))
    };

    // ── Code section ── defined bodies, then N makes, then call, then cabi_realloc.
    let imp = |name: &str| {
        *import_index
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` imported")) as u64
    };
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // make[i]: forward params, `call <export body>`, `resource.new`.
    for mk in makes {
        let mut inner = uleb_bytes(0);
        for p in 0..mk.param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(mk.export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // The shared bytes-`call` — identical body to the single-export bytes core (the code slot is recovered
    // from the rep, so ONE `call` serves all makes).
    {
        const OUT: i64 = 8;
        let arity = arg_vts.len() as u32;
        let cell = 1 + arity;
        let bh = cell + 1;
        let nlen = bh + 1;
        let iv = nlen + 1;
        let tuple_local = iv + 1;
        let n_locals = 4 + tuples.len() as u32; // cell/bh/n/i + one i32 per rebuilt tuple-arg cell
        let mut inner = Vec::new();
        inner.extend_from_slice(&wasm_vec(1, &{
            let mut g = uleb_bytes(n_locals as u64);
            g.push(wasm_abi::CORE_I32);
            g
        }));
        let get = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_GET);
            uleb128(l as u64, out);
        };
        let set = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_SET);
            uleb128(l as u64, out);
        };
        let ci32 = |v: i64, out: &mut Vec<u8>| {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(v, out);
        };
        // cell = self (borrow: rep passed directly) or resource.rep(self) (own).
        get(0, &mut inner);
        if !call_borrow {
            inner.push(op::CALL);
            uleb128(f_rrep as u64, &mut inner);
        }
        set(cell, &mut inner);
        get(cell, &mut inner);
        emit_closure_call_args(tuples, tuple_local, arity, &imp, &mut inner);
        get(cell, &mut inner);
        ci32(0, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("arr-get"), &mut inner);
        inner.push(op::CALL);
        uleb128(imp("get-int"), &mut inner);
        inner.push(op::I32_WRAP_I64);
        inner.push(op::CALL_INDIRECT);
        uleb128(lifted_type_idx as u64, &mut inner);
        uleb128(0, &mut inner);
        set(bh, &mut inner);
        // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally). Separate
        // from the closure cell + the transient Bytes handle.
        for ti in 0..tuples.len() as u32 {
            emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
        }
        // OWN: drop the cell now. BORROW: host keeps it (repeatable), dtor reclaims — do NOT drop. The
        // transient Bytes handle `bh` is separate and dropped after the copy either way.
        if !call_borrow {
            get(cell, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
        }
        get(bh, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-len"), &mut inner);
        set(nlen, &mut inner);
        ci32(0, &mut inner);
        set(iv, &mut inner);
        inner.push(op::BLOCK);
        inner.push(wasm_abi::BLOCK_EMPTY);
        inner.push(op::LOOP);
        inner.push(wasm_abi::BLOCK_EMPTY);
        get(iv, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_GE_U);
        inner.push(op::BR_IF);
        uleb128(1, &mut inner);
        ci32(OUT, &mut inner);
        get(iv, &mut inner);
        inner.push(op::I32_ADD);
        get(bh, &mut inner);
        get(iv, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-get"), &mut inner);
        inner.push(op::I32_STORE8);
        inner.push(0x00);
        inner.push(0x00);
        get(iv, &mut inner);
        ci32(1, &mut inner);
        inner.push(op::I32_ADD);
        set(iv, &mut inner);
        inner.push(op::BR);
        uleb128(0, &mut inner);
        inner.push(op::END);
        inner.push(op::END);
        ci32(0, &mut inner);
        ci32(OUT, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        ci32(4, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        get(bh, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        ci32(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // cabi_realloc stub.
    {
        let mut inner = uleb_bytes(0);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(wasm_abi::CORE_SEC_CODE, &wasm_vec(n + nmk + 2, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// The MULTI-EXPORT VARIABLE-LENGTH-collection-result closure core module: N `make-<name>` functions sharing
/// ONE `call` that returns `list<u8>` carrying the canonical VALUE FORM of a List/Map/Set result, rendered
/// at run time by `value-encode(rep, desc)`. Combines [`multi_closure_bytes_resource_core_module`] (N makes +
/// shared list-`call` + memory/cabi_realloc + plain exports) with [`closure_value_encode_resource_core_module`]'s
/// value-encode body (build the descriptor Bytes, encode the returned collection handle, copy the doc out).
/// Every export shares the closure SIGNATURE — hence the SAME result type + the ONE shape `descriptor` — so a
/// single shared `call` dispatches whichever closure a handle names, then value-encodes its collection result.
#[allow(clippy::too_many_arguments)]
pub fn multi_closure_value_encode_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    makes: &[ClosureMake],
    plain: &[PlainExport],
    arg_vts: &[ValType],
    lifted_type_idx: u32,
    descriptor: &[u8],
    layout: &Layout,
    call_borrow: bool,
    tuples: &[TupleArgRebuild],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let nmk = makes.len();
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── imports 0..k; resource-new/rep; defined bodies; N make functypes; shared call
    // `(i32 self, args…)->i32 retptr`; cabi_realloc `(i32×4)->i32`.
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
    type_items.extend_from_slice(&i32_to_i32); // resource-new (k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let make_type_base = defined_type_base + n;
    for mk in makes {
        let params: Vec<u8> = mk.param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let call_type_idx = make_type_base + nmk;
    {
        let mut params = vec![wasm_abi::CORE_I32];
        params.extend(arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let realloc_type_idx = call_type_idx + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + nmk + 2;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ──
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

    // ── Function section ── defined bodies, N makes, call, cabi_realloc.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    for i in 0..nmk {
        uleb128((make_type_base + i) as u64, &mut func_items);
    }
    uleb128(call_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + nmk + 2, &func_items),
    );
    let make_abs_base = (defined_type_base + n) as u32;
    let call_abs = make_abs_base + nmk as u32;
    let realloc_abs = call_abs + 1;

    // ── Table + Memory ──
    let n_lifted = layout.lifted.len();
    let mut table_entry = vec![0x70u8, 0x01];
    uleb128(n_lifted as u64, &mut table_entry);
    uleb128(n_lifted as u64, &mut table_entry);
    let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Export section ── memory, N make-<name>, call, cabi_realloc, then plain exports.
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
        for (i, mk) in makes.iter().enumerate() {
            items.extend_from_slice(&export(
                &mk.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                make_abs_base + i as u32,
            ));
        }
        items.extend_from_slice(&export("call", wasm_abi::EXPORT_KIND_FUNC, call_abs));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        for p in plain {
            items.extend_from_slice(&export(
                &p.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                p.body_abs,
            ));
        }
        section(
            wasm_abi::CORE_SEC_EXPORT,
            &wasm_vec(nmk + 3 + plain.len(), &items),
        )
    };

    // ── Element ──
    let elem_sec = {
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg))
    };

    // ── Code section ── defined bodies, N makes, shared value-encode call, cabi_realloc. No data section
    // (the descriptor bytes are baked into the `call` body as constants).
    let imp = |name: &str| {
        *import_index
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` imported")) as u64
    };
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    for mk in makes {
        let mut inner = uleb_bytes(0);
        for p in 0..mk.param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(mk.export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // The shared value-encode `call` — identical body to the single-export value-encode core: dispatch → the
    // collection handle, drop the cell, build the descriptor Bytes, value-encode(rep, desc) → the document,
    // copy it out, release rep/desc/doc. ONE `call` serves all makes (the descriptor is common, since all
    // exports share the result type).
    {
        const OUT: i64 = 8;
        let arity = arg_vts.len() as u32;
        let cell = 1 + arity;
        let rep = cell + 1;
        let desc = rep + 1;
        let doc = desc + 1;
        let nlen = doc + 1;
        let iv = nlen + 1;
        let tuple_local = iv + 1;
        let n_locals = 6 + tuples.len() as u32; // cell/rep/desc/doc/n/i + one i32 per rebuilt tuple-arg cell
        let mut inner = Vec::new();
        inner.extend_from_slice(&wasm_vec(1, &{
            let mut g = uleb_bytes(n_locals as u64);
            g.push(wasm_abi::CORE_I32);
            g
        }));
        let get = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_GET);
            uleb128(l as u64, out);
        };
        let set = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_SET);
            uleb128(l as u64, out);
        };
        let ci32 = |v: i64, out: &mut Vec<u8>| {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(v, out);
        };
        // cell = self (borrow: rep passed directly) or resource.rep(self) (own).
        get(0, &mut inner);
        if !call_borrow {
            inner.push(op::CALL);
            uleb128(f_rrep as u64, &mut inner);
        }
        set(cell, &mut inner);
        get(cell, &mut inner);
        emit_closure_call_args(tuples, tuple_local, arity, &imp, &mut inner);
        get(cell, &mut inner);
        ci32(0, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("arr-get"), &mut inner);
        inner.push(op::CALL);
        uleb128(imp("get-int"), &mut inner);
        inner.push(op::I32_WRAP_I64);
        inner.push(op::CALL_INDIRECT);
        uleb128(lifted_type_idx as u64, &mut inner);
        uleb128(0, &mut inner);
        set(rep, &mut inner);
        // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally), before
        // the value-encode. Separate from the closure cell + the collection result handle.
        for ti in 0..tuples.len() as u32 {
            emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
        }
        // OWN: drop the cell now. BORROW: host keeps it (repeatable), dtor reclaims — do NOT drop. The
        // transient collection handle `rep` is separate and released after value-encode.
        if !call_borrow {
            get(cell, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
        }
        // desc = bytes-alloc(len); bytes-set each constant descriptor byte.
        ci32(descriptor.len() as i64, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-alloc"), &mut inner);
        set(desc, &mut inner);
        for (j, &byte) in descriptor.iter().enumerate() {
            get(desc, &mut inner);
            ci32(j as i64, &mut inner);
            ci32(byte as i64, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-set"), &mut inner);
            set(desc, &mut inner);
        }
        // doc = value-encode(rep, desc); n = bytes-len(doc).
        get(rep, &mut inner);
        get(desc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("value-encode"), &mut inner);
        set(doc, &mut inner);
        get(doc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-len"), &mut inner);
        set(nlen, &mut inner);
        // GROW to cover [OUT, OUT+n) before the copy — a >64-KiB value-form document would otherwise write
        // OOB past the single initial page (the #7793 page-boundary class, on the closure value-encode
        // escape copy-out). No-op when memory already suffices.
        emit_grow_to_cover_out(nlen, OUT, &mut inner);
        // copy loop: for i in 0..n { store8(OUT+i, bytes-get(doc, i)) }.
        ci32(0, &mut inner);
        set(iv, &mut inner);
        inner.push(op::BLOCK);
        inner.push(wasm_abi::BLOCK_EMPTY);
        inner.push(op::LOOP);
        inner.push(wasm_abi::BLOCK_EMPTY);
        get(iv, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_GE_U);
        inner.push(op::BR_IF);
        uleb128(1, &mut inner);
        ci32(OUT, &mut inner);
        get(iv, &mut inner);
        inner.push(op::I32_ADD);
        get(doc, &mut inner);
        get(iv, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("bytes-get"), &mut inner);
        inner.push(op::I32_STORE8);
        inner.push(0x00);
        inner.push(0x00);
        get(iv, &mut inner);
        ci32(1, &mut inner);
        inner.push(op::I32_ADD);
        set(iv, &mut inner);
        inner.push(op::BR);
        uleb128(0, &mut inner);
        inner.push(op::END);
        inner.push(op::END);
        ci32(0, &mut inner);
        ci32(OUT, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        ci32(4, &mut inner);
        get(nlen, &mut inner);
        inner.push(op::I32_STORE);
        inner.push(0x02);
        inner.push(0x00);
        get(rep, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        get(desc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        get(doc, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        ci32(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // cabi_realloc stub.
    {
        let mut inner = uleb_bytes(0);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(wasm_abi::CORE_SEC_CODE, &wasm_vec(n + nmk + 2, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// The MULTI-EXPORT COMPOUND-VALUE-result closure core module: N `make-<name>` functions sharing ONE
/// `call` that returns `list<u8>` carrying the canonical VALUE FORM of a tuple/record/sum result. Combines
/// [`multi_closure_bytes_resource_core_module`] (N makes + shared list-`call` + memory/cabi_realloc + plain
/// exports) with [`closure_value_resource_core_module`]'s value-form body (a data-section template walked
/// from the closure's returned handle). Every export shares the closure SIGNATURE — hence the SAME result
/// type + the ONE value-form `template` — so a single shared `call` dispatches whichever closure a handle
/// names (the code slot travels in the rep), then walks its compound result into the template.
#[allow(clippy::too_many_arguments)]
pub fn multi_closure_value_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    makes: &[ClosureMake],
    plain: &[PlainExport],
    arg_vts: &[ValType],
    lifted_type_idx: u32,
    template: &crate::lower::ValueFormTemplate,
    layout: &Layout,
    call_borrow: bool,
    tuples: &[TupleArgRebuild],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let nmk = makes.len();
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── imports 0..k; resource-new/rep; defined bodies; N make functypes; shared call
    // `(i32 self, args…)->i32 retptr`; cabi_realloc `(i32×4)->i32`.
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
    type_items.extend_from_slice(&i32_to_i32); // resource-new (k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let make_type_base = defined_type_base + n;
    for mk in makes {
        let params: Vec<u8> = mk.param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let call_type_idx = make_type_base + nmk;
    {
        let mut params = vec![wasm_abi::CORE_I32];
        params.extend(arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let realloc_type_idx = call_type_idx + 1;
    {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + nmk + 2;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ──
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

    // ── Function section ── defined bodies, N makes, call, cabi_realloc.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    for i in 0..nmk {
        uleb128((make_type_base + i) as u64, &mut func_items);
    }
    uleb128(call_type_idx as u64, &mut func_items);
    uleb128(realloc_type_idx as u64, &mut func_items);
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + nmk + 2, &func_items),
    );
    let make_abs_base = (defined_type_base + n) as u32;
    let call_abs = make_abs_base + nmk as u32;
    let realloc_abs = call_abs + 1;

    // ── Table + Memory ──
    let n_lifted = layout.lifted.len();
    let mut table_entry = vec![0x70u8, 0x01];
    uleb128(n_lifted as u64, &mut table_entry);
    uleb128(n_lifted as u64, &mut table_entry);
    let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
    let mem_sec = section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]));

    // ── Export section ── memory, N make-<name>, call, cabi_realloc, then plain exports.
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
        for (i, mk) in makes.iter().enumerate() {
            items.extend_from_slice(&export(
                &mk.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                make_abs_base + i as u32,
            ));
        }
        items.extend_from_slice(&export("call", wasm_abi::EXPORT_KIND_FUNC, call_abs));
        items.extend_from_slice(&export(
            "cabi_realloc",
            wasm_abi::EXPORT_KIND_FUNC,
            realloc_abs,
        ));
        for p in plain {
            items.extend_from_slice(&export(
                &p.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                p.body_abs,
            ));
        }
        section(
            wasm_abi::CORE_SEC_EXPORT,
            &wasm_vec(nmk + 3 + plain.len(), &items),
        )
    };

    // ── Element ──
    let elem_sec = {
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg))
    };

    // ── Data section ── the value-form template at `byte_off=0`, its `(ptr,len)` return area at `ret_off`.
    let byte_off = 0usize;
    let mut data_bytes: Vec<u8> = template.bytes.clone();
    let ret_off = (data_bytes.len() + 3) & !3;
    data_bytes.resize(ret_off, 0);
    data_bytes.extend_from_slice(&(byte_off as u32).to_le_bytes());
    data_bytes.extend_from_slice(&(template.bytes.len() as u32).to_le_bytes());
    let data_sec = {
        let mut item = vec![0x00];
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut item);
        item.push(op::END);
        item.extend_from_slice(&uleb_bytes(data_bytes.len() as u64));
        item.extend_from_slice(&data_bytes);
        section(wasm_abi::CORE_SEC_DATA, &wasm_vec(1, &item))
    };

    // ── Code section ── defined bodies, N makes, shared value-form call, cabi_realloc.
    let imp = |name: &str| {
        *import_index
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` imported")) as u64
    };
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    for mk in makes {
        let mut inner = uleb_bytes(0);
        for p in 0..mk.param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(mk.export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // The shared value-form `call` — identical body to the single-export value core (the code slot is
    // recovered from the rep, so ONE `call` serves all makes; the value form is the same for every export
    // since they share the result type).
    {
        let arity = arg_vts.len() as u32;
        let cell = 1 + arity;
        let rep = cell + 1;
        // i32 group FIRST (cell, rep, [one per tuple]) then the i64 scratch — scratch index = cell + n_i32.
        let n_i32: u32 = 2 + tuples.len() as u32; // cell, rep, + one i32 per rebuilt tuple-arg cell
        let tuple_local = rep + 1; // the first rebuilt tuple cell (only valid when !tuples.is_empty())
        let scratch = cell + n_i32;
        let mut inner = Vec::new();
        inner.extend_from_slice(&wasm_vec(2, &{
            let mut g = uleb_bytes(n_i32 as u64);
            g.push(wasm_abi::CORE_I32);
            let mut g2 = uleb_bytes(1);
            g2.push(wasm_abi::CORE_I64);
            g.extend_from_slice(&g2);
            g
        }));
        let get = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_GET);
            uleb128(l as u64, out);
        };
        let set = |l: u32, out: &mut Vec<u8>| {
            out.push(op::LOCAL_SET);
            uleb128(l as u64, out);
        };
        // cell = self (borrow: rep passed directly) or resource.rep(self) (own).
        get(0, &mut inner);
        if !call_borrow {
            inner.push(op::CALL);
            uleb128(f_rrep as u64, &mut inner);
        }
        set(cell, &mut inner);
        get(cell, &mut inner);
        emit_closure_call_args(tuples, tuple_local, arity, &imp, &mut inner);
        get(cell, &mut inner);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("arr-get"), &mut inner);
        inner.push(op::CALL);
        uleb128(imp("get-int"), &mut inner);
        inner.push(op::I32_WRAP_I64);
        inner.push(op::CALL_INDIRECT);
        uleb128(lifted_type_idx as u64, &mut inner);
        uleb128(0, &mut inner);
        set(rep, &mut inner);
        // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally), before
        // walking the compound result. Separate from the closure cell + the compound result handle.
        for ti in 0..tuples.len() as u32 {
            emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
        }
        // OWN: drop the cell now. BORROW: host keeps it (repeatable), dtor reclaims — do NOT drop. The
        // transient compound handle `rep` is separate and dropped after the walk.
        if !call_borrow {
            get(cell, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
        }
        for hole in &template.leaves {
            emit_hole_fill(hole, byte_off, rep, scratch, &import_index, &mut inner);
        }
        get(rep, &mut inner);
        inner.push(op::CALL);
        uleb128(imp("drop"), &mut inner);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(ret_off as i64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    {
        let mut inner = uleb_bytes(0);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(wasm_abi::CORE_SEC_CODE, &wasm_vec(n + nmk + 2, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    Ok(core)
}

/// The MULTI-EXPORT closure-resource core module: N `make-<name>` functions (one per closure export,
/// each building its export's cell + `resource.new`) sharing ONE `call` method. The shared `call` is the
/// load-bearing realization (proven by the `multi_export_closures_share_one_call` oracle): the closure's
/// code slot is recovered from the resource rep at call time (`resource.rep` → `arr-get(cell,0)` →
/// `call_indirect`), so ONE `call` dispatches WHICHEVER closure a handle names, provided all exports share
/// the closure SIGNATURE (`arg_vts`/`ret_vt`/`lifted_type_idx` are common — distinct signatures are a
/// later slice with N resource types). Func/type index layout mirrors the single-export shape with the
/// single make replaced by N makes: imports 0..k+2, n defined bodies, then N makes, then `call`.
#[allow(clippy::too_many_arguments)]
pub fn multi_closure_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    makes: &[ClosureMake],
    plain: &[PlainExport],
    arg_vts: &[ValType],
    ret_vt: ValType,
    lifted_type_idx: u32,
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    multi_closure_resource_core_module_borrow(
        funcs,
        imports,
        makes,
        plain,
        arg_vts,
        ret_vt,
        lifted_type_idx,
        layout,
        false,
    )
}

/// [`multi_closure_resource_core_module`] with a `call_borrow` switch — the multi-export scalar shared-`call`
/// front to [`multi_closure_resource_core_module_with_host_borrow`]. `call_borrow = true` makes the ONE
/// shared `call` a repeatable `borrow<t>` method (each make's handle survives across calls; the `t-dtor`
/// reclaims on drop); `false` the shipped own/self-drop shared `call`.
#[allow(clippy::too_many_arguments)]
pub fn multi_closure_resource_core_module_borrow(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    makes: &[ClosureMake],
    plain: &[PlainExport],
    arg_vts: &[ValType],
    ret_vt: ValType,
    lifted_type_idx: u32,
    layout: &Layout,
    call_borrow: bool,
) -> Result<Vec<u8>, String> {
    multi_closure_resource_core_module_with_host_borrow(
        funcs,
        imports,
        &[],
        makes,
        plain,
        arg_vts,
        ret_vt,
        lifted_type_idx,
        layout,
        call_borrow,
        &[],
        &[],
    )
}

/// [`multi_closure_resource_core_module`] with a HOST-IMPORT set (the build-time-delegated closure-capture
/// case). Host ops are laid FIRST in the import section (core funcs `0..h`), so a `Lir::CallHostImport(i)`
/// — whose `i` is the op's RAW position in `layout.host_order` — resolves to exactly core func `i` with NO
/// index recomputation (the same invariant the plain `emit` path relies on). The value-heap runtime ops
/// then occupy `h..h+k`, and the resource intrinsics `h+k`, `h+k+1`; every derived base shifts by `h`.
/// `host_fns` EMPTY reproduces the original layout byte-for-byte (the closure path with no host effect).
#[allow(clippy::too_many_arguments)]
pub fn multi_closure_resource_core_module_with_host(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    host_fns: &[crate::backend::wasm::host::HostImport],
    makes: &[ClosureMake],
    plain: &[PlainExport],
    arg_vts: &[ValType],
    ret_vt: ValType,
    lifted_type_idx: u32,
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    multi_closure_resource_core_module_with_host_borrow(
        funcs,
        imports,
        host_fns,
        makes,
        plain,
        arg_vts,
        ret_vt,
        lifted_type_idx,
        layout,
        false,
        &[],
        &[],
    )
}

/// [`multi_closure_resource_core_module_with_host`] with a `call_borrow` switch. When `call_borrow` is
/// TRUE the shared scalar `call` takes `borrow<t>` instead of `own<t>`: the component-model `lift_borrow`
/// hands the guest the resource REP DIRECTLY as `call`'s `self` param (NOT a table index), so the body uses
/// param 0 as the cell rep WITHOUT `resource.rep` (which TRAPS on a borrow in wasmtime 37) and does NOT drop
/// the cell (the host keeps ownership; the `t-dtor` reclaims when the host finally drops the handle). This
/// makes the closure handle REPEATABLE — the host can `call` it any number of times before dropping it (the
/// natural callback shape), versus `own<t>`'s consume-per-call. The value-heap `encode` borrow method proved
/// this shape runs under wasmtime 37. `false` reproduces the own/self-drop body byte-for-byte (the shipped
/// leak-free single-use posture). Only the SCALAR-result `call` body differs; the type/functype/export
/// layout is identical (the own-vs-borrow distinction is a COMPONENT-type detail the envelope carries).
#[allow(clippy::too_many_arguments)]
pub fn multi_closure_resource_core_module_with_host_borrow(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    host_fns: &[crate::backend::wasm::host::HostImport],
    makes: &[ClosureMake],
    plain: &[PlainExport],
    arg_vts: &[ValType],
    ret_vt: ValType,
    lifted_type_idx: u32,
    layout: &Layout,
    call_borrow: bool,
    // ZERO OR MORE fixed-shape tuple/record args (each a `TupleArgRebuild` at its own `base_param`, ascending).
    // The scalar `call` rebuilds each cell from its flattened fields, interleaved with scalars in arg order,
    // and drops each after `call_indirect`. `&[]` = no tuple arg (byte-identical to the scalar path).
    tuples: &[TupleArgRebuild],
    // ZERO OR MORE fixed-shape SUM args (Option/Result — each a `SumArgRebuild` at its own `base_param`). The
    // `call` rebuilds each sum cell by branching on the flattened disc + `sum-new`, and drops each after
    // `call_indirect`. `&[]` = no sum arg (byte-identical). This increment wires the SOLE-sum-arg case.
    sums: &[SumArgRebuild],
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let h = host_fns.len();
    let k = imports.len();
    let n = funcs.len();
    let nmk = makes.len();
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── HOST-op functypes 0..h, then runtime import functypes h..h+k, resource-new/
    // resource-rep (h+k, h+k+1), one functype per defined body, then one make functype PER make (params
    // may differ per export), then call `(i32 self, args…)->R`.
    let mut type_items = Vec::new();
    for f in host_fns {
        type_items.extend_from_slice(&host_import_functype(f));
    }
    for o in imports {
        type_items.extend_from_slice(&import_functype(o));
    }
    let i32_to_i32 = {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        t
    };
    type_items.extend_from_slice(&i32_to_i32); // resource-new (index h+k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (index h+k+1)
    let defined_type_base = h + k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    // make[i] `(export-params…)->i32` — one per closure export (its type index = defined_type_base+n+i).
    let make_type_base = defined_type_base + n;
    for mk in makes {
        let params: Vec<u8> = mk.param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    // call `(i32 self, args…) -> R` — shared across all makes (same closure signature).
    let call_type_idx = make_type_base + nmk;
    {
        let mut params = vec![wasm_abi::CORE_I32]; // self rep
        params.extend(arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[vt_byte(ret_vt)]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + nmk + 1;
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── h HOST ops (from "host", core funcs 0..h — so a `CallHostImport(i)` hits func
    // `i` verbatim), then k runtime ops (from "heap", h..h+k), then resource-new + resource-rep (h+k,
    // h+k+1). Their type indices match the type-section order above.
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut import_items = Vec::new();
    for (i, f) in host_fns.iter().enumerate() {
        import_items.extend_from_slice(&host_import_item(&f.op, i as u32));
        // NOTE: a host op is called via `CallHostImport(raw host_order position)`, NOT via `import_index`
        // (which keys runtime-op NAMES); no map entry needed. Its core func index IS `i` by construction.
    }
    for (j, o) in imports.iter().enumerate() {
        let ti = (h + j) as u32;
        import_items.extend_from_slice(&import_item(o.name, ti));
        import_index.insert(o.name, ti);
    }
    import_items.extend_from_slice(&import_item("resource-new", (h + k) as u32));
    import_items.extend_from_slice(&import_item("resource-rep", (h + k + 1) as u32));
    let import_sec = section(2, &wasm_vec(h + k + 2, &import_items));
    let f_rnew = (h + k) as u32;
    let f_rrep = (h + k + 1) as u32;

    // ── Function section ── defined bodies, then the N makes, then call.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    for i in 0..nmk {
        uleb128((make_type_base + i) as u64, &mut func_items);
    }
    uleb128(call_type_idx as u64, &mut func_items);
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + nmk + 1, &func_items),
    );
    let make_abs_base = (defined_type_base + n) as u32; // first make's core func index
    let call_abs = make_abs_base + nmk as u32;

    // ── Table + Element sections ── the funcref table holding the lifted closure(s), from `layout.lifted`
    // (same shape `core_module` emits). REQUIRED here: `call` dispatches `call_indirect` over table 0.
    let n_lifted = layout.lifted.len();
    let (table_sec, elem_sec) = if n_lifted == 0 {
        (Vec::new(), Vec::new())
    } else {
        let mut table_entry = vec![0x70u8, 0x01];
        uleb128(n_lifted as u64, &mut table_entry);
        uleb128(n_lifted as u64, &mut table_entry);
        let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        let elem_sec = section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg));
        (table_sec, elem_sec)
    };

    // ── Export section ── the N makes (each under its own boundary name) + call (no memory: a
    // scalar-arg/result `call` needs no linear memory).
    let export_sec = {
        let export = |name: &str, kind: u8, idx: u32| {
            let mut item = uleb_bytes(name.len() as u64);
            item.extend_from_slice(name.as_bytes());
            item.push(kind);
            uleb128(idx as u64, &mut item);
            item
        };
        let mut items = Vec::new();
        for (i, mk) in makes.iter().enumerate() {
            items.extend_from_slice(&export(
                &mk.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                make_abs_base + i as u32,
            ));
        }
        items.extend_from_slice(&export("call", wasm_abi::EXPORT_KIND_FUNC, call_abs));
        // PLAIN (non-closure) exports ride along: their bodies are already defined funcs, so just name each
        // by its core-func index (the envelope aliases + lifts them as ordinary top-level component funcs).
        for p in plain {
            items.extend_from_slice(&export(
                &p.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                p.body_abs,
            ));
        }
        section(
            wasm_abi::CORE_SEC_EXPORT,
            &wasm_vec(nmk + 1 + plain.len(), &items),
        )
    };

    // ── Code section ── defined bodies, then the N makes, then call.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // make[i]: forward its export's params (locals 0..len), `call <export body>` (builds the closure cell,
    // closing over those params), then `resource.new`.
    for mk in makes {
        let mut inner = uleb_bytes(0); // make declares no locals of its own
        for p in 0..mk.param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(mk.export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // call(self, args…): recover the cell rep, materialize it into a local (read twice — as the env and
    // for the code slot), push env + args, read the table slot, `call_indirect`.
    {
        // Locals beyond the params: one i32 for the cell rep, plus (for a flattened tuple arg) one i32 for
        // the rebuilt tuple-cell handle. Params are: 0 = self, 1..1+arity = the closure's args (the FLATTENED
        // tuple fields when `tuple_arg` is set). `arg_vts` is the boundary/core param list either way.
        let cell_local = (1 + arg_vts.len()) as u32;
        // One i32 per tuple arg for its rebuilt cell handle (at `tuple_local + i`), after the cell-rep local;
        // then one i32 per SUM arg (at `sum_local + i`), after the tuple locals.
        let tuple_local = cell_local + 1;
        let sum_local = tuple_local + tuples.len() as u32;
        let n_extra_locals = 1 + tuples.len() as u32 + sums.len() as u32;
        let mut inner = Vec::new();
        // one local group: n_extra_locals × i32.
        inner.extend_from_slice(&wasm_vec(1, &{
            let mut g = uleb_bytes(n_extra_locals as u64);
            g.push(wasm_abi::CORE_I32);
            g
        }));
        // rep = self. With `own<t>`, wasmtime hands `self` (param 0) as a resource-TABLE index, so the cell
        // rep is `resource.rep(self)`. With `borrow<t>`, `lift_borrow` hands the REP DIRECTLY as param 0 (no
        // table index), so the rep IS `self` — and `resource.rep` on a borrow TRAPS in wasmtime 37, so it
        // must NOT be called. Either way `cell_local` ends up holding the heap cell rep.
        inner.push(op::LOCAL_GET);
        uleb128(0, &mut inner);
        if !call_borrow {
            inner.push(op::CALL);
            uleb128(f_rrep as u64, &mut inner);
        }
        inner.push(op::LOCAL_SET);
        uleb128(cell_local as u64, &mut inner);
        // push env (the cell) then the closure's argument(s). The lifted fn is `(env, closure-args…) -> R`.
        inner.push(op::LOCAL_GET);
        uleb128(cell_local as u64, &mut inner);
        // Push the closure args, threading each fixed-shape tuple-arg rebuild among the scalars (or the raw
        // scalar args when `tuples` is empty). See [`emit_closure_call_args`].
        {
            let imp = |name: &str| *import_index.get(name).expect("rebuild op imported") as u64;
            emit_closure_call_args_with_sums(
                tuples,
                tuple_local,
                sums,
                sum_local,
                arg_vts.len() as u32,
                &imp,
                &mut inner,
            );
        }
        // indirection index: arr-get(cell, 0) → get-int → i32.wrap_i64.
        inner.push(op::LOCAL_GET);
        uleb128(cell_local as u64, &mut inner);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::CALL);
        uleb128(
            *import_index.get("arr-get").expect("arr-get imported") as u64,
            &mut inner,
        );
        inner.push(op::CALL);
        uleb128(
            *import_index.get("get-int").expect("get-int imported") as u64,
            &mut inner,
        );
        inner.push(op::I32_WRAP_I64);
        inner.push(op::CALL_INDIRECT);
        uleb128(lifted_type_idx as u64, &mut inner);
        uleb128(0, &mut inner); // table 0
        // Ownership release. With `own<t>` the canonical ABI transferred ownership INTO `call`, so it owns
        // the cell's last reference — RELEASE it now (`heap.drop(rep)`), after `call_indirect` returned (the
        // lifted body finished BORROWING the env for its captures). Balances `make`'s `arr-alloc`, so an
        // own make+call leaves NO live heap cell. With `borrow<t>` the host KEEPS ownership across calls
        // (the handle is repeatable), so `call` must NOT drop — the `t-dtor` reclaims the cell when the host
        // finally drops the handle (`resource_dtor_module_with_drop`). The result R is already on the stack;
        // a `drop` takes the rep (a separate push) and returns nothing, leaving R on top.
        if !call_borrow {
            inner.push(op::LOCAL_GET);
            uleb128(cell_local as u64, &mut inner);
            inner.push(op::CALL);
            uleb128(
                *import_index
                    .get("drop")
                    .expect("drop imported for the closure-cell release") as u64,
                &mut inner,
            );
        }
        // Each REBUILT tuple-arg cell is an owned temporary this `call` fabricated per invocation (NOT owned
        // by the host across calls, unlike the closure handle) — so each drops UNCONDITIONALLY here (both own
        // and borrow), after the lifted body finished borrowing it, balancing its `arr-alloc`. Leaves R on top.
        for ti in 0..tuples.len() as u32 {
            inner.push(op::LOCAL_GET);
            uleb128((tuple_local + ti) as u64, &mut inner);
            inner.push(op::CALL);
            uleb128(
                *import_index
                    .get("drop")
                    .expect("drop imported for the tuple-arg cell release") as u64,
                &mut inner,
            );
        }
        // Each REBUILT sum-arg cell is likewise an owned per-call temporary — drop it unconditionally,
        // balancing its `sum-new`. (`sum-new` with the inline-unit payload allocs a heap sum node either way.)
        for si in 0..sums.len() as u32 {
            inner.push(op::LOCAL_GET);
            uleb128((sum_local + si) as u64, &mut inner);
            inner.push(op::CALL);
            uleb128(
                *import_index
                    .get("drop")
                    .expect("drop imported for the sum-arg cell release") as u64,
                &mut inner,
            );
        }
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(wasm_abi::CORE_SEC_CODE, &wasm_vec(n + nmk + 1, &code_items));

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    Ok(core)
}

/// One SIGNATURE GROUP for the distinct-signature multi-export ([`distinct_sig_resource_core_module`]):
/// all exports sharing ONE closure signature `(-> A… R)` — they become ONE resource type with its own
/// `resource.new`/`resource.rep` intrinsics + one shared `call`. `arg_vts`/`ret_vt` are the group's `call`
/// boundary shape; `lifted_type_idx` the group's `call_indirect` functype; `makes` its per-export `make`s.
pub struct SigGroup {
    pub makes: Vec<ClosureMake>,
    pub arg_vts: Vec<ValType>,
    pub ret_vt: ValType,
    /// The table SLOT of a REPRESENTATIVE lifted lambda of this group's signature — its `call_indirect`
    /// functype index in the core is derived from it (`defined_type_base + order.len() + slot`), so the
    /// caller passes the slot rather than a pre-baked type index (the distinct-sig core's type layout
    /// differs from the multi-export one, so `layout.lifted_type_index` cannot be reused here).
    pub lifted_slot: usize,
    /// True when this group's closure result is a byte-rope (`Bytes`/`String`) — its `call-<g>` returns an
    /// i32 retptr into memory (a `list<u8>` at the canon boundary) instead of a scalar. `ret_vt` is `I32`
    /// either way (a byte-rope handle IS an i32), so the core FUNCTYPE is identical; only the call BODY (a
    /// `bytes-len`/`bytes-get` copy loop writing a `(ptr,len)` return area) and the envelope's list-lift
    /// differ. When ANY group is byte-rope the core gains a memory + `cabi_realloc` (shared across groups).
    pub ret_is_bytes: bool,
    /// `Some(template)` when this group's closure result is a fixed-shape COMPOUND (tuple/record/sum) — its
    /// `call-<g>` returns the canonical VALUE FORM as `list<u8>` (walking the returned handle into the
    /// per-group value-form template). Mutually exclusive with `ret_is_bytes` (both cross as `list<u8>` but
    /// a compound writes the value form, a byte-rope the raw payload). Each compound group's template gets
    /// its OWN data-section region; byte-rope groups write dynamically PAST all compound data so the two
    /// never collide. When any group is byte-rope OR compound the core gains a memory + `cabi_realloc`.
    pub ret_template: Option<crate::lower::ValueFormTemplate>,
    /// `Some(descriptor)` when this group's closure result is a VARIABLE-LENGTH collection (List/Map/Set) —
    /// its `call-<g>` renders the value form at run time via `value-encode(rep, desc)` against this group's
    /// shape descriptor (no static template). Like a byte-rope group it writes its runtime-length payload
    /// PAST all compound-template data (`bytes_out_off`), so a compound group + a collection group + a
    /// byte-rope group never collide. Mutually exclusive with `ret_is_bytes`/`ret_template`.
    pub ret_descriptor: Option<Vec<u8>>,
    /// ZERO OR MORE fixed-shape tuple/record ARGUMENTS this group's closure takes (the direct-call
    /// compound-arg path). Each arg crossed the boundary FLATTENED into its scalar fields, so the group's
    /// `call-<g>` rebuilds each tuple cell (`arr-alloc N` + per field box/`arr-set`, at `tuple_local + i`)
    /// before `call_indirect`, then drops each rebuilt cell (an owned per-call temporary). `arg_vts` for such
    /// a group is the FULL flattened field valtypes of every arg. `&[]` = scalar args (byte-identical); a
    /// single rebuild reproduces the one-tuple body byte-for-byte; ≥2 is the N-compound-args case.
    pub tuples: Vec<TupleArgRebuild>,
    /// ZERO OR MORE fixed-shape SUM args (Option/Result) this group's closure takes — the `call-<g>` rebuilds
    /// each sum cell (branch on the flattened disc → `sum-new`) before `call_indirect`, then drops each. `&[]`
    /// = no sum arg (byte-identical). This increment wires the SOLE-sum-arg per group.
    pub sums: Vec<SumArgRebuild>,
}

/// The DISTINCT-SIGNATURE multi-export core module: closures of G DIFFERENT signatures cross as G resource
/// types. Each group `g` gets its OWN `resource-new-g`/`resource-rep-g` imports (a core `resource.new` is
/// typed to ONE resource — so `make` for group g news through group g's intrinsic), its per-export
/// `make-<name>` functions, and ONE shared `call-<g>` (dispatching any closure of that signature via the
/// group's `resource.rep-g` → the shared funcref table). All groups share the ONE guest funcref table (the
/// resource-TYPE distinction is a boundary concern; a table slot is a slot). The `distinct_signature_…`
/// oracle proved the shape. Exports: per group, its makes (`make-<name>`) + `call-<g>`.
pub fn distinct_sig_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    groups: &[SigGroup],
    plain: &[PlainExport],
    layout: &Layout,
    call_borrow: bool,
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let g = groups.len();
    let total_makes: usize = groups.iter().map(|gr| gr.makes.len()).sum();
    // A group whose result crosses as `list<u8>` — a byte-rope (`ret_is_bytes`) OR a fixed-shape COMPOUND
    // (`ret_template`). Either makes the component need a memory + `cabi_realloc` (shared across groups); the
    // envelope lifts each such group with the Memory/Realloc canon options. A compound group writes the
    // VALUE FORM from its own data-section template region; a byte-rope group writes a runtime-length
    // payload PAST all compound data so the two never collide.
    let is_list =
        |gr: &SigGroup| gr.ret_is_bytes || gr.ret_template.is_some() || gr.ret_descriptor.is_some();
    let any_list = groups.iter().any(is_list);
    // Per COMPOUND group: place its template + `(ptr,len)` retarea in the data section (4-aligned), record
    // `(byte_off, ret_off)`. `data_end` is the 4-aligned end of all compound data — where byte-rope groups
    // put their dynamic retarea (`data_end`) + payload (`data_end + 8`); only one `call` runs per host
    // invocation, so all byte-rope groups can share that region.
    let mut data_bytes: Vec<u8> = Vec::new();
    let mut compound_place: Vec<Option<(usize, usize)>> = Vec::with_capacity(g);
    for gr in groups {
        if let Some(t) = &gr.ret_template {
            let byte_off = (data_bytes.len() + 3) & !3;
            data_bytes.resize(byte_off, 0);
            data_bytes.extend_from_slice(&t.bytes);
            let ret_off = (data_bytes.len() + 3) & !3;
            data_bytes.resize(ret_off, 0);
            data_bytes.extend_from_slice(&(byte_off as u32).to_le_bytes());
            data_bytes.extend_from_slice(&(t.bytes.len() as u32).to_le_bytes());
            compound_place.push(Some((byte_off, ret_off)));
        } else {
            compound_place.push(None);
        }
    }
    let bytes_ret_off = (data_bytes.len() + 3) & !3; // byte-rope retarea (past all compound templates)
    let bytes_out_off = bytes_ret_off + 8; // byte-rope payload starts after its 8-byte (ptr,len) area
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };

    // ── Type section ── import functypes 0..k; then 2*G resource-intrinsic functypes `(i32)->i32` (new_g,
    // rep_g per group); one functype per defined body; then per group: its make functype(s) + one call
    // functype. To keep indices simple, ALL resource intrinsics share the ONE `(i32)->i32` functype.
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
    // One shared `(i32)->i32` functype for every resource intrinsic (index k).
    type_items.extend_from_slice(&i32_to_i32);
    let rintr_type_idx = k as u32;
    let defined_type_base = k + 1;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    // Per group: make functype(s) then a call functype. Record each function's type index.
    let mut make_type_idx: Vec<u32> = Vec::new(); // flat, in (group, make) order
    let mut call_type_idx: Vec<u32> = Vec::new(); // one per group
    let mut next_type = defined_type_base + n;
    for gr in groups {
        for mk in &gr.makes {
            let params: Vec<u8> = mk.param_vts.iter().map(|v| vt_byte(*v)).collect();
            let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
            t.extend_from_slice(&wasm_vec(params.len(), &params));
            t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
            type_items.extend_from_slice(&t);
            make_type_idx.push(next_type as u32);
            next_type += 1;
        }
        let mut params = vec![wasm_abi::CORE_I32]; // self rep
        params.extend(gr.arg_vts.iter().map(|v| vt_byte(*v)));
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[vt_byte(gr.ret_vt)]));
        type_items.extend_from_slice(&t);
        call_type_idx.push(next_type as u32);
        next_type += 1;
    }
    // If any group crosses as `list<u8>`, one shared `cabi_realloc` functype `(i32×4)->i32` after the group
    // functypes.
    let realloc_type_idx = next_type as u32;
    if any_list {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + total_makes + g + usize::from(any_list);
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── k ops (each against its own `import_functype` at index i) + per group
    // `resource-new-<g>` + `resource-rep-<g>` (2*G intrinsics, all against the shared `(i32)->i32` type).
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut import_items = Vec::new();
    for (i, o) in imports.iter().enumerate() {
        import_items.extend_from_slice(&import_item(o.name, i as u32));
        import_index.insert(o.name, i as u32);
    }
    let mut rnew_fn: Vec<u32> = Vec::new();
    let mut rrep_fn: Vec<u32> = Vec::new();
    let mut next_import_fn = k as u32;
    for gi in 0..g {
        import_items.extend_from_slice(&import_item(&format!("resource-new-{gi}"), rintr_type_idx));
        rnew_fn.push(next_import_fn);
        next_import_fn += 1;
        import_items.extend_from_slice(&import_item(&format!("resource-rep-{gi}"), rintr_type_idx));
        rrep_fn.push(next_import_fn);
        next_import_fn += 1;
    }
    let import_sec = section(2, &wasm_vec(k + 2 * g, &import_items));

    // ── Function section ── defined bodies, then per group (makes then call).
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    for &ti in &make_type_idx {
        uleb128(ti as u64, &mut func_items);
    }
    for &ti in &call_type_idx {
        uleb128(ti as u64, &mut func_items);
    }
    if any_list {
        uleb128(realloc_type_idx as u64, &mut func_items);
    }
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + total_makes + g + usize::from(any_list), &func_items),
    );
    // Absolute core-func indices: defined bodies at import_count..; then makes; then calls; then (if any
    // list-returning group) the shared cabi_realloc.
    let import_count = k + 2 * g;
    let defined_abs_base = import_count as u32;
    let make_abs_base = defined_abs_base + n as u32;
    let call_abs_base = make_abs_base + total_makes as u32;
    let realloc_abs = call_abs_base + g as u32; // valid only when any_list

    // ── Memory ── only when a list-returning group needs to write its `list<u8>` payload.
    let mem_sec = if any_list {
        section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]))
    } else {
        Vec::new()
    };

    // ── Table + Element ── the ONE funcref table from `layout.lifted` (all groups' lifteds share it).
    let n_lifted = layout.lifted.len();
    let (table_sec, elem_sec) = if n_lifted == 0 {
        (Vec::new(), Vec::new())
    } else {
        let mut table_entry = vec![0x70u8, 0x01];
        uleb128(n_lifted as u64, &mut table_entry);
        uleb128(n_lifted as u64, &mut table_entry);
        let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        let elem_sec = section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg));
        (table_sec, elem_sec)
    };

    // ── Export section ── per group: its makes (each `make-<name>`) + `call-<g>`; plus (when any byte-rope
    // group) `memory` + `cabi_realloc` for the compound `call`'s canon lift.
    let export_sec = {
        let export = |name: &str, kind: u8, idx: u32| {
            let mut item = uleb_bytes(name.len() as u64);
            item.extend_from_slice(name.as_bytes());
            item.push(kind);
            let mut b = item;
            uleb128(idx as u64, &mut b);
            b
        };
        let func_export = |name: &str, idx: u32| export(name, wasm_abi::EXPORT_KIND_FUNC, idx);
        let mut items = Vec::new();
        let mut make_i = 0u32;
        for (gi, gr) in groups.iter().enumerate() {
            for mk in &gr.makes {
                items.extend_from_slice(&func_export(&mk.export_name, make_abs_base + make_i));
                make_i += 1;
            }
            items.extend_from_slice(&func_export(
                &format!("call-g{gi}"),
                call_abs_base + gi as u32,
            ));
        }
        // PLAIN (non-closure) exports ride along: their bodies are already defined funcs, so just name each
        // by its core-func index (the envelope aliases + lifts them as ordinary top-level component funcs).
        for p in plain {
            items.extend_from_slice(&func_export(&p.export_name, p.body_abs));
        }
        if any_list {
            items.extend_from_slice(&export("memory", wasm_abi::EXPORT_KIND_MEMORY, 0));
            items.extend_from_slice(&func_export("cabi_realloc", realloc_abs));
        }
        section(
            wasm_abi::CORE_SEC_EXPORT,
            &wasm_vec(
                total_makes + g + plain.len() + if any_list { 2 } else { 0 },
                &items,
            ),
        )
    };

    // ── Code section ── defined bodies, then per group (makes then call).
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // makes (flat, in group order): each forwards its export params, calls the export body, `resource.new-g`.
    for (gi, gr) in groups.iter().enumerate() {
        for mk in &gr.makes {
            let mut inner = uleb_bytes(0);
            for p in 0..mk.param_vts.len() {
                inner.push(op::LOCAL_GET);
                uleb128(p as u64, &mut inner);
            }
            inner.push(op::CALL);
            uleb128(mk.export_abs as u64, &mut inner);
            inner.push(op::CALL);
            uleb128(rnew_fn[gi] as u64, &mut inner);
            inner.push(op::END);
            let mut e = uleb_bytes(inner.len() as u64);
            e.extend_from_slice(&inner);
            code_items.extend_from_slice(&e);
        }
    }
    // calls (one per group): resource.rep-g → cell → dispatch the group's lifted functype. A SCALAR group
    // returns the dispatched value directly (drop the cell after); a BYTE-ROPE group's lifted call yields a
    // runtime Bytes/String handle, which the copy-loop writes out as a `list<u8>` `(ptr,len)` return area
    // returning an i32 retptr; a COMPOUND group walks the returned handle into ITS value-form template
    // region + returns that template's `(ptr,len)` retarea.
    let imp = |name: &str| import_index[name] as u64;
    for (gi, gr) in groups.iter().enumerate() {
        let lifted_tyi = (defined_type_base + layout.order.len() + gr.lifted_slot) as u32;
        let arity = gr.arg_vts.len() as u32;
        let mut inner = Vec::new();
        if let Some(descriptor) = &gr.ret_descriptor {
            // Variable-length collection: dispatch → the collection handle, drop the cell, build the
            // descriptor Bytes, value-encode(rep, desc) → the document, copy it out (PAST all compound data
            // at `bytes_out_off`), release rep/desc/doc, return the retptr (`bytes_ret_off`).
            let out_off = bytes_out_off as i64;
            let cell = 1 + arity;
            let rep = cell + 1;
            let desc = rep + 1;
            let doc = desc + 1;
            let nlen = doc + 1;
            let iv = nlen + 1;
            let tuple_local = iv + 1;
            let n_locals = 6 + gr.tuples.len() as u32; // cell/rep/desc/doc/n/i + one i32 per rebuilt tuple cell
            inner.extend_from_slice(&wasm_vec(1, &{
                let mut gl = uleb_bytes(n_locals as u64);
                gl.push(wasm_abi::CORE_I32);
                gl
            }));
            let get = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_GET);
                uleb128(l as u64, out);
            };
            let set = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_SET);
                uleb128(l as u64, out);
            };
            let ci32 = |v: i64, out: &mut Vec<u8>| {
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(v, out);
            };
            // cell = self (borrow: rep passed directly) or resource.rep-g(self) (own).
            get(0, &mut inner);
            if !call_borrow {
                inner.push(op::CALL);
                uleb128(rrep_fn[gi] as u64, &mut inner);
            }
            set(cell, &mut inner);
            get(cell, &mut inner);
            emit_closure_call_args(&gr.tuples, tuple_local, arity, &imp, &mut inner);
            get(cell, &mut inner);
            ci32(0, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("arr-get"), &mut inner);
            inner.push(op::CALL);
            uleb128(imp("get-int"), &mut inner);
            inner.push(op::I32_WRAP_I64);
            inner.push(op::CALL_INDIRECT);
            uleb128(lifted_tyi as u64, &mut inner);
            uleb128(0, &mut inner);
            set(rep, &mut inner);
            // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally).
            for ti in 0..gr.tuples.len() as u32 {
                emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
            }
            // OWN: drop the cell now. BORROW: host keeps it (repeatable), dtor reclaims — do NOT drop. The
            // transient collection handle `rep` is separate and released after value-encode.
            if !call_borrow {
                get(cell, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            // desc = bytes-alloc(len); bytes-set each constant descriptor byte.
            ci32(descriptor.len() as i64, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-alloc"), &mut inner);
            set(desc, &mut inner);
            for (j, &byte) in descriptor.iter().enumerate() {
                get(desc, &mut inner);
                ci32(j as i64, &mut inner);
                ci32(byte as i64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("bytes-set"), &mut inner);
                set(desc, &mut inner);
            }
            // doc = value-encode(rep, desc); n = bytes-len(doc).
            get(rep, &mut inner);
            get(desc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("value-encode"), &mut inner);
            set(doc, &mut inner);
            get(doc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-len"), &mut inner);
            set(nlen, &mut inner);
            ci32(0, &mut inner);
            set(iv, &mut inner);
            inner.push(op::BLOCK);
            inner.push(wasm_abi::BLOCK_EMPTY);
            inner.push(op::LOOP);
            inner.push(wasm_abi::BLOCK_EMPTY);
            get(iv, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_GE_U);
            inner.push(op::BR_IF);
            uleb128(1, &mut inner);
            ci32(out_off, &mut inner);
            get(iv, &mut inner);
            inner.push(op::I32_ADD);
            get(doc, &mut inner);
            get(iv, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-get"), &mut inner);
            inner.push(op::I32_STORE8);
            inner.push(0x00);
            inner.push(0x00);
            get(iv, &mut inner);
            ci32(1, &mut inner);
            inner.push(op::I32_ADD);
            set(iv, &mut inner);
            inner.push(op::BR);
            uleb128(0, &mut inner);
            inner.push(op::END);
            inner.push(op::END);
            ci32(bytes_ret_off as i64, &mut inner);
            ci32(out_off, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            ci32(bytes_ret_off as i64 + 4, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            get(rep, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            get(desc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            get(doc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            ci32(bytes_ret_off as i64, &mut inner);
            inner.push(op::END);
        } else if let Some(template) = &gr.ret_template {
            // Compound: dispatch → the compound handle (rep), drop the cell, walk the handle into this
            // group's template region, drop the handle, return the template's retarea pointer.
            let (byte_off, ret_off) =
                compound_place[gi].expect("a compound group has a data placement");
            let cell = 1 + arity;
            let rep = cell + 1;
            // i32 group FIRST (cell, rep, [one per tuple]) then the i64 scratch — scratch index = cell + n_i32.
            let n_i32: u32 = 2 + gr.tuples.len() as u32; // cell, rep, + one i32 per rebuilt tuple cell
            let tuple_local = rep + 1; // the first rebuilt tuple cell (only valid when !gr.tuples.is_empty())
            let scratch = cell + n_i32;
            inner.extend_from_slice(&wasm_vec(2, &{
                let mut gl = uleb_bytes(n_i32 as u64);
                gl.push(wasm_abi::CORE_I32);
                let mut g2 = uleb_bytes(1);
                g2.push(wasm_abi::CORE_I64);
                gl.extend_from_slice(&g2);
                gl
            }));
            let get = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_GET);
                uleb128(l as u64, out);
            };
            let set = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_SET);
                uleb128(l as u64, out);
            };
            // cell = self (borrow: rep passed directly) or resource.rep-g(self) (own).
            get(0, &mut inner);
            if !call_borrow {
                inner.push(op::CALL);
                uleb128(rrep_fn[gi] as u64, &mut inner);
            }
            set(cell, &mut inner);
            get(cell, &mut inner);
            emit_closure_call_args(&gr.tuples, tuple_local, arity, &imp, &mut inner);
            get(cell, &mut inner);
            inner.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(0, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("arr-get"), &mut inner);
            inner.push(op::CALL);
            uleb128(imp("get-int"), &mut inner);
            inner.push(op::I32_WRAP_I64);
            inner.push(op::CALL_INDIRECT);
            uleb128(lifted_tyi as u64, &mut inner);
            uleb128(0, &mut inner);
            set(rep, &mut inner);
            // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally).
            for ti in 0..gr.tuples.len() as u32 {
                emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
            }
            // OWN: drop the cell now. BORROW: host keeps it (repeatable), dtor reclaims — do NOT drop. The
            // transient compound handle `rep` is separate and dropped after the walk.
            if !call_borrow {
                get(cell, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            for hole in &template.leaves {
                emit_hole_fill(hole, byte_off, rep, scratch, &import_index, &mut inner);
            }
            get(rep, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            inner.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(ret_off as i64, &mut inner);
            inner.push(op::END);
        } else if gr.ret_is_bytes {
            let out_off = bytes_out_off as i64;
            let cell = 1 + arity;
            let bh = cell + 1;
            let nlen = bh + 1;
            let iv = nlen + 1;
            let tuple_local = iv + 1;
            let n_locals = 4 + gr.tuples.len() as u32; // cell/bh/n/i + one i32 per rebuilt tuple cell
            inner.extend_from_slice(&wasm_vec(1, &{
                let mut gl = uleb_bytes(n_locals as u64);
                gl.push(wasm_abi::CORE_I32);
                gl
            }));
            let get = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_GET);
                uleb128(l as u64, out);
            };
            let set = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_SET);
                uleb128(l as u64, out);
            };
            let ci32 = |v: i64, out: &mut Vec<u8>| {
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(v, out);
            };
            // cell = self (borrow: rep passed directly) or resource.rep-g(self) (own).
            get(0, &mut inner);
            if !call_borrow {
                inner.push(op::CALL);
                uleb128(rrep_fn[gi] as u64, &mut inner);
            }
            set(cell, &mut inner);
            get(cell, &mut inner);
            emit_closure_call_args(&gr.tuples, tuple_local, arity, &imp, &mut inner);
            get(cell, &mut inner);
            ci32(0, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("arr-get"), &mut inner);
            inner.push(op::CALL);
            uleb128(imp("get-int"), &mut inner);
            inner.push(op::I32_WRAP_I64);
            inner.push(op::CALL_INDIRECT);
            uleb128(lifted_tyi as u64, &mut inner);
            uleb128(0, &mut inner);
            set(bh, &mut inner);
            // Each rebuilt tuple-arg cell is an owned per-call temporary — drop it now (unconditionally).
            for ti in 0..gr.tuples.len() as u32 {
                emit_tuple_rebuilt_drop(tuple_local + ti, &imp, &mut inner);
            }
            // OWN: drop the cell now. BORROW: host keeps it (repeatable), dtor reclaims — do NOT drop. The
            // transient Bytes handle `bh` is separate and dropped after the copy.
            if !call_borrow {
                get(cell, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            get(bh, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-len"), &mut inner);
            set(nlen, &mut inner);
            ci32(0, &mut inner);
            set(iv, &mut inner);
            inner.push(op::BLOCK);
            inner.push(wasm_abi::BLOCK_EMPTY);
            inner.push(op::LOOP);
            inner.push(wasm_abi::BLOCK_EMPTY);
            get(iv, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_GE_U);
            inner.push(op::BR_IF);
            uleb128(1, &mut inner);
            ci32(out_off, &mut inner);
            get(iv, &mut inner);
            inner.push(op::I32_ADD);
            get(bh, &mut inner);
            get(iv, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-get"), &mut inner);
            inner.push(op::I32_STORE8);
            inner.push(0x00);
            inner.push(0x00);
            get(iv, &mut inner);
            ci32(1, &mut inner);
            inner.push(op::I32_ADD);
            set(iv, &mut inner);
            inner.push(op::BR);
            uleb128(0, &mut inner);
            inner.push(op::END);
            inner.push(op::END);
            ci32(bytes_ret_off as i64, &mut inner);
            ci32(out_off, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            ci32(bytes_ret_off as i64 + 4, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            get(bh, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            ci32(bytes_ret_off as i64, &mut inner);
            inner.push(op::END);
        } else {
            // A tuple-arg group needs one MORE i32 local per rebuilt tuple cell (the closure's compound args,
            // reassembled from the flattened field params, at `tuple_local + i`); a plain scalar group needs
            // just the closure-cell local.
            let cell_local = 1 + arity;
            let tuple_local = cell_local + 1;
            let sum_local = tuple_local + gr.tuples.len() as u32;
            let n_locals = 1 + gr.tuples.len() as u32 + gr.sums.len() as u32; // cell + one i32 per tuple/sum
            inner.extend_from_slice(&wasm_vec(1, &{
                let mut gl = uleb_bytes(n_locals as u64);
                gl.push(wasm_abi::CORE_I32);
                gl
            }));
            // cell = self (borrow: rep passed directly) or resource.rep-g(self) (own).
            inner.push(op::LOCAL_GET);
            uleb128(0, &mut inner);
            if !call_borrow {
                inner.push(op::CALL);
                uleb128(rrep_fn[gi] as u64, &mut inner);
            }
            inner.push(op::LOCAL_SET);
            uleb128(cell_local as u64, &mut inner);
            // push env (the cell) then the closure's args (rebuilt tuples/sums among the scalars).
            inner.push(op::LOCAL_GET);
            uleb128(cell_local as u64, &mut inner);
            emit_closure_call_args_with_sums(
                &gr.tuples,
                tuple_local,
                &gr.sums,
                sum_local,
                arity,
                &imp,
                &mut inner,
            );
            inner.push(op::LOCAL_GET);
            uleb128(cell_local as u64, &mut inner);
            inner.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(0, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("arr-get"), &mut inner);
            inner.push(op::CALL);
            uleb128(imp("get-int"), &mut inner);
            inner.push(op::I32_WRAP_I64);
            inner.push(op::CALL_INDIRECT);
            // The group's lifted `call_indirect` functype index in THIS core: the lifted bodies are the
            // trailing `funcs` after the `order` defs, so their functype sits at
            // `defined_type_base + order.len() + slot` (NOT `layout.lifted_type_index`, which bakes in the
            // multi-export `import_base`; the distinct-sig core has a different type layout).
            uleb128(lifted_tyi as u64, &mut inner);
            uleb128(0, &mut inner); // table 0
            // OWN: drop the cell after dispatch (own<t> consumed). BORROW: host keeps it (repeatable), the
            // dtor reclaims — do NOT drop.
            if !call_borrow {
                inner.push(op::LOCAL_GET);
                uleb128(cell_local as u64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            // Each rebuilt tuple-arg/sum-arg cell is an owned per-call temporary — drop it UNCONDITIONALLY
            // after dispatch (both own + borrow), balancing its `arr-alloc`/`sum-new`.
            for ti in 0..gr.tuples.len() as u32 {
                inner.push(op::LOCAL_GET);
                uleb128((tuple_local + ti) as u64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            for si in 0..gr.sums.len() as u32 {
                inner.push(op::LOCAL_GET);
                uleb128((sum_local + si) as u64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            inner.push(op::END);
        }
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // cabi_realloc stub (only when a list-returning group needs it).
    if any_list {
        let mut inner = uleb_bytes(0);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(
        wasm_abi::CORE_SEC_CODE,
        &wasm_vec(n + total_makes + g + usize::from(any_list), &code_items),
    );

    // ── Data section ── the compound groups' value-form templates + retareas (byte-rope groups write PAST
    // them at run time). Only present when a compound group laid template bytes.
    let data_sec = if data_bytes.is_empty() {
        Vec::new()
    } else {
        let mut item = vec![0x00];
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut item);
        item.push(op::END);
        item.extend_from_slice(&uleb_bytes(data_bytes.len() as u64));
        item.extend_from_slice(&data_bytes);
        section(wasm_abi::CORE_SEC_DATA, &wasm_vec(1, &item))
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    Ok(core)
}

/// One parameter of a round-trip CONSUMER export ([`ClosureConsume`]): either the closure RESOURCE being
/// handed back (its boundary handle is `resource.rep`'d to the guest cell before the consumer body runs)
/// or an ordinary SCALAR passed straight through.
#[derive(Clone, Copy)]
pub enum ConsumeParam {
    /// The closure resource — its core param is an i32 resource handle; the wrapper `resource.rep`s it to
    /// the guest cell the consumer body expects (which treats the closure as a plain cell handle).
    Closure,
    /// A scalar param — passed through unchanged.
    Scalar(ValType),
}

/// One CONSUMER export for the round-trip (C-HOST-4): a Cadenza export whose PARAMETER is a closure the
/// host hands back (`(def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x))`). The consumer BODY is
/// selected normally (its closure param is a plain cell handle, applied via `Core::CallClosure`); the
/// serializer emits a WRAPPER that `resource.rep`s each closure param (boundary handle → guest cell)
/// before calling the body — the exact mirror of how `make` wraps a producer body with `resource.new`.
#[derive(Clone)]
pub struct ClosureConsume {
    /// The boundary export name (verbatim source name, e.g. `apply-it`).
    pub export_name: String,
    /// The consumer BODY's core function index — the wrapper calls it with the rep'd cells + scalars.
    pub consume_abs: u32,
    /// The params in order — a `Closure` gets a `resource.rep`, a `Scalar` passes through.
    pub params: Vec<ConsumeParam>,
    /// The consumer's result valtype.
    pub ret_vt: ValType,
    /// True when the consumer's result is a byte-rope (`Bytes`/`String`) — the wrapper copies the body's
    /// returned handle out as a `list<u8>` `(ptr,len)` return area (via `bytes-len`/`bytes-get`) instead of
    /// returning the scalar value. `ret_vt` is `I32` either way (a bytes handle IS an i32) so the core
    /// consumer functype's result stays `i32`; only the BODY differs. When any consumer is byte-rope the
    /// module gains a shared memory + `cabi_realloc`.
    pub ret_is_bytes: bool,
    /// `Some(template)` when the consumer's result is a fixed-shape COMPOUND (tuple/record/sum) — the wrapper
    /// walks the body's returned handle into this consumer's value-form template region + returns its
    /// `(ptr,len)` retarea (the canonical value form as `list<u8>`). Mutually exclusive with `ret_is_bytes`
    /// (both cross as `list<u8>`; a compound writes the value form, a byte-rope the raw payload). Each
    /// compound consumer's template gets its own data-section region; byte-rope consumers write dynamically
    /// PAST all compound data. When any consumer crosses as `list<u8>` the module gains a memory +
    /// `cabi_realloc`.
    pub ret_template: Option<crate::lower::ValueFormTemplate>,
    /// `Some(descriptor)` when the consumer's result is a VARIABLE-LENGTH collection (List/Map/Set) — the
    /// wrapper renders the value form at run time via `value-encode(rep, desc)` against this consumer's shape
    /// descriptor (no static template), then copies the document out PAST all compound-template data
    /// (`bytes_out_off`). Mutually exclusive with `ret_is_bytes`/`ret_template`.
    pub ret_descriptor: Option<Vec<u8>>,
}

/// The ROUND-TRIP closure-resource core module (C-HOST-4): N producer `make-<name>` functions (as in
/// [`multi_closure_resource_core_module`]) PLUS M consumer exports that take a closure resource back and
/// apply it. Producers and consumers share the ONE resource type + funcref table (the closure the host
/// holds was lifted in THIS module, so a consumer's `call_indirect` resolves against the same in-program
/// lifted lambda by signature). Each consumer wrapper `resource.rep`s its closure param(s) to the guest
/// cell, then calls the consumer body. Layout: imports 0..k+2, n defined bodies, then N makes, then M
/// consumer wrappers (no shared `call` method — a round-trip program applies the closure via its OWN
/// consumer export, not a resource method).
#[allow(clippy::too_many_arguments)]
pub fn roundtrip_resource_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    makes: &[ClosureMake],
    consumers: &[ClosureConsume],
    plain: &[PlainExport],
    lifted_type_idx: u32,
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let nmk = makes.len();
    let ncons = consumers.len();
    // A consumer whose result crosses as `list<u8>` — a byte-rope (`ret_is_bytes`, raw payload) OR a
    // fixed-shape COMPOUND (`ret_template`, value form). Either makes the module need a shared memory +
    // `cabi_realloc`; the envelope lifts each such consumer with the Memory/Realloc canon options. A
    // compound consumer writes the VALUE FORM from its own data-section region; a byte-rope consumer writes a
    // runtime-length payload PAST all compound data so the two never collide.
    let consumer_is_list = |c: &ClosureConsume| {
        c.ret_is_bytes || c.ret_template.is_some() || c.ret_descriptor.is_some()
    };
    let any_list = consumers.iter().any(consumer_is_list);
    // Per COMPOUND consumer: place its template + `(ptr,len)` retarea in the data section (4-aligned),
    // record `(byte_off, ret_off)`. `bytes_ret_off`/`bytes_out_off` are past all compound data — where the
    // byte-rope consumers put their dynamic retarea + payload (only one consumer runs per host invocation).
    let mut data_bytes: Vec<u8> = Vec::new();
    let mut consumer_place: Vec<Option<(usize, usize)>> = Vec::with_capacity(ncons);
    for c in consumers {
        if let Some(t) = &c.ret_template {
            let byte_off = (data_bytes.len() + 3) & !3;
            data_bytes.resize(byte_off, 0);
            data_bytes.extend_from_slice(&t.bytes);
            let ret_off = (data_bytes.len() + 3) & !3;
            data_bytes.resize(ret_off, 0);
            data_bytes.extend_from_slice(&(byte_off as u32).to_le_bytes());
            data_bytes.extend_from_slice(&(t.bytes.len() as u32).to_le_bytes());
            consumer_place.push(Some((byte_off, ret_off)));
        } else {
            consumer_place.push(None);
        }
    }
    let bytes_ret_off = (data_bytes.len() + 3) & !3;
    let bytes_out_off = bytes_ret_off + 8;
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };
    // A consumer's boundary/core param valtype: a closure param crosses as an i32 resource handle; a
    // scalar as its own valtype.
    let consume_param_vt = |p: &ConsumeParam| match p {
        ConsumeParam::Closure => ValType::I32,
        ConsumeParam::Scalar(v) => *v,
    };

    // ── Type section ── import functypes 0..k, resource-new/rep (k, k+1), one per defined body, then one
    // make functype per make, then one consumer functype per consumer.
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
    type_items.extend_from_slice(&i32_to_i32); // resource-new (k)
    type_items.extend_from_slice(&i32_to_i32); // resource-rep (k+1)
    let defined_type_base = k + 2;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    let make_type_base = defined_type_base + n;
    for mk in makes {
        let params: Vec<u8> = mk.param_vts.iter().map(|v| vt_byte(*v)).collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let consume_type_base = make_type_base + nmk;
    for c in consumers {
        let params: Vec<u8> = c
            .params
            .iter()
            .map(|p| vt_byte(consume_param_vt(p)))
            .collect();
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(params.len(), &params));
        t.extend_from_slice(&wasm_vec(1, &[vt_byte(c.ret_vt)]));
        type_items.extend_from_slice(&t);
    }
    // If any consumer is byte-rope, one shared `cabi_realloc` functype `(i32×4)->i32` after the consumers.
    let realloc_type_idx = (defined_type_base + n + nmk + ncons) as u32;
    if any_list {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + nmk + ncons + usize::from(any_list);
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── k ops + resource-new + resource-rep.
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

    // ── Function section ── defined bodies, then makes, then consumer wrappers.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    for i in 0..nmk {
        uleb128((make_type_base + i) as u64, &mut func_items);
    }
    for i in 0..ncons {
        uleb128((consume_type_base + i) as u64, &mut func_items);
    }
    if any_list {
        uleb128(realloc_type_idx as u64, &mut func_items);
    }
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(n + nmk + ncons + usize::from(any_list), &func_items),
    );
    let make_abs_base = (defined_type_base + n) as u32;
    let consume_abs_base = make_abs_base + nmk as u32;
    let realloc_abs = consume_abs_base + ncons as u32; // valid only when any_list

    // ── Table + Element ── the funcref table from `layout.lifted` (a consumer's call_indirect dispatches
    // over it; the closure the host handed back was lifted in this module).
    let n_lifted = layout.lifted.len();
    let (table_sec, elem_sec) = if n_lifted == 0 {
        (Vec::new(), Vec::new())
    } else {
        let mut table_entry = vec![0x70u8, 0x01];
        uleb128(n_lifted as u64, &mut table_entry);
        uleb128(n_lifted as u64, &mut table_entry);
        let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        let elem_sec = section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg));
        (table_sec, elem_sec)
    };

    // ── Memory ── only when a byte-rope consumer must write its `list<u8>` payload.
    let mem_sec = if any_list {
        section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]))
    } else {
        Vec::new()
    };

    // ── Export section ── each make + each consumer, under its boundary name; plus (when byte-rope) `memory`
    // + `cabi_realloc` for the compound consumer's canon lift.
    let export_sec = {
        let export = |name: &str, kind: u8, idx: u32| {
            let mut item = uleb_bytes(name.len() as u64);
            item.extend_from_slice(name.as_bytes());
            item.push(kind);
            uleb128(idx as u64, &mut item);
            item
        };
        let mut items = Vec::new();
        for (i, mk) in makes.iter().enumerate() {
            items.extend_from_slice(&export(
                &mk.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                make_abs_base + i as u32,
            ));
        }
        for (i, c) in consumers.iter().enumerate() {
            items.extend_from_slice(&export(
                &c.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                consume_abs_base + i as u32,
            ));
        }
        // PLAIN (non-closure) exports ride along: their bodies are already defined funcs, so just name each
        // by its core-func index (the envelope aliases + lifts them as ordinary top-level component funcs).
        for p in plain {
            items.extend_from_slice(&export(
                &p.export_name,
                wasm_abi::EXPORT_KIND_FUNC,
                p.body_abs,
            ));
        }
        if any_list {
            items.extend_from_slice(&export("memory", wasm_abi::EXPORT_KIND_MEMORY, 0));
            items.extend_from_slice(&export(
                "cabi_realloc",
                wasm_abi::EXPORT_KIND_FUNC,
                realloc_abs,
            ));
        }
        section(
            wasm_abi::CORE_SEC_EXPORT,
            &wasm_vec(
                nmk + ncons + plain.len() + if any_list { 2 } else { 0 },
                &items,
            ),
        )
    };

    // ── Code section ── defined bodies, then make wrappers, then consumer wrappers.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    for mk in makes {
        let mut inner = uleb_bytes(0);
        for p in 0..mk.param_vts.len() {
            inner.push(op::LOCAL_GET);
            uleb128(p as u64, &mut inner);
        }
        inner.push(op::CALL);
        uleb128(mk.export_abs as u64, &mut inner);
        inner.push(op::CALL);
        uleb128(f_rnew as u64, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // consumer[i](params…) = consume_body(<rep'd closures / passthrough scalars>…). A CLOSURE param's
    // boundary handle is `resource.rep`'d to the guest cell (in a scratch local), then that cell is passed
    // to the body; a scalar param is forwarded straight. The consumer body treats its closure param(s) as
    // plain cell handles (a normal `Core::CallClosure`), so this wrapper is the boundary→cell bridge. A
    // SCALAR-result consumer leaves the body's value R on the stack; a BYTE-ROPE-result consumer instead
    // copies the body's returned Bytes/String handle out as a `list<u8>` `(ptr,len)` return area.
    let imp = |name: &str| import_index[name] as u64;
    for c in consumers {
        let nparams = c.params.len() as u32;
        // One i32 scratch local per closure param (holding the rep'd cell). A byte-rope consumer needs 3
        // MORE i32 scratch locals (the returned Bytes handle, its length, the copy index).
        let n_closures = c
            .params
            .iter()
            .filter(|p| matches!(p, ConsumeParam::Closure))
            .count();
        // Extra i32 scratch beyond the closure cells: a byte-rope consumer needs 3 (the returned handle, its
        // length, the copy index); a COMPOUND consumer needs 1 (the returned handle) + a SEPARATE i64 group
        // (the walk scratch); a COLLECTION consumer needs 5 (rep, desc, doc, n, i) for the value-encode.
        let extra_i32 = if c.ret_descriptor.is_some() {
            5
        } else if c.ret_is_bytes {
            3
        } else if c.ret_template.is_some() {
            1
        } else {
            0
        };
        let n_i32_scratch = n_closures + extra_i32;
        let want_i64 = c.ret_template.is_some();
        let mut inner = Vec::new();
        {
            let n_groups = usize::from(n_i32_scratch > 0) + usize::from(want_i64);
            let mut decls = Vec::new();
            if n_i32_scratch > 0 {
                let mut g = uleb_bytes(n_i32_scratch as u64);
                g.push(wasm_abi::CORE_I32);
                decls.extend_from_slice(&g);
            }
            if want_i64 {
                let mut g = uleb_bytes(1);
                g.push(wasm_abi::CORE_I64);
                decls.extend_from_slice(&g);
            }
            inner.extend_from_slice(&wasm_vec(n_groups, &decls));
        }
        // resource.rep each closure param into its scratch cell.
        let mut cell_slot = nparams;
        let mut cell_of: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for (i, p) in c.params.iter().enumerate() {
            if matches!(p, ConsumeParam::Closure) {
                inner.push(op::LOCAL_GET);
                uleb128(i as u64, &mut inner);
                inner.push(op::CALL);
                uleb128(f_rrep as u64, &mut inner);
                inner.push(op::LOCAL_SET);
                uleb128(cell_slot as u64, &mut inner);
                cell_of.insert(i as u32, cell_slot);
                cell_slot += 1;
            }
        }
        // push args in order: a closure param → its rep'd cell scratch; a scalar → its param local.
        for (i, p) in c.params.iter().enumerate() {
            inner.push(op::LOCAL_GET);
            match p {
                ConsumeParam::Closure => uleb128(cell_of[&(i as u32)] as u64, &mut inner),
                ConsumeParam::Scalar(_) => uleb128(i as u64, &mut inner),
            }
        }
        inner.push(op::CALL);
        uleb128(c.consume_abs as u64, &mut inner);
        if let Some(descriptor) = &c.ret_descriptor {
            // The body returned a COLLECTION HANDLE (on the stack). Save it in `rep`, drop the closure cells,
            // build the descriptor Bytes, value-encode(rep, desc) → the document, copy it out PAST all
            // compound-template data (`bytes_out_off`), release rep/desc/doc, return the retptr.
            let out_off = bytes_out_off as i64;
            let rep = cell_slot;
            let desc = cell_slot + 1;
            let doc = cell_slot + 2;
            let nlen = cell_slot + 3;
            let iv = cell_slot + 4;
            let get = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_GET);
                uleb128(l as u64, out);
            };
            let set = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_SET);
                uleb128(l as u64, out);
            };
            let ci32 = |v: i64, out: &mut Vec<u8>| {
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(v, out);
            };
            set(rep, &mut inner);
            for cell in cell_of.values() {
                get(*cell, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            // desc = bytes-alloc(len); bytes-set each constant descriptor byte.
            ci32(descriptor.len() as i64, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-alloc"), &mut inner);
            set(desc, &mut inner);
            for (j, &byte) in descriptor.iter().enumerate() {
                get(desc, &mut inner);
                ci32(j as i64, &mut inner);
                ci32(byte as i64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("bytes-set"), &mut inner);
                set(desc, &mut inner);
            }
            get(rep, &mut inner);
            get(desc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("value-encode"), &mut inner);
            set(doc, &mut inner);
            get(doc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-len"), &mut inner);
            set(nlen, &mut inner);
            ci32(0, &mut inner);
            set(iv, &mut inner);
            inner.push(op::BLOCK);
            inner.push(wasm_abi::BLOCK_EMPTY);
            inner.push(op::LOOP);
            inner.push(wasm_abi::BLOCK_EMPTY);
            get(iv, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_GE_U);
            inner.push(op::BR_IF);
            uleb128(1, &mut inner);
            ci32(out_off, &mut inner);
            get(iv, &mut inner);
            inner.push(op::I32_ADD);
            get(doc, &mut inner);
            get(iv, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-get"), &mut inner);
            inner.push(op::I32_STORE8);
            inner.push(0x00);
            inner.push(0x00);
            get(iv, &mut inner);
            ci32(1, &mut inner);
            inner.push(op::I32_ADD);
            set(iv, &mut inner);
            inner.push(op::BR);
            uleb128(0, &mut inner);
            inner.push(op::END);
            inner.push(op::END);
            ci32(bytes_ret_off as i64, &mut inner);
            ci32(out_off, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            ci32(bytes_ret_off as i64 + 4, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            get(rep, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            get(desc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            get(doc, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            ci32(bytes_ret_off as i64, &mut inner);
        } else if let Some(template) = &c.ret_template {
            // The body returned a COMPOUND HANDLE (on the stack). Save it in `rep`, drop the closure cells,
            // walk the handle into this consumer's value-form template region, drop the handle, return the
            // template's retarea pointer. `rep` is the i32 slot past the closure cells; `scratch` (i64) is
            // the last local (past all i32 scratch).
            let (byte_off, ret_off) =
                consumer_place[/* index */ consumers.iter().position(|x| std::ptr::eq(x, c)).unwrap()]
                    .expect("a compound consumer has a data placement");
            let rep = cell_slot;
            let scratch = nparams + n_i32_scratch as u32; // the i64 local (its own group, after all i32)
            let get = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_GET);
                uleb128(l as u64, out);
            };
            let set = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_SET);
                uleb128(l as u64, out);
            };
            set(rep, &mut inner);
            for cell in cell_of.values() {
                get(*cell, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            for hole in &template.leaves {
                emit_hole_fill(hole, byte_off, rep, scratch, &import_index, &mut inner);
            }
            get(rep, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            inner.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(ret_off as i64, &mut inner);
        } else if c.ret_is_bytes {
            // The body returned a Bytes/String HANDLE (on the stack). Save it, drop the closure cells
            // (own<t> release), then copy the byte-rope out as a `list<u8>` `(ptr,len)` return area — the
            // same copy loop the byte-rope `call` uses. `cell_slot` is past the params + closure cells; the
            // retarea/payload go PAST any compound consumers' template data (`bytes_ret_off`/`bytes_out_off`).
            let out_off = bytes_out_off as i64;
            let bh = cell_slot;
            let nlen = cell_slot + 1;
            let iv = cell_slot + 2;
            let get = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_GET);
                uleb128(l as u64, out);
            };
            let set = |l: u32, out: &mut Vec<u8>| {
                out.push(op::LOCAL_SET);
                uleb128(l as u64, out);
            };
            let ci32 = |v: i64, out: &mut Vec<u8>| {
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(v, out);
            };
            set(bh, &mut inner);
            // release the closure cells now that the body is done borrowing them.
            for cell in cell_of.values() {
                get(*cell, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
            get(bh, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-len"), &mut inner);
            set(nlen, &mut inner);
            ci32(0, &mut inner);
            set(iv, &mut inner);
            inner.push(op::BLOCK);
            inner.push(wasm_abi::BLOCK_EMPTY);
            inner.push(op::LOOP);
            inner.push(wasm_abi::BLOCK_EMPTY);
            get(iv, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_GE_U);
            inner.push(op::BR_IF);
            uleb128(1, &mut inner);
            ci32(out_off, &mut inner);
            get(iv, &mut inner);
            inner.push(op::I32_ADD);
            get(bh, &mut inner);
            get(iv, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("bytes-get"), &mut inner);
            inner.push(op::I32_STORE8);
            inner.push(0x00);
            inner.push(0x00);
            get(iv, &mut inner);
            ci32(1, &mut inner);
            inner.push(op::I32_ADD);
            set(iv, &mut inner);
            inner.push(op::BR);
            uleb128(0, &mut inner);
            inner.push(op::END);
            inner.push(op::END);
            ci32(bytes_ret_off as i64, &mut inner);
            ci32(out_off, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            ci32(bytes_ret_off as i64 + 4, &mut inner);
            get(nlen, &mut inner);
            inner.push(op::I32_STORE);
            inner.push(0x02);
            inner.push(0x00);
            get(bh, &mut inner);
            inner.push(op::CALL);
            uleb128(imp("drop"), &mut inner);
            ci32(bytes_ret_off as i64, &mut inner);
        } else {
            // C-HOST-5: each closure param crossed as `own<t>` (ownership transferred INTO the consumer), so
            // the wrapper owns each handed-back cell's last reference — RELEASE each now (`heap.drop(rep)`),
            // after the consumer BODY returned (it finished borrowing the cell for every `(g x)`
            // application, including a body that applies the closure more than once). The body's result R is
            // on the stack; each `drop` takes a rep (a separate push) and returns nothing, leaving R on top.
            for cell in cell_of.values() {
                inner.push(op::LOCAL_GET);
                uleb128(*cell as u64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
            }
        }
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    // cabi_realloc stub (only when a byte-rope consumer needs it).
    if any_list {
        let mut inner = uleb_bytes(0);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(
        wasm_abi::CORE_SEC_CODE,
        &wasm_vec(n + nmk + ncons + usize::from(any_list), &code_items),
    );

    // ── Data section ── the compound consumers' value-form templates + retareas (byte-rope consumers write
    // PAST them at run time). Only present when a compound consumer laid template bytes.
    let data_sec = if data_bytes.is_empty() {
        Vec::new()
    } else {
        let mut item = vec![0x00];
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut item);
        item.push(op::END);
        item.extend_from_slice(&uleb_bytes(data_bytes.len() as u64));
        item.extend_from_slice(&data_bytes);
        section(wasm_abi::CORE_SEC_DATA, &wasm_vec(1, &item))
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    let _ = lifted_type_idx; // reserved: a consumer's call_indirect type resolves in its selected body
    Ok(core)
}

/// One SIGNATURE GROUP for the DISTINCT-SIGNATURE ROUND-TRIP ([`distinct_sig_roundtrip_core_module`]): all
/// producers + consumers of ONE closure signature `(-> A… R)` → one resource type with its own
/// `resource-new-<g>`/`resource-rep-<g>` intrinsics. `makes` mint the closure (each `resource.new-<g>`);
/// `consumers` take one back (each closure param `resource.rep-<g>`'d to the guest cell). Unlike a pure
/// producer group ([`SigGroup`]), a round-trip group has NO shared `call-<g>` method — the closure is
/// applied via the consumers' OWN bodies.
pub struct RtSigGroup {
    pub makes: Vec<ClosureMake>,
    pub consumers: Vec<ClosureConsume>,
}

/// The DISTINCT-SIGNATURE ROUND-TRIP core module: closures of G different signatures each cross as their
/// own resource type, and each group has PRODUCERS (`make-<name>`) AND CONSUMERS (named exports that take a
/// closure of that signature back and apply it). The union of `distinct_sig_resource_core_module` (per-group
/// resource intrinsics + makes) and `roundtrip_resource_core_module` (consumer wrappers): per group `g`, a
/// `resource-new-<g>`/`resource-rep-<g>` pair, its makes (using `new-<g>`), and its consumer wrappers (each
/// `resource.rep-<g>`'ing its closure param(s) → the guest cell, calling the body, then dropping the cell).
/// All groups share the ONE guest funcref table; a consumer's `call_indirect` resolves in its selected body.
/// Layout: imports 0..k + 2*G intrinsics; n defined bodies; then per group its makes then its consumers.
pub fn distinct_sig_roundtrip_core_module(
    funcs: &[SelectedFunc],
    imports: &[&RtOp],
    groups: &[RtSigGroup],
    plain: &[PlainExport],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    use crate::backend::wasm::wasm_abi::op;
    let k = imports.len();
    let n = funcs.len();
    let g = groups.len();
    let total_makes: usize = groups.iter().map(|gr| gr.makes.len()).sum();
    let total_cons: usize = groups.iter().map(|gr| gr.consumers.len()).sum();
    // A consumer whose result crosses as `list<u8>` — a byte-rope (`ret_is_bytes`, raw payload), a
    // fixed-shape COMPOUND (`ret_template`, value form), OR a VARIABLE-LENGTH collection (`ret_descriptor`,
    // value form via `value-encode`). Any makes the module need a shared memory + `cabi_realloc`. A compound
    // consumer writes the VALUE FORM from its own data-section region; a byte-rope/collection consumer writes
    // a runtime-length payload PAST all compound data so they never collide. Per COMPOUND consumer (flat group
    // order — makes then consumers per group) record its `(byte_off, ret_off)`.
    let consumer_is_list = |c: &ClosureConsume| {
        c.ret_is_bytes || c.ret_template.is_some() || c.ret_descriptor.is_some()
    };
    let any_list = groups
        .iter()
        .any(|gr| gr.consumers.iter().any(consumer_is_list));
    let mut data_bytes: Vec<u8> = Vec::new();
    let mut consumer_place: Vec<Option<(usize, usize)>> = Vec::new();
    for gr in groups {
        for c in &gr.consumers {
            if let Some(t) = &c.ret_template {
                let byte_off = (data_bytes.len() + 3) & !3;
                data_bytes.resize(byte_off, 0);
                data_bytes.extend_from_slice(&t.bytes);
                let ret_off = (data_bytes.len() + 3) & !3;
                data_bytes.resize(ret_off, 0);
                data_bytes.extend_from_slice(&(byte_off as u32).to_le_bytes());
                data_bytes.extend_from_slice(&(t.bytes.len() as u32).to_le_bytes());
                consumer_place.push(Some((byte_off, ret_off)));
            } else {
                consumer_place.push(None);
            }
        }
    }
    let bytes_ret_off = (data_bytes.len() + 3) & !3;
    let bytes_out_off = bytes_ret_off + 8;
    let vt_byte = |v: ValType| match v {
        ValType::I32 => wasm_abi::CORE_I32,
        ValType::I64 => wasm_abi::CORE_I64,
        ValType::F32 => wasm_abi::CORE_F32,
        ValType::F64 => wasm_abi::CORE_F64,
    };
    let consume_param_vt = |p: &ConsumeParam| match p {
        ConsumeParam::Closure => ValType::I32,
        ConsumeParam::Scalar(v) => *v,
    };

    // ── Type section ── import functypes 0..k; one shared `(i32)->i32` rintr functype (index k); one per
    // defined body; then per group its make functype(s) + consumer functype(s), flat in group order.
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
    // Emit 2*G resource-intrinsic functypes (all identical `(i32)->i32`, one per `resource-new-<g>`/
    // `resource-rep-<g>` import), so `defined_type_base = k + 2*G = import_count`. This ALIGNS the type
    // layout with `import_base` (= k + 2*G), which is what a SELECTED consumer body assumes when it embeds
    // `call_indirect(layout.lifted_type_index(slot, import_base))` — otherwise the consumer's indirect-call
    // type index would be off by 2*G-1 (a "function failed to validate" in wasmtime). The rintr imports all
    // reference `rintr_type_idx = k` (the first of these), which is fine — they are all the same shape.
    for _ in 0..(2 * g) {
        type_items.extend_from_slice(&i32_to_i32);
    }
    let rintr_type_idx = k as u32;
    let defined_type_base = k + 2 * g;
    for f in funcs {
        type_items.extend_from_slice(&functype(f)?);
    }
    // ALL FUNCTION SECTIONS USE ONE ORDER: per group, its makes then its consumers (matching the envelope's
    // per-group alias order). `fn_type_idx` records each function's core type index in that exact flat
    // order, so the function/export/code sections stay consistent (an earlier cut interleaved the type build
    // per-group but listed functions makes-flat-then-consumers-flat → the envelope's per-group aliases got
    // the wrong functype).
    let mut fn_type_idx: Vec<u32> = Vec::new();
    let mut next_type = defined_type_base + n;
    for gr in groups {
        for mk in &gr.makes {
            let params: Vec<u8> = mk.param_vts.iter().map(|v| vt_byte(*v)).collect();
            let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
            t.extend_from_slice(&wasm_vec(params.len(), &params));
            t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
            type_items.extend_from_slice(&t);
            fn_type_idx.push(next_type as u32);
            next_type += 1;
        }
        for c in &gr.consumers {
            let params: Vec<u8> = c
                .params
                .iter()
                .map(|p| vt_byte(consume_param_vt(p)))
                .collect();
            let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
            t.extend_from_slice(&wasm_vec(params.len(), &params));
            t.extend_from_slice(&wasm_vec(1, &[vt_byte(c.ret_vt)]));
            type_items.extend_from_slice(&t);
            fn_type_idx.push(next_type as u32);
            next_type += 1;
        }
    }
    // If any consumer is byte-rope, one shared `cabi_realloc` functype `(i32×4)->i32` after the group fns.
    let realloc_type_idx = next_type as u32;
    if any_list {
        let mut t = vec![wasm_abi::CORE_FUNCTYPE_FORM];
        t.extend_from_slice(&wasm_vec(4, &[wasm_abi::CORE_I32; 4]));
        t.extend_from_slice(&wasm_vec(1, &[wasm_abi::CORE_I32]));
        type_items.extend_from_slice(&t);
    }
    let total_types = defined_type_base + n + total_makes + total_cons + usize::from(any_list);
    let type_sec = section(wasm_abi::CORE_SEC_TYPE, &wasm_vec(total_types, &type_items));

    // ── Import section ── k ops + per group `resource-new-<g>`/`resource-rep-<g>`.
    let mut import_index: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    let mut import_items = Vec::new();
    for (i, o) in imports.iter().enumerate() {
        import_items.extend_from_slice(&import_item(o.name, i as u32));
        import_index.insert(o.name, i as u32);
    }
    let mut rnew_fn: Vec<u32> = Vec::new();
    let mut rrep_fn: Vec<u32> = Vec::new();
    let mut next_import_fn = k as u32;
    for gi in 0..g {
        import_items.extend_from_slice(&import_item(&format!("resource-new-{gi}"), rintr_type_idx));
        rnew_fn.push(next_import_fn);
        next_import_fn += 1;
        import_items.extend_from_slice(&import_item(&format!("resource-rep-{gi}"), rintr_type_idx));
        rrep_fn.push(next_import_fn);
        next_import_fn += 1;
    }
    let import_sec = section(2, &wasm_vec(k + 2 * g, &import_items));

    // ── Function section ── defined bodies, then the group functions in per-group order (`fn_type_idx`),
    // then (when byte-rope) the shared cabi_realloc.
    let mut func_items = Vec::new();
    for i in 0..n {
        uleb128((defined_type_base + i) as u64, &mut func_items);
    }
    for &ti in &fn_type_idx {
        uleb128(ti as u64, &mut func_items);
    }
    if any_list {
        uleb128(realloc_type_idx as u64, &mut func_items);
    }
    let func_sec = section(
        wasm_abi::CORE_SEC_FUNCTION,
        &wasm_vec(
            n + total_makes + total_cons + usize::from(any_list),
            &func_items,
        ),
    );
    let import_count = k + 2 * g;
    // The group functions start at core-func `import_count + n`, in per-group (makes then consumers) order.
    let group_fn_abs_base = (import_count + n) as u32;
    let realloc_abs = group_fn_abs_base + (total_makes + total_cons) as u32; // valid only when any_list

    // ── Memory ── only when a byte-rope consumer must write its `list<u8>` payload.
    let mem_sec = if any_list {
        section(wasm_abi::CORE_SEC_MEMORY, &wasm_vec(1, &[0x00, 0x01]))
    } else {
        Vec::new()
    };

    // ── Table + Element ── the ONE funcref table (all groups' lifteds share it).
    let n_lifted = layout.lifted.len();
    let (table_sec, elem_sec) = if n_lifted == 0 {
        (Vec::new(), Vec::new())
    } else {
        let mut table_entry = vec![0x70u8, 0x01];
        uleb128(n_lifted as u64, &mut table_entry);
        uleb128(n_lifted as u64, &mut table_entry);
        let table_sec = section(wasm_abi::CORE_SEC_TABLE, &wasm_vec(1, &table_entry));
        let mut seg = Vec::new();
        seg.push(0x00);
        seg.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut seg);
        seg.push(op::END);
        let mut idxs = Vec::new();
        for slot in 0..n_lifted {
            uleb128(layout.lifted_abs(slot) as u64, &mut idxs);
        }
        seg.extend_from_slice(&wasm_vec(n_lifted, &idxs));
        let elem_sec = section(wasm_abi::CORE_SEC_ELEMENT, &wasm_vec(1, &seg));
        (table_sec, elem_sec)
    };

    // ── Export section ── per group: each make (`make-<name>`) then each consumer (its export name); plus
    // (when byte-rope) `memory` + `cabi_realloc` for the compound consumer's canon lift.
    let export_sec = {
        let export = |name: &str, kind: u8, idx: u32| {
            let mut item = uleb_bytes(name.len() as u64);
            item.extend_from_slice(name.as_bytes());
            item.push(kind);
            let mut b = item;
            uleb128(idx as u64, &mut b);
            b
        };
        let func_export = |name: &str, idx: u32| export(name, wasm_abi::EXPORT_KIND_FUNC, idx);
        let mut items = Vec::new();
        let mut fi = 0u32; // running per-group function index (makes then consumers per group)
        for gr in groups {
            for mk in &gr.makes {
                items.extend_from_slice(&func_export(&mk.export_name, group_fn_abs_base + fi));
                fi += 1;
            }
            for c in &gr.consumers {
                items.extend_from_slice(&func_export(&c.export_name, group_fn_abs_base + fi));
                fi += 1;
            }
        }
        // PLAIN (non-closure) exports ride along: their bodies are already defined funcs, exported by index.
        for p in plain {
            items.extend_from_slice(&func_export(&p.export_name, p.body_abs));
        }
        if any_list {
            items.extend_from_slice(&export("memory", wasm_abi::EXPORT_KIND_MEMORY, 0));
            items.extend_from_slice(&func_export("cabi_realloc", realloc_abs));
        }
        section(
            wasm_abi::CORE_SEC_EXPORT,
            &wasm_vec(
                total_makes + total_cons + plain.len() + if any_list { 2 } else { 0 },
                &items,
            ),
        )
    };

    // ── Code section ── defined bodies, then PER GROUP its makes (using group's rnew) then its consumers
    // (group's rrep) — the SAME per-group order as the function/export sections.
    let mut code_items = Vec::new();
    for f in funcs {
        code_items.extend_from_slice(&code_entry(f, &import_index));
    }
    // `flat_cons` indexes `consumer_place` (consumers in group order); advances once per consumer emitted.
    let mut flat_cons = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        // this group's makes.
        for mk in &gr.makes {
            let mut inner = uleb_bytes(0);
            for p in 0..mk.param_vts.len() {
                inner.push(op::LOCAL_GET);
                uleb128(p as u64, &mut inner);
            }
            inner.push(op::CALL);
            uleb128(mk.export_abs as u64, &mut inner);
            inner.push(op::CALL);
            uleb128(rnew_fn[gi] as u64, &mut inner);
            inner.push(op::END);
            let mut e = uleb_bytes(inner.len() as u64);
            e.extend_from_slice(&inner);
            code_items.extend_from_slice(&e);
        }
        // this group's consumers — each closure param rep'd via THIS group's rrep, then dropped. A SCALAR
        // consumer leaves the body's value on the stack; a BYTE-ROPE consumer copies the body's returned
        // handle out as a `list<u8>` `(ptr,len)` area; a COMPOUND consumer walks the returned handle into its
        // value-form template region (same bodies as the single-sig round-trip). `flat_cons` indexes
        // `consumer_place` (built in group order, consumers only).
        let imp = |name: &str| import_index[name] as u64;
        for c in &gr.consumers {
            let nparams = c.params.len() as u32;
            let n_closures = c
                .params
                .iter()
                .filter(|p| matches!(p, ConsumeParam::Closure))
                .count();
            // Extra i32 scratch beyond the closure cells: a COLLECTION consumer needs 5 (rep, desc, doc, n, i)
            // for the value-encode; a byte-rope needs 3 (handle, len, index); a compound needs 1 (handle) + a
            // SEPARATE i64 group (walk scratch).
            let extra_i32 = if c.ret_descriptor.is_some() {
                5
            } else if c.ret_is_bytes {
                3
            } else if c.ret_template.is_some() {
                1
            } else {
                0
            };
            let n_i32_scratch = n_closures + extra_i32;
            let want_i64 = c.ret_template.is_some();
            let mut inner = Vec::new();
            {
                let n_groups = usize::from(n_i32_scratch > 0) + usize::from(want_i64);
                let mut decls = Vec::new();
                if n_i32_scratch > 0 {
                    let mut gl = uleb_bytes(n_i32_scratch as u64);
                    gl.push(wasm_abi::CORE_I32);
                    decls.extend_from_slice(&gl);
                }
                if want_i64 {
                    let mut gl = uleb_bytes(1);
                    gl.push(wasm_abi::CORE_I64);
                    decls.extend_from_slice(&gl);
                }
                inner.extend_from_slice(&wasm_vec(n_groups, &decls));
            }
            let mut cell_slot = nparams;
            let mut cell_of: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            for (i, p) in c.params.iter().enumerate() {
                if matches!(p, ConsumeParam::Closure) {
                    inner.push(op::LOCAL_GET);
                    uleb128(i as u64, &mut inner);
                    inner.push(op::CALL);
                    uleb128(rrep_fn[gi] as u64, &mut inner);
                    inner.push(op::LOCAL_SET);
                    uleb128(cell_slot as u64, &mut inner);
                    cell_of.insert(i as u32, cell_slot);
                    cell_slot += 1;
                }
            }
            for (i, p) in c.params.iter().enumerate() {
                inner.push(op::LOCAL_GET);
                match p {
                    ConsumeParam::Closure => uleb128(cell_of[&(i as u32)] as u64, &mut inner),
                    ConsumeParam::Scalar(_) => uleb128(i as u64, &mut inner),
                }
            }
            inner.push(op::CALL);
            uleb128(c.consume_abs as u64, &mut inner);
            let place = consumer_place[flat_cons];
            flat_cons += 1;
            if let Some(descriptor) = &c.ret_descriptor {
                // Collection: the body returned a COLLECTION HANDLE (on the stack). Save it in `rep`, drop the
                // closure cells, build the descriptor Bytes, value-encode(rep, desc) → the document, copy it
                // out PAST all compound-template data (`bytes_out_off`), release rep/desc/doc, return the
                // retptr. Same body as the single-sig round-trip's collection consumer.
                let out_off = bytes_out_off as i64;
                let rep = cell_slot;
                let desc = cell_slot + 1;
                let doc = cell_slot + 2;
                let nlen = cell_slot + 3;
                let iv = cell_slot + 4;
                let get = |l: u32, out: &mut Vec<u8>| {
                    out.push(op::LOCAL_GET);
                    uleb128(l as u64, out);
                };
                let set = |l: u32, out: &mut Vec<u8>| {
                    out.push(op::LOCAL_SET);
                    uleb128(l as u64, out);
                };
                let ci32 = |v: i64, out: &mut Vec<u8>| {
                    out.push(op::I32_CONST);
                    crate::backend::wasm::encode::sleb128(v, out);
                };
                set(rep, &mut inner);
                for cell in cell_of.values() {
                    get(*cell, &mut inner);
                    inner.push(op::CALL);
                    uleb128(imp("drop"), &mut inner);
                }
                // desc = bytes-alloc(len); bytes-set each constant descriptor byte.
                ci32(descriptor.len() as i64, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("bytes-alloc"), &mut inner);
                set(desc, &mut inner);
                for (j, &byte) in descriptor.iter().enumerate() {
                    get(desc, &mut inner);
                    ci32(j as i64, &mut inner);
                    ci32(byte as i64, &mut inner);
                    inner.push(op::CALL);
                    uleb128(imp("bytes-set"), &mut inner);
                    set(desc, &mut inner);
                }
                get(rep, &mut inner);
                get(desc, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("value-encode"), &mut inner);
                set(doc, &mut inner);
                get(doc, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("bytes-len"), &mut inner);
                set(nlen, &mut inner);
                ci32(0, &mut inner);
                set(iv, &mut inner);
                inner.push(op::BLOCK);
                inner.push(wasm_abi::BLOCK_EMPTY);
                inner.push(op::LOOP);
                inner.push(wasm_abi::BLOCK_EMPTY);
                get(iv, &mut inner);
                get(nlen, &mut inner);
                inner.push(op::I32_GE_U);
                inner.push(op::BR_IF);
                uleb128(1, &mut inner);
                ci32(out_off, &mut inner);
                get(iv, &mut inner);
                inner.push(op::I32_ADD);
                get(doc, &mut inner);
                get(iv, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("bytes-get"), &mut inner);
                inner.push(op::I32_STORE8);
                inner.push(0x00);
                inner.push(0x00);
                get(iv, &mut inner);
                ci32(1, &mut inner);
                inner.push(op::I32_ADD);
                set(iv, &mut inner);
                inner.push(op::BR);
                uleb128(0, &mut inner);
                inner.push(op::END);
                inner.push(op::END);
                ci32(bytes_ret_off as i64, &mut inner);
                ci32(out_off, &mut inner);
                inner.push(op::I32_STORE);
                inner.push(0x02);
                inner.push(0x00);
                ci32(bytes_ret_off as i64 + 4, &mut inner);
                get(nlen, &mut inner);
                inner.push(op::I32_STORE);
                inner.push(0x02);
                inner.push(0x00);
                get(rep, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
                get(desc, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
                get(doc, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
                ci32(bytes_ret_off as i64, &mut inner);
            } else if let Some(template) = &c.ret_template {
                // Compound: save the returned handle in `rep`, drop the closure cells, walk the handle into
                // this consumer's value-form template region, drop the handle, return the retarea pointer.
                let (byte_off, ret_off) = place.expect("a compound consumer has a data placement");
                let rep = cell_slot;
                let scratch = nparams + n_i32_scratch as u32; // the i64 local (own group, after all i32)
                let get = |l: u32, out: &mut Vec<u8>| {
                    out.push(op::LOCAL_GET);
                    uleb128(l as u64, out);
                };
                let set = |l: u32, out: &mut Vec<u8>| {
                    out.push(op::LOCAL_SET);
                    uleb128(l as u64, out);
                };
                set(rep, &mut inner);
                for cell in cell_of.values() {
                    get(*cell, &mut inner);
                    inner.push(op::CALL);
                    uleb128(imp("drop"), &mut inner);
                }
                for hole in &template.leaves {
                    emit_hole_fill(hole, byte_off, rep, scratch, &import_index, &mut inner);
                }
                get(rep, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
                inner.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(ret_off as i64, &mut inner);
            } else if c.ret_is_bytes {
                let out_off = bytes_out_off as i64;
                let bh = cell_slot;
                let nlen = cell_slot + 1;
                let iv = cell_slot + 2;
                let get = |l: u32, out: &mut Vec<u8>| {
                    out.push(op::LOCAL_GET);
                    uleb128(l as u64, out);
                };
                let set = |l: u32, out: &mut Vec<u8>| {
                    out.push(op::LOCAL_SET);
                    uleb128(l as u64, out);
                };
                let ci32 = |v: i64, out: &mut Vec<u8>| {
                    out.push(op::I32_CONST);
                    crate::backend::wasm::encode::sleb128(v, out);
                };
                set(bh, &mut inner);
                for cell in cell_of.values() {
                    get(*cell, &mut inner);
                    inner.push(op::CALL);
                    uleb128(imp("drop"), &mut inner);
                }
                get(bh, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("bytes-len"), &mut inner);
                set(nlen, &mut inner);
                ci32(0, &mut inner);
                set(iv, &mut inner);
                inner.push(op::BLOCK);
                inner.push(wasm_abi::BLOCK_EMPTY);
                inner.push(op::LOOP);
                inner.push(wasm_abi::BLOCK_EMPTY);
                get(iv, &mut inner);
                get(nlen, &mut inner);
                inner.push(op::I32_GE_U);
                inner.push(op::BR_IF);
                uleb128(1, &mut inner);
                ci32(out_off, &mut inner);
                get(iv, &mut inner);
                inner.push(op::I32_ADD);
                get(bh, &mut inner);
                get(iv, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("bytes-get"), &mut inner);
                inner.push(op::I32_STORE8);
                inner.push(0x00);
                inner.push(0x00);
                get(iv, &mut inner);
                ci32(1, &mut inner);
                inner.push(op::I32_ADD);
                set(iv, &mut inner);
                inner.push(op::BR);
                uleb128(0, &mut inner);
                inner.push(op::END);
                inner.push(op::END);
                ci32(bytes_ret_off as i64, &mut inner);
                ci32(out_off, &mut inner);
                inner.push(op::I32_STORE);
                inner.push(0x02);
                inner.push(0x00);
                ci32(bytes_ret_off as i64 + 4, &mut inner);
                get(nlen, &mut inner);
                inner.push(op::I32_STORE);
                inner.push(0x02);
                inner.push(0x00);
                get(bh, &mut inner);
                inner.push(op::CALL);
                uleb128(imp("drop"), &mut inner);
                ci32(bytes_ret_off as i64, &mut inner);
            } else {
                for cell in cell_of.values() {
                    inner.push(op::LOCAL_GET);
                    uleb128(*cell as u64, &mut inner);
                    inner.push(op::CALL);
                    uleb128(imp("drop"), &mut inner);
                }
            }
            inner.push(op::END);
            let mut e = uleb_bytes(inner.len() as u64);
            e.extend_from_slice(&inner);
            code_items.extend_from_slice(&e);
        }
    }
    // cabi_realloc stub (only when a byte-rope consumer needs it).
    if any_list {
        let mut inner = uleb_bytes(0);
        inner.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut inner);
        inner.push(op::END);
        let mut e = uleb_bytes(inner.len() as u64);
        e.extend_from_slice(&inner);
        code_items.extend_from_slice(&e);
    }
    let code_sec = section(
        wasm_abi::CORE_SEC_CODE,
        &wasm_vec(
            n + total_makes + total_cons + usize::from(any_list),
            &code_items,
        ),
    );

    // ── Data section ── the compound consumers' value-form templates + retareas (byte-rope consumers write
    // PAST them at run time). Only present when a compound consumer laid template bytes.
    let data_sec = if data_bytes.is_empty() {
        Vec::new()
    } else {
        let mut item = vec![0x00];
        item.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(0, &mut item);
        item.push(op::END);
        item.extend_from_slice(&uleb_bytes(data_bytes.len() as u64));
        item.extend_from_slice(&data_bytes);
        section(wasm_abi::CORE_SEC_DATA, &wasm_vec(1, &item))
    };

    let mut core = Vec::new();
    core.extend_from_slice(CORE_MAGIC);
    core.extend_from_slice(&type_sec);
    core.extend_from_slice(&import_sec);
    core.extend_from_slice(&func_sec);
    core.extend_from_slice(&table_sec);
    core.extend_from_slice(&mem_sec);
    core.extend_from_slice(&export_sec);
    core.extend_from_slice(&elem_sec);
    core.extend_from_slice(&code_sec);
    core.extend_from_slice(&data_sec);
    Ok(core)
}

/// The STRICT component-boundary valtype of a type (`None` = unit / no result) — read directly for a
/// PARAMETER (where only an exactly-representable width is admitted) and as the base mapping `export_result`
/// widens over. A type with no boundary representation is an error here: a NON-ALIASED integer width
/// (`(UInt 48)`, …) is internal-only, so a PARAMETER of one DECLINES (naming the width) rather than
/// accepting an incoming wider value the guest cannot verify fits the narrower width. A non-aliased
/// RESULT, by contrast, crosses WIDENED to the next aliased width — that value-preserving relaxation lives
/// in [`export_result`], not here (it is unsound for a parameter).
///
/// Returning `Err` for a type with no boundary form is how the compiler keeps such a type out of an
/// emitted signature — it declines rather than emit an interface naming an unrepresentable type:
///
//= spec/contracts/component-abi.md#every-exported-type-has-a-stable-boundary-representation
//# A type that has no defined boundary representation MUST NOT appear in an exported or imported signature.
///
/// A result crosses as its PROPER component type — a scalar as its faithful component primitive (`s64`/
/// `u8`/`bool`/…), a compound as the typed `list<u8>` of its canonical value form — never collapsed to a
/// dynamically-tagged or stringly-typed value, so the exported boundary is strictly, statically typed:
//= spec/capabilities/self-hosting-surface.md#the-result-crosses-the-boundary-as-its-proper-type
//# A compiled program's entry MUST export its result as the result's proper component type, so that the boundary is strictly, statically typed rather than a dynamically-tagged value.
//= spec/capabilities/self-hosting-surface.md#the-result-crosses-the-boundary-as-its-proper-type
//# The compiler MUST NOT collapse a typed result to an untyped string at the boundary in place of the result's proper component type, so that static typing is enforced at the boundary rather than deferred to a stringly-typed convention.
pub fn export_result_valtype(ret: &Ty, ncx: &crate::ty::NameCtx) -> Result<Option<u8>, String> {
    // A NOMINAL newtype's boundary form is its ERASED underlying type (the tag adds nothing to the
    // representation): peel it so a nominal-over-scalar crosses as its scalar, and a nominal-over-compound
    // reaching HERE (a multi-export/parameterized position) declines as the underlying compound would.
    let ret = ret.strip_nominal();
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
            ret.render_name(ncx)
        )),
        // A sum crosses ONLY via the single-nullary-export escape path (handled in `emit` before this is
        // reached); a sum in any OTHER boundary position — a multi-export result, a parameter — has no
        // scalar boundary form and declines here (the escape's disc-switch renderer applies only to the
        // lone-export case).
        Ty::Sum { .. } => Err(format!(
            "a `{}` sum crosses the host boundary only as a single nullary export's result",
            ret.render_name(ncx)
        )),
        other => match comp_valtype_of(other) {
            Some(b) => Ok(Some(b)),
            None => Err(format!(
                "type `{}` has no component boundary representation (only the aliased integer widths \
                 8/16/32/64 cross the boundary)",
                other.render_name(ncx)
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
pub fn export_result(
    ret: &Ty,
    ncx: &crate::ty::NameCtx,
) -> Result<crate::backend::wasm::envelope::BoundaryResult, String> {
    use crate::backend::wasm::envelope::BoundaryResult;
    // A non-aliased integer width has no component primitive of its own, so `export_result_valtype`
    // declines it — but a RESULT we PRODUCE is guaranteed in range (its own arithmetic/`wrap` fold
    // computed it at width N), so it can cross faithfully WIDENED to the smallest aliased width ≥ N of the
    // SAME signedness (`(UInt 48)`→`u64`, `(Int 24)`→`s32`). This is value-preserving: N and its widened
    // width sit on the same side of the 32-bit core-slot boundary (8/16/32 → i32, 33..=64 → i64), so the
    // core function already returns the exact slot the canonical ABI lifts to that aliased primitive (a
    // non-negative unsigned value, or a sign-extended signed one). This is RESULT-ONLY: a non-aliased
    // ARGUMENT still declines (`export_result_valtype`, the param path) — accepting one would mean trusting
    // an incoming wider value fits the narrower declared width, which the guest cannot verify.
    if let Ty::Int(it) = ret.strip_nominal()
        && comp_valtype_of(ret).is_none()
        && let Some(b) = widened_result_int_comp_byte(it.ground_signed(), it.ground_width())
    {
        return Ok(BoundaryResult::Primitive(b));
    }
    match export_result_valtype(ret, ncx) {
        Ok(None) => Ok(BoundaryResult::None),
        Ok(Some(b)) => Ok(BoundaryResult::Primitive(b)),
        Err(e) => Err(e),
    }
}

/// The component primitive byte a NON-ALIASED integer RESULT of `(signed, width)` crosses as: the
/// smallest aliased width ≥ `width` of the SAME signedness (7→8, 24→32, 48→64…). `None` for an aliased
/// width (handled by `comp_valtype_of`) or an out-of-range width (`0`/`>64`, already CDZ0302 at
/// type-build). Value-preserving for a produced result — see `export_result`.
fn widened_result_int_comp_byte(signed: bool, width: u32) -> Option<u8> {
    if width == 0 || width > 64 {
        return None;
    }
    let aliased = match width {
        1..=8 => 8,
        9..=16 => 16,
        17..=32 => 32,
        _ => 64,
    };
    comp_valtype_of(&Ty::Int(crate::ty::IntTy::fixed(signed, aliased)))
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

#[cfg(test)]
mod host_import_functype_tests {
    use super::*;
    use crate::backend::wasm::host::{HostImport, HostParam, RecordFieldAbi};
    use crate::backend::wasm::runtime_abi::AbiValType;

    fn imp(params: Vec<HostParam>, result: Option<AbiValType>) -> HostImport {
        HostImport {
            effect: "state".into(),
            op: "op".into(),
            params,
            result,
            spilled_result: None,
            enum_result: None,
        }
    }

    // Shape d (core flatten): a RECORD param flattens to one core slot per field, in field order — a
    // scalar+bytes+scalar analogue but here all scalar. `record { a: s64, b: bool }` → core params
    // `(i64, i32)`. The CORE functype is `0x60 <params-vec> <results-vec>`; a `Unit` result → empty
    // results vec. This pins the flatten so a regression in `host_import_functype`'s Record arm is caught.
    #[test]
    fn a_record_param_flattens_to_one_core_slot_per_field() {
        let f = imp(
            vec![HostParam::Record(vec![
                ("a".into(), RecordFieldAbi::Scalar(AbiValType::S64)),
                ("b".into(), RecordFieldAbi::Scalar(AbiValType::Bool)),
            ])],
            None,
        );
        // 0x60 form; params = vec(2, [i64=0x7E, i32=0x7F]); results = vec(0, []).
        assert_eq!(host_import_functype(&f), vec![0x60, 0x02, 0x7E, 0x7F, 0x00]);
    }

    // A record param INTERLEAVES its flattened slots with sibling params in order: a leading scalar
    // `s64`, then `record { a: s64, b: bool }` → core `(i64, i64, i32)`. Guards the `params.len()`
    // slot-count (a record contributes N, not 1, to the core arity).
    #[test]
    fn a_record_param_interleaves_its_flattened_slots_with_siblings() {
        let f = imp(
            vec![
                HostParam::Scalar(AbiValType::S64),
                HostParam::Record(vec![
                    ("a".into(), RecordFieldAbi::Scalar(AbiValType::S64)),
                    ("b".into(), RecordFieldAbi::Scalar(AbiValType::Bool)),
                ]),
            ],
            None,
        );
        assert_eq!(
            host_import_functype(&f),
            vec![0x60, 0x03, 0x7E, 0x7E, 0x7F, 0x00]
        );
    }
}

#[cfg(test)]
mod bump_realloc_grow_tests {
    use super::*;

    /// True iff `needle` occurs as a contiguous subsequence of `hay`.
    fn contains_subseq(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    // The §3c bytes-roundtrip member module's `cabi_realloc` bump allocator MUST grow linear memory to
    // cover its allocation — a >64-KiB `list<u8>` the host lowers here (or a >1-page result buffer) would
    // otherwise write OOB past the initial single 64-KiB page (the rcw4 page-boundary class; twin of the
    // DEFINE-mode `cabi_realloc` grow in `core_module_impl`). Pin that the emitted body carries the grow:
    // `memory.grow` of mem 0 encodes as the byte pair `[0x40, 0x00]` (opcode 0x40 + mem-index 0), and
    // `memory.size` of mem 0 as `[0x3f, 0x00]`. Neither pair occurs elsewhere in this body: the only other
    // `0x40` is the `if`'s BLOCK_EMPTY block-type, which is followed by `local.get` (`0x20`) not `0x00`; and
    // `i32.const 0` is `0x41 0x00`, not `0x40 0x00`. So their presence is a precise witness of the grow
    // guard — if a future edit drops the grow, this test reds in fast-gate before the OOB can regress.
    #[test]
    fn bump_realloc_body_grows_memory_to_cover_its_allocation() {
        // A non-`63`/`64` global index so a stray `0x3f`/`0x40` can't come from the `global.get/set` operand.
        let body = emit_bump_realloc_body(5);
        assert!(
            contains_subseq(&body, &[0x40, 0x00]),
            "emit_bump_realloc_body must emit `memory.grow 0` ([0x40,0x00]) so a >64-KiB list<u8> lowered \
             here does not write OOB past the initial page — the grow-to-cover was dropped: {body:02x?}"
        );
        assert!(
            contains_subseq(&body, &[0x3f, 0x00]),
            "emit_bump_realloc_body must query `memory.size 0` ([0x3f,0x00]) to guard the grow: {body:02x?}"
        );
    }
}
