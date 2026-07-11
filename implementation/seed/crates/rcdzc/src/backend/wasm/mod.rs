//! The wasm backend — a linearizing backend that emits a WebAssembly component.
//!
//! It is a function of the typed core and the target-neutral [`Layout`]
//! (`backends-and-targets.md` §A Backend Is A Function Of The Typed Core And A Target-Neutral
//! Layout): [`emit`] selects each reachable definition's body to flat Lir (its own representation),
//! serializes them into an embedded core module, and wraps that in the N-export component envelope.
//! Every step reads columns from the `Db` on demand — the backend is the producer of the artifact
//! column, filling it by reading the earlier ones (`query-engine.md` §Producing An Artifact Is A
//! Column A Backend Fills).
//!
//! Multi-export: every `(export …)` in the layout is emitted, each by its signature ABI, under its
//! verbatim name — no single hard-coded entry.

pub mod encode;
pub mod envelope;
pub mod lir;
// The GENERATED value-heap runtime-ABI table (`cargo xtask codegen`, from the runtime WIT + the built
// runtime's content hash) — the structured op signatures + typed `OPS` accessor the per-program import
// section + component envelope are built from (value-heap H1). `cargo xtask codegen --check` (a hard
// gate in `xtask check`) keeps it current with the runtime. Most ops are unused until a compound op
// lowers to them (value-heap H2+), so allow dead code on the table's unreferenced entries.
#[allow(dead_code)]
pub mod runtime_abi;
pub mod select;
pub mod serialize;
// The GENERATED wasm / component-model byte table (`cargo xtask codegen`, extracted from the
// `wasm-encoder` spec encoder) — every opcode, valtype, section id, magic header, and functype form
// byte the serializer lays down, so no raw byte is hand-written in the emit path. `encode::op`,
// `serialize`, `lir`, and `envelope` read these. `#[allow(dead_code)]` because the table is COMPLETE
// (it mirrors the encoder): a few entries — the `f32`/`f64` valtypes — belong to the ABI but the
// scalar-integer backend does not emit them yet. `cargo xtask codegen --check` (a hard gate) keeps
// it current with the encoder.
#[allow(dead_code)]
pub mod wasm_abi;

use crate::backend::wasm::envelope::BoundaryExport;
use crate::backend::wasm::select::{SelectedFunc, select_function};
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// Emit a WebAssembly component for the program in `db` under the boundary `layout`. Selects each
/// definition in the layout's emission order, serializes the core module, and assembles the envelope.
pub fn emit(db: &mut Db, layout: &Layout) -> Result<Vec<u8>, Reject> {
    // The per-program runtime IMPORT SET must be fixed BEFORE selection, because it determines both
    // `layout.import_base` (the shift a defined func's index takes) and the index a `CallImport`
    // resolves to. Walk every reachable body's core for the value-heap ops it will emit
    // (`collect_used_ops`, which mirrors `select`'s op choices exactly), collect them into a
    // deterministic sorted set, and resolve each to its generated `RtOp`. Empty for a program that uses
    // no runtime op — no import section, no shift → byte-identical to a runtime-free build.
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        select::collect_used_ops(db, body, &mut used);
    }
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;

    // The layout with the import base fixed to the used-set size — a defined function's absolute index
    // is `import_base + its emission position` (imports occupy `0..import_base`). `layout` is otherwise
    // as computed; clone-with-base so `abs` (read by both the export section and every `Lir::Call`)
    // accounts for the shift.
    let layout = Layout {
        import_base: imports.len() as u32,
        ..layout.clone()
    };
    let layout = &layout;

    // Select each reachable definition's body, in emission order, WITH its parameters — so a
    // parameterized function (exported OR an internal callee reached by a runtime `Core::Call`) selects
    // to a real wasm function (params → local slots, body → machine ops). An EXPORT's params come from
    // its plan (which already solved boundary valtypes); a reachable NON-export callee (a recursive
    // function) reads its params via `layout::def_params` (core valtypes only — it never crosses the
    // boundary).
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.exports.iter().find(|e| e.def == def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function(db, body, &params, layout)?);
    }

    // Serialize the embedded core module (multi-export core module, functions in emission order).
    let core = serialize::core_module(&funcs, &imports, layout).map_err(Reject::decline)?;

    // Build the component-boundary export list (each export's parameter + result valtypes) and
    // assemble the envelope. Export `k` in the layout lifts core func `k` (exports first, in order).
    let mut boundary: Vec<BoundaryExport> = Vec::new();
    for e in &layout.exports {
        let result = serialize::export_result_valtype(&e.result).map_err(Reject::decline)?;
        // Each parameter's COMPONENT-boundary valtype (distinct from the core valtype — a signed 64
        // integer is `s64` at the boundary, `i64` in the core). Reuses the result mapping per param.
        let mut params = Vec::new();
        for (_, ty) in &e.params {
            let vt = serialize::export_result_valtype(ty)
                .map_err(Reject::decline)?
                .ok_or_else(|| Reject::decline("a parameter type has no component valtype"))?;
            params.push(vt);
        }
        boundary.push(BoundaryExport {
            name: e.name.clone(),
            params,
            result,
        });
    }

    // The versioned runtime import name (`cadenza:runtime/heap@0.0.0+<hash>`) — the name the runtime
    // component is imported under, carrying the content-address suffix `cdz-run` resolves it by. Unused
    // when `imports` is empty (the bare envelope). Built here (not in `envelope`) so the envelope stays
    // ABI-agnostic; the ABI identity lives in the generated `runtime_abi` table.
    let import_name = runtime_import_name();
    Ok(envelope::assemble(&core, &boundary, &imports, &import_name))
}

/// The program's runtime import name: the interface (`cadenza:runtime/heap`) pinned to the semver
/// `0.0.0` with the runtime's content hash as build-metadata (`+<hash>`) — the versioned form `cdz-run`
/// matches against the composed runtime (`component-abi.md` §The Value-Heap Runtime Crosses By A
/// Well-Known Import). Both parts come from the generated ABI table, so a runtime change re-pins it.
fn runtime_import_name() -> String {
    format!(
        "{}@0.0.0+{}",
        runtime_abi::RUNTIME_IFACE,
        runtime_abi::REQUIRED_RUNTIME_HASH
    )
}

/// The AST body occurrence of definition `def`, or a decline if it is malformed (no body).
fn def_body(db: &Db, def: usize) -> Result<crate::ast::StructId, Reject> {
    db.defs[def]
        .body
        .ok_or_else(|| Reject::decline(format!("definition `{}` has no body", db.defs[def].name)))
}

#[cfg(test)]
mod runtime_abi_tests {
    use super::runtime_abi::{AbiValType, OPS, RUNTIME_IFACE, RUNTIME_OPS};

    /// The generated ABI carries the known product/sum op signatures from the WIT — a guard that
    /// `xtask codegen` faithfully maps the WIT types to LOGICAL ABI types (arr-get borrows a u32 index
    /// → u32, sum-new pairs two u32 handles → u32). Pins the H0 done-criterion: the structured data is
    /// correct, keeping the logical (not core-collapsed) type the component import instance-type needs.
    #[test]
    fn generated_ops_match_the_known_signatures() {
        // `arr-get(arr, index) -> elem` : two u32 params (handle + index) → a u32 handle.
        assert_eq!(OPS.arr_get.name, "arr-get");
        assert_eq!(OPS.arr_get.params, &[AbiValType::U32, AbiValType::U32]);
        assert_eq!(OPS.arr_get.result, Some(AbiValType::U32));
        // `sum-new(disc, payload) -> handle`.
        assert_eq!(OPS.sum_new.name, "sum-new");
        assert_eq!(OPS.sum_new.params, &[AbiValType::U32, AbiValType::U32]);
        // `box-int(s64) -> handle` : the one s64 param op.
        assert_eq!(OPS.box_int.params, &[AbiValType::S64]);
        // `dup(handle)` : a borrow op with NO result.
        assert_eq!(OPS.dup.result, None);
        // The two byte projections: a u32 handle is core i32 (0x7F) but component u32 (0x79) — the
        // distinction the logical type preserves (H1b's whole reason for keeping it logical).
        assert_eq!(AbiValType::U32.core_byte(), 0x7F);
        assert_eq!(AbiValType::U32.comp_byte(), 0x79);
        assert_eq!(AbiValType::S64.core_byte(), 0x7E);
        assert_eq!(AbiValType::S64.comp_byte(), 0x78);
        assert_eq!(RUNTIME_IFACE, "cadenza:runtime/heap");
    }

    /// Every `OPS` field points at the same-named entry in `RUNTIME_OPS` — the typed accessor and the
    /// iterable list agree (no offset drift in the generated struct).
    #[test]
    fn ops_accessor_agrees_with_the_list() {
        for op in [
            OPS.arr_alloc,
            OPS.arr_set,
            OPS.arr_get,
            OPS.arr_len,
            OPS.sum_disc,
        ] {
            assert!(
                RUNTIME_OPS.iter().any(|o| std::ptr::eq(o, op)),
                "OPS.{} does not point into RUNTIME_OPS",
                op.name
            );
        }
        // A lowerable op has only core-scalar params; str-new (string) is flagged unlowerable.
        assert!(OPS.arr_get.lowerable);
        assert!(!OPS.str_new.lowerable);
    }
}

#[cfg(test)]
mod wasm_abi_tests {
    //! The generated `wasm_abi` table is byte-for-byte what `wasm-encoder` (the byte oracle, a
    //! dev-dependency) emits. `xtask codegen` EXTRACTS these from `wasm-encoder`, so this re-derives
    //! the same bytes IN THE CRATE and compares — a guard that the committed generated file matches
    //! the encoder for the exact rcdzc-resolved `wasm-encoder` version (the `--check` staleness gate
    //! lives in xtask; this is the in-crate correctness pin, alongside the envelope byte-identity
    //! oracle tests in `tests.rs`).
    use super::wasm_abi;

    /// A single opcode is the first byte `wasm-encoder` emits for the matching `Instruction`.
    fn opcode(insn: wasm_encoder::Instruction) -> u8 {
        use wasm_encoder::Encode;
        let mut b = Vec::new();
        insn.encode(&mut b);
        b[0]
    }

    #[test]
    fn opcodes_match_wasm_encoder() {
        use wasm_encoder::{BlockType, Instruction as I};
        // A representative spread across the arithmetic / comparison / control / conversion families
        // the serializer emits — each generated `op` const is the encoder's byte for its instruction.
        assert_eq!(wasm_abi::op::I32_ADD, opcode(I::I32Add));
        assert_eq!(wasm_abi::op::I64_MUL, opcode(I::I64Mul));
        assert_eq!(wasm_abi::op::I32_DIV_U, opcode(I::I32DivU));
        assert_eq!(wasm_abi::op::I64_REM_S, opcode(I::I64RemS));
        assert_eq!(wasm_abi::op::I32_GE_U, opcode(I::I32GeU));
        assert_eq!(wasm_abi::op::I64_EQ, opcode(I::I64Eq));
        assert_eq!(wasm_abi::op::I32_SHR_U, opcode(I::I32ShrU));
        assert_eq!(wasm_abi::op::LOCAL_GET, opcode(I::LocalGet(0)));
        assert_eq!(wasm_abi::op::CALL, opcode(I::Call(0)));
        assert_eq!(wasm_abi::op::IF, opcode(I::If(BlockType::Empty)));
        assert_eq!(wasm_abi::op::END, opcode(I::End));
        assert_eq!(wasm_abi::op::UNREACHABLE, opcode(I::Unreachable));
        assert_eq!(wasm_abi::op::I32_WRAP_I64, opcode(I::I32WrapI64));
        assert_eq!(wasm_abi::op::I64_EXTEND_I32_S, opcode(I::I64ExtendI32S));
    }

    #[test]
    fn valtypes_and_forms_match_wasm_encoder() {
        use wasm_encoder::{Encode, PrimitiveValType, ValType};
        let one = |v: &dyn Fn(&mut Vec<u8>)| {
            let mut b = Vec::new();
            v(&mut b);
            assert_eq!(b.len(), 1);
            b[0]
        };
        // Core valtypes.
        assert_eq!(wasm_abi::CORE_I32, one(&|b| ValType::I32.encode(b)));
        assert_eq!(wasm_abi::CORE_I64, one(&|b| ValType::I64.encode(b)));
        // Component primitives (the faithful boundary widths + bool).
        assert_eq!(
            wasm_abi::COMP_BOOL,
            one(&|b| PrimitiveValType::Bool.encode(b))
        );
        assert_eq!(wasm_abi::COMP_S8, one(&|b| PrimitiveValType::S8.encode(b)));
        assert_eq!(wasm_abi::COMP_U8, one(&|b| PrimitiveValType::U8.encode(b)));
        assert_eq!(
            wasm_abi::COMP_S64,
            one(&|b| PrimitiveValType::S64.encode(b))
        );
        assert_eq!(
            wasm_abi::COMP_U64,
            one(&|b| PrimitiveValType::U64.encode(b))
        );
        // The empty block type.
        assert_eq!(
            wasm_abi::BLOCK_EMPTY,
            one(&|b| wasm_encoder::BlockType::Empty.encode(b))
        );
    }

    #[test]
    fn magic_headers_match_wasm_encoder() {
        assert_eq!(wasm_abi::CORE_MAGIC, wasm_encoder::Module::HEADER);
        assert_eq!(wasm_abi::COMPONENT_MAGIC, wasm_encoder::Component::HEADER);
    }

    #[test]
    fn section_ids_match_wasm_encoder() {
        use wasm_encoder::{ComponentSectionId, SectionId};
        assert_eq!(wasm_abi::CORE_SEC_TYPE, SectionId::Type as u8);
        assert_eq!(wasm_abi::CORE_SEC_FUNCTION, SectionId::Function as u8);
        assert_eq!(wasm_abi::CORE_SEC_EXPORT, SectionId::Export as u8);
        assert_eq!(wasm_abi::CORE_SEC_CODE, SectionId::Code as u8);
        assert_eq!(
            wasm_abi::COMP_SEC_CORE_MODULE,
            ComponentSectionId::CoreModule as u8
        );
        assert_eq!(
            wasm_abi::COMP_SEC_CORE_INSTANCE,
            ComponentSectionId::CoreInstance as u8
        );
        assert_eq!(wasm_abi::COMP_SEC_ALIAS, ComponentSectionId::Alias as u8);
        assert_eq!(wasm_abi::COMP_SEC_TYPE, ComponentSectionId::Type as u8);
        assert_eq!(
            wasm_abi::COMP_SEC_CANONICAL,
            ComponentSectionId::CanonicalFunction as u8
        );
        assert_eq!(wasm_abi::COMP_SEC_EXPORT, ComponentSectionId::Export as u8);
    }
}
