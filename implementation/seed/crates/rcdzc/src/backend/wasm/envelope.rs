//! `envelope` — the N-export component-model envelope wrapping an embedded core module.
//!
//! A component exporting N functions is fully mechanical: one embedded core module, a fixed
//! core-instance (`instantiate module 0`), then four count-prefixed vectors — one entry per export —
//! for the component types, the core-func aliases, the canon lifts, and the component exports. The
//! per-export item grammar is byte-identical to the authoritative component-model encoder (the
//! `wasm-encoder` oracle test pins it), which is what licenses hand-emitting the envelope with no
//! external encoder in the compile path (`reference-compiler.md` §Emission Is Validated Byte-Identical
//! To An Independent Encoder). Multi-export from the start — no single hard-coded `run`.
//!
//! Section order is load-bearing for byte identity: the component TYPE section (7) precedes the core
//! ALIAS section (6). These index spaces are independent, so type-before-alias is well-formed, and
//! matching this order is what keeps a single nullary-scalar entry byte-identical to the oracle (and
//! to the old compiler's frame).
//!
//! What comes from where: the single-byte ABI values here — the component MAGIC header, the section
//! ids, the component functype form tag — are read from the GENERATED `wasm_abi` table (extracted
//! from `wasm-encoder`), so none is hand-typed. The per-item GRAMMARS below (`INSTANCE_BODY`, the
//! alias / canon-lift / export items, the result-list form) still lay their bytes by hand: they
//! encode the component-model "sort" tags (`0x00` core, `0x01` func, …) which `wasm-encoder` does
//! NOT expose as public constants. Those are pinned instead by the byte-identity oracle test
//! (`envelope_matches_wasm_encoder_oracle`) — a whole-item diff against the authoritative encoder,
//! which is the stronger check for a multi-byte structural encoding.

use crate::backend::wasm::encode::{section, uleb_bytes, uleb128, wasm_vec};
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
    pub const COMPONENT_EXPORT: u8 = wasm_abi::COMP_SEC_EXPORT;
}

/// The fixed core-instance body: `instantiate module 0`, no args → `<count=1> <kind=0x00> <mod=0>
/// <argcount=0>` = `01 00 00 00`.
const INSTANCE_BODY: &[u8] = &[0x01, 0x00, 0x00, 0x00];

/// One export as the envelope assembler needs it: its verbatim boundary name, its parameter component
/// valtype bytes (in order; empty for a nullary export), and its result's component valtype byte
/// (`None` for a unit / no-result export).
pub struct BoundaryExport {
    pub name: String,
    pub params: Vec<u8>,
    pub result: Option<u8>,
}

/// Assemble the whole component around an embedded `core` module for the given boundary exports (in
/// declaration/emission order; export `k` lifts core func `k`). Returns the component bytes.
pub fn assemble(core: &[u8], exports: &[BoundaryExport]) -> Vec<u8> {
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
        alias_items.extend_from_slice(&alias_item(&e.name));
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
    out.push(sec::CORE_MODULE);
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(core);
    // sec 2: core instance.
    out.extend_from_slice(&section(sec::CORE_INSTANCE, INSTANCE_BODY));
    // Component TYPE (7) BEFORE core ALIAS (6) — load-bearing for byte identity.
    out.extend_from_slice(&type_sec);
    out.extend_from_slice(&alias_sec);
    out.extend_from_slice(&canon_sec);
    out.extend_from_slice(&export_sec);
    out
}

/// A sec-7 component functype item: `<func:0x40> <params-vec> <result-form>`. The params vec is
/// `<count> (<name> <valtype>)*` — each parameter is NAMED at the component boundary (a positional
/// call ignores the name, so they are synthesized `p0`, `p1`, …). The result form is `00 <valtype>`
/// for one result, `01 00` for none. (Matches the oracle: `() -> s64` = `40 00 00 78`; a `(p0: s64,
/// p1: s64) -> s64` prefixes the two named params.)
fn comp_functype(e: &BoundaryExport) -> Vec<u8> {
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
        Some(vt) => item.extend_from_slice(&[0x00, vt]),
        None => item.extend_from_slice(&[0x01, 0x00]),
    }
    item
}

/// A sec-6 core-func alias item: `00 00 01 00 <namelen> <name>` — sort core:0x00, coresort func:0x00,
/// alias-target core-instance-export (`01 00` = instance 0), then the export name.
fn alias_item(name: &str) -> Vec<u8> {
    let mut item = vec![0x00, 0x00, 0x01, 0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
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
