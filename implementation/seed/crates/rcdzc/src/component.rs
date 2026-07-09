//! `component` — the N-export component-model envelope, hand-built around an embedded core module.
//!
//! A component that exports N functions is fully mechanical: one embedded core module, a fixed
//! core-instance (`instantiate 0`), then FOUR count-prefixed vectors, one entry per export —
//!
//! ```text
//! sec 1  core-module   : the embedded core bytes
//! sec 2  core-instance : instantiate module 0            (fixed: 02 04 01 00 00 00)
//! sec 6  core-alias    : N × alias core-instance 0 export "name" (core func)
//! sec 7  component-type : N × functype (from each export's signature → component valtypes)
//! sec 8  canon         : N × canon lift (core func i) (type i)
//! sec 11 comp-export   : N × export "name" (func i)
//! ```
//!
//! This REPLACES pasting the fixed `FRAME_COMP_*` blobs (each of which hard-codes ONE `run` export and
//! so cannot compose). The per-export item grammar is derived byte-for-byte from a wasm-encoder oracle
//! (see the `oracle` test): at N=1 this reproduces the `frame` constants exactly (so a single
//! nullary-scalar entry stays byte-identical to the old compiler); at N>1 it generalizes. Hand-built
//! in plain byte pushes so it ports 1:1 to the Cadenza self-host (no wasm-encoder in the byte path).
//!
//! Boundary valtypes: each export's signature maps to component valtypes via `Ty::comp_valtype`
//! (Int→s64 `0x78`, Bool→`0x7f`, …). A parameterized or compound export is not yet lifted here (the
//! heap/`compile` surfaces are assembled by `heap`); this module covers the scalar func-export surface
//! (nullary and, structurally, parameterized scalar exports) that the multi-export foundation needs.

use crate::frame::FRAME_COMP_MAGIC;
use crate::layout::ExportPlan;
use crate::wasm::{section, uleb128, uleb_bytes, wasm_vec};

/// Component section ids used by the envelope (the component-model section numbering, distinct from
/// the core-wasm one — e.g. component "type" is 7, "export" is 11).
mod sec {
    pub const CORE_MODULE: u8 = 1;
    pub const CORE_INSTANCE: u8 = 2;
    pub const ALIAS: u8 = 6;
    pub const COMPONENT_TYPE: u8 = 7;
    pub const CANON: u8 = 8;
    pub const COMPONENT_EXPORT: u8 = 11;
}

/// The fixed core-instance section body: `instantiate module 0`, no arguments. Encodes as
/// `<count=1> <kind=instantiate:0x00> <module-idx=0> <arg-count=0>` = `01 00 00 00`.
const INSTANCE_BODY: &[u8] = &[0x01, 0x00, 0x00, 0x00];

/// Assemble the whole component for a set of scalar function exports around an embedded `core`
/// module. `exports` are the boundary exports in emission order (their `func` fields already carry
/// each core function's index — for the scalar path the k-th export is core func k). Returns the
/// component bytes.
pub fn assemble(core: &[u8], exports: &[ExportForCore]) -> Result<Vec<u8>, String> {
    let n = exports.len();

    // sec 6: one core-func alias per export.
    let mut alias_items = Vec::new();
    for e in exports {
        alias_items.extend_from_slice(&alias_item(&e.name));
    }
    let alias_sec = section(sec::ALIAS, &wasm_vec(n, &alias_items));

    // sec 7: one component functype per export (from its signature).
    let mut type_items = Vec::new();
    for e in exports {
        type_items.extend_from_slice(&comp_functype(e)?);
    }
    let type_sec = section(sec::COMPONENT_TYPE, &wasm_vec(n, &type_items));

    // sec 8: one canon-lift per export — lift core func i using component type i.
    let mut canon_items = Vec::new();
    for i in 0..n {
        canon_items.extend_from_slice(&canon_lift_item(i as u32, i as u32));
    }
    let canon_sec = section(sec::CANON, &wasm_vec(n, &canon_items));

    // sec 11: one component export per export — export component func i under its boundary name.
    let mut export_items = Vec::new();
    for (i, e) in exports.iter().enumerate() {
        export_items.extend_from_slice(&comp_export_item(&e.name, i as u32));
    }
    let export_sec = section(sec::COMPONENT_EXPORT, &wasm_vec(n, &export_items));

    let mut out = Vec::new();
    out.extend_from_slice(FRAME_COMP_MAGIC);
    // sec 1: embedded core module.
    out.push(sec::CORE_MODULE);
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(core);
    // sec 2: core instance.
    out.extend_from_slice(&section(sec::CORE_INSTANCE, INSTANCE_BODY));
    // The component-model section order matches the old compiler (the byte gate): the component TYPE
    // (7) precedes the core ALIAS (6), then the canon lift (8), then the export (11). These index
    // spaces are independent, so type-before-alias is well-formed; matching this order is what keeps a
    // single nullary-scalar entry byte-identical.
    out.extend_from_slice(&type_sec);
    out.extend_from_slice(&alias_sec);
    out.extend_from_slice(&canon_sec);
    out.extend_from_slice(&export_sec);
    Ok(out)
}

/// A scalar function export as the component assembler needs it: the boundary name and the signature
/// as component valtypes (params and an optional single result).
pub struct ExportForCore {
    pub name: String,
    /// Each parameter's component valtype byte (e.g. `0x78` = s64).
    pub params: Vec<u8>,
    /// The result's component valtype byte, or `None` for a `unit` (no result) export.
    pub result: Option<u8>,
}

impl ExportForCore {
    /// Derive the core-view of an `ExportPlan` (its component valtypes from its `Ty` signature). A
    /// parameter/return type with no component valtype (a compound, unit, or unsolved var) is a clean
    /// error here — those surfaces are assembled elsewhere (heap) / are not exportable.
    pub fn from_plan(plan: &ExportPlan) -> Result<ExportForCore, String> {
        let params = plan
            .params
            .iter()
            .map(|t| t.comp_valtype().ok_or_else(|| format!("export `{}` parameter has no component valtype", plan.name)))
            .collect::<Result<Vec<u8>, String>>()?;
        // A `unit` return is a no-result export; any other type must have a component valtype.
        let result = if matches!(plan.ret, crate::ty::Ty::Unit) {
            None
        } else {
            Some(
                plan.ret
                    .comp_valtype()
                    .ok_or_else(|| format!("export `{}` result has no component valtype", plan.name))?,
            )
        };
        Ok(ExportForCore { name: plan.name.clone(), params, result })
    }
}

/// A sec-6 core-func alias item: `<sort=core:0x00> <coresort=func:0x00> <instance-idx=0> <name>`.
/// (Decoded item bytes: `00 00 01 00 <namelen> <name>` — the `01 00` is the alias-target encoding
/// "core-instance-export, instance 0".)
fn alias_item(name: &str) -> Vec<u8> {
    let mut item = vec![0x00, 0x00, 0x01, 0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item
}

/// A sec-7 component functype item for a scalar signature: `<func:0x40> <params-vec> <result-form>`.
/// The result form is `00 <valtype>` for one result, `00 00`(?) — decoded as `00 <valtype>` for a
/// present result and `01 00` for none. (Matches the wasm-encoder oracle: `()->s64` = `40 00 00 78`.)
fn comp_functype(e: &ExportForCore) -> Result<Vec<u8>, String> {
    // The multi-export FOUNDATION realizes the nullary scalar surface (every corpus entry is nullary).
    // A component functype's params are a vector of LABELLED `(name, type)` pairs — a distinct
    // encoding whose bytes are not yet oracle-verified — so a parameterized export declines here
    // (decline-don't-miscompile) rather than emit a guessed encoding. The `compile` ABI (a
    // `list<u8>` param) gets its own verified surface when that path is built.
    if !e.params.is_empty() {
        return Err(format!("export `{}`: parameterized component exports not yet supported", e.name));
    }
    let mut item = vec![0x40]; // function type form
    item.extend_from_slice(&wasm_vec(0, &[])); // no params
    match e.result {
        // one result present: result-kind 0x00 then the valtype byte (oracle: `00 78`).
        Some(vt) => item.extend_from_slice(&[0x00, vt]),
        // no result: result-kind 0x01 (none) then a trailing 0x00 (oracle: `() -> ()` = `40 00 01 00`).
        None => item.extend_from_slice(&[0x01, 0x00]),
    }
    Ok(item)
}

/// A sec-8 canon-lift item: lift `core_func` as component func using component `type_idx`. Decoded
/// item bytes: `00 00 <core-func-uleb> 00 <type-uleb>` — `00 00` = "canon lift core func", `00` =
/// empty canon-options vector.
fn canon_lift_item(core_func: u32, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00, 0x00];
    uleb128(core_func as u64, &mut item);
    item.push(0x00); // canon options: none
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-11 component-export item: `<0x00> <name> <sort=func:0x01> <func-idx> <ty=none:0x00>`.
/// (Decoded item bytes: `00 <namelen><name> 01 <funcidx> 00`.)
fn comp_export_item(name: &str, func_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x01); // sort: component func
    uleb128(func_idx as u64, &mut item);
    item.push(0x00); // no declared type ascription
    item
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal core module exporting `names` as nullary `() -> i64` functions, each returning
    /// `42 + i` — the core the assembler wraps (independent of the compiler front end, so the test
    /// isolates the component envelope). Hand-built with the shared `wasm` helpers.
    fn core_scalar(names: &[&str]) -> Vec<u8> {
        use crate::wasm::{section, sleb128, uleb128, uleb_bytes, wasm_vec};
        let n = names.len();
        let functype = { let mut t = vec![0x60]; t.extend_from_slice(&wasm_vec(0, &[])); t.extend_from_slice(&wasm_vec(1, &[0x7e])); t };
        let mut types = Vec::new();
        for _ in 0..n { types.extend_from_slice(&functype); }
        let type_sec = section(1, &wasm_vec(n, &types));
        let mut fitems = Vec::new();
        for i in 0..n { uleb128(i as u64, &mut fitems); }
        let func_sec = section(3, &wasm_vec(n, &fitems));
        let mut eitems = Vec::new();
        for (i, nm) in names.iter().enumerate() {
            let mut it = uleb_bytes(nm.len() as u64);
            it.extend_from_slice(nm.as_bytes());
            it.push(0x00);
            uleb128(i as u64, &mut it);
            eitems.extend_from_slice(&it);
        }
        let export_sec = section(7, &wasm_vec(n, &eitems));
        let mut citems = Vec::new();
        for i in 0..n {
            let mut inner = vec![0x00]; // no locals
            inner.push(0x42); // i64.const
            sleb128(42 + i as i64, &mut inner);
            inner.push(0x0b); // end
            citems.extend_from_slice(&uleb_bytes(inner.len() as u64));
            citems.extend_from_slice(&inner);
        }
        let code_sec = section(10, &wasm_vec(n, &citems));
        let mut core = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        core.extend_from_slice(&type_sec);
        core.extend_from_slice(&func_sec);
        core.extend_from_slice(&export_sec);
        core.extend_from_slice(&code_sec);
        core
    }

    /// The wasm-encoder ORACLE: the canonical component for `names` (each a nullary `() -> s64`
    /// export), section order matching the assembler (component-type before core-alias).
    fn oracle(core: &[u8], names: &[&str]) -> Vec<u8> {
        use wasm_encoder::*;
        let mut c = Component::new();
        c.section(&RawSection { id: ComponentSectionId::CoreModule as u8, data: core });
        let mut inst = InstanceSection::new();
        inst.instantiate(0, std::iter::empty::<(&str, ModuleArg)>());
        c.section(&inst);
        // component types (sec 7) FIRST — matches the assembler / the old compiler's frame order.
        let mut ts = ComponentTypeSection::new();
        for _ in names {
            ts.function()
                .params(std::iter::empty::<(&str, ComponentValType)>())
                .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
        }
        c.section(&ts);
        let mut al = ComponentAliasSection::new();
        for nm in names {
            al.alias(Alias::CoreInstanceExport { instance: 0, kind: ExportKind::Func, name: nm });
        }
        c.section(&al);
        let mut canon = CanonicalFunctionSection::new();
        for i in 0..names.len() {
            canon.lift(i as u32, i as u32, []);
        }
        c.section(&canon);
        let mut ex = ComponentExportSection::new();
        for (i, nm) in names.iter().enumerate() {
            ex.export(nm, ComponentExportKind::Func, i as u32, None);
        }
        c.section(&ex);
        c.finish()
    }

    /// Build the assembler's component for `names` (each nullary s64), over the same core.
    fn ours(core: &[u8], names: &[&str]) -> Vec<u8> {
        let exports: Vec<ExportForCore> = names
            .iter()
            .map(|nm| ExportForCore { name: nm.to_string(), params: vec![], result: Some(0x78) })
            .collect();
        assemble(core, &exports).expect("assemble")
    }

    /// The hand-built N-export component is BYTE-IDENTICAL to the wasm-encoder oracle, at N=1 and N=2.
    /// This is what licenses hand-encoding the component envelope (no external encoder in the byte
    /// path): the per-export item grammar is exactly the authoritative encoder's.
    #[test]
    fn assembler_matches_wasm_encoder_oracle() {
        for names in [&["run"][..], &["run", "double"][..]] {
            let core = core_scalar(names);
            assert_eq!(ours(&core, names), oracle(&core, names), "mismatch for exports {names:?}");
        }
    }

    /// A UNIT-result (no-result) export's component functype byte-matches the wasm-encoder oracle
    /// (`() -> ()` = `40 00 01 00` — the trailing `00` after the none-marker is easy to omit).
    #[test]
    fn unit_result_functype_matches_oracle() {
        use wasm_encoder::*;
        // Just the component type section body from the oracle, for one `() -> ()` function.
        let mut ts = ComponentTypeSection::new();
        ts.function()
            .params(std::iter::empty::<(&str, ComponentValType)>())
            .result(None);
        let mut oracle_comp = Component::new();
        oracle_comp.section(&ts);
        let oracle_bytes = oracle_comp.finish();
        // The type-section content (skip the 8-byte component preamble, section id, and length).
        let oracle_sec = &oracle_bytes[8..];
        // Ours: a single no-result export's functype, wrapped as the same section.
        let item = comp_functype(&ExportForCore { name: "run".into(), params: vec![], result: None }).unwrap();
        let ours_sec = section(sec::COMPONENT_TYPE, &wasm_vec(1, &item));
        assert_eq!(ours_sec, oracle_sec, "unit-result functype section mismatch");
    }
}
