//! Host-import type + group construction for the wasm backend (extracted from `mod.rs`).
//!
//! Builds the per-program host `import` section from the layout's host ops: collecting the constant
//! string args a `Core::HostCall` passes, laying the component functype items (`host_op_comp_functype`,
//! `extern_op_comp_functype`), the record/result import defined-types (`build_record_import_types`,
//! `build_host_result_types`, `remap_cdef`, `record_field_cref`), the per-op import group
//! (`build_host_group`), the result-lift op declarations, and the extern-import bridge
//! (`host_as_extern_for`, `host_param_abi`). Behavior-preserving relocation of a cohesive `mod.rs`
//! cluster; imports are function-local and travel with each body, with the sibling backend submodules
//! and the two module-level uses (`Db`, `Reject`) re-declared here.

use super::{encode, envelope, host, runtime_abi, wasm_abi};
use crate::db::Db;
use crate::diag::Reject;

/// The component functype item (`0x40 <params> <result>`) for a host op — its parameter and result
/// COMPONENT valtype bytes (the faithful boundary primitives). Params are NAMED (`p0`, `p1`, …) as the
/// component model requires. A SCALAR param is its `AbiValType::comp_byte`; a STRING param is the
/// component `string` primitive (`COMP_STRING`). A `Unit` domain/result was already elided.
/// Collect the CONSTANT string values a `Core::HostCall` passes as a `string` argument, in encounter
/// order (E2h-string). Each becomes a data-segment entry the arg emit points `(ptr,len)` at. A non-string
/// arg is ignored; the walk descends every child so a host call nested anywhere is found. Mirrors
/// `host::collect_host_imports`' descent but gathers the arg strings rather than the op signatures.
pub(super) fn collect_host_arg_strings(
    db: &mut Db,
    id: crate::ast::StructId,
    out: &mut Vec<String>,
) {
    // WALK-DEPTH GUARD — like the other core walks (`collect_host_imports`, `collect_closure_codes`): this
    // drives `core_of` at every node, so a non-normalizing self-application would overflow the stack.
    if db.walk_depth >= crate::db::WALK_DEPTH_LIMIT {
        return;
    }
    // SHARING-AWARE VISITED-SET (see [`Db::host_arg_string_visited`], same class as `collect_host_imports`
    // / `callee_visited`): skip an already-walked shared node — the laid-string set is presence-only (the
    // consumer dedups distinct strings), so this changes no output while collapsing the DAG re-descent.
    // Cleared at the top-level entry (`walk_depth == 0`); after the depth guard (clip is accepted-neutral).
    if db.walk_depth == 0 {
        db.host_arg_string_visited.clear();
    }
    if !db.host_arg_string_visited.insert(id) {
        return;
    }
    db.walk_depth += 1;
    collect_host_arg_strings_at(db, id, out);
    db.walk_depth -= 1;
}

/// The CORE walk of [`collect_host_arg_strings`] — the sibling of `host::collect_host_imports`, laying the
/// same discipline: descend the LOWERED core (NOT the AST), so a constant string argument of a `HostCall`
/// reached through an INLINED helper — a `report/Test.fail("msg")` where the message became a `ConstStr`
/// on β-substitution of the helper's param — is laid in the data segment. Was AST-walked, so an inlined
/// helper's host-arg string was missed ("a host-arg string was not laid in the data segment"). Exhaustive
/// (no wildcard) so a new `Core` variant is a compile error, not a silently-unlaid string.
pub(super) fn collect_host_arg_strings_at(
    db: &mut Db,
    id: crate::ast::StructId,
    out: &mut Vec<String>,
) {
    use crate::core::Core;
    match crate::lower::core_of(db, id) {
        Core::HostCall { args, effect, .. } => {
            // Only a HOST call marshals a constant `string` arg through the data segment (`(ptr,len)`); a
            // PEER call (a peer-BOUND effect) crosses a String/Bytes arg as a runtime rope HANDLE built on
            // the value heap (see the peer `Core::HostCall` emit + `collect_used_ops`), so its constant
            // strings must NOT be laid in the data segment — doing so would trip the spurious `mem` import
            // (`needs_memory = !host_strings.is_empty()`) that the runtime-only peer envelope never supplies,
            // yielding an invalid consumer component.
            let peer_bound = db.effect_bindings.contains_key(&*effect);
            if !peer_bound {
                for a in args.iter() {
                    if let Core::ConstStr(s) = crate::lower::core_of(db, *a) {
                        out.push(s.to_string());
                    }
                }
            }
            for &a in args.iter() {
                collect_host_arg_strings(db, a, out);
            }
        }
        Core::Call { args, .. } => {
            for &a in args.iter() {
                collect_host_arg_strings(db, a, out);
            }
        }
        Core::CallClosure { closure, args } => {
            collect_host_arg_strings(db, closure, out);
            for &a in args.iter() {
                collect_host_arg_strings(db, a, out);
            }
        }
        // A closure's CAPTURES may include a host-call result carrying a string arg — walk them (the body
        // is walked as its own lifted function). Mirrors `collect_host_imports`'s `Core::Closure` arm.
        Core::Closure { captures, .. } => {
            for &c in captures.iter() {
                collect_host_arg_strings(db, c, out);
            }
        }
        Core::If { cond, then_, else_ } => {
            collect_host_arg_strings(db, cond, out);
            collect_host_arg_strings(db, then_, out);
            collect_host_arg_strings(db, else_, out);
        }
        Core::Let { bindings, body } => {
            for (_, value) in bindings.iter().copied() {
                collect_host_arg_strings(db, value, out);
            }
            collect_host_arg_strings(db, body, out);
        }
        Core::Seq { stmts, tail } => {
            for &s in stmts.iter() {
                collect_host_arg_strings(db, s, out);
            }
            collect_host_arg_strings(db, tail, out);
        }
        // A boundary block / break — descend into the body / break value for any host-arg string inside.
        Core::Block { body, .. } => collect_host_arg_strings(db, body, out),
        Core::Break { value } => collect_host_arg_strings(db, value, out),
        // The abort VALUE is evaluated before the non-local branch; a HostCall with a constant string arg
        // inside it needs its string in the data segment. Recurse into it; `handle_id` is a reference to the
        // target handle node, not an emitted subexpression.
        Core::HandleAbort { value, .. } => collect_host_arg_strings(db, value, out),
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::ValueEq { lhs, rhs }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. }
        | Core::ListConcat { lhs, rhs }
        | Core::BytesConcat { lhs, rhs }
        | Core::BigIntBinOp { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalOfInts { num: lhs, den: rhs }
        | Core::RationalBinOp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. } => {
            collect_host_arg_strings(db, lhs, out);
            collect_host_arg_strings(db, rhs, out);
        }
        Core::BigIntOfI64 { value } => collect_host_arg_strings(db, value, out),
        Core::BigIntToI64 { operand } => collect_host_arg_strings(db, operand, out),
        Core::CharToInt { operand } | Core::IntToCharChecked { operand, .. } => {
            collect_host_arg_strings(db, operand, out)
        }
        Core::RationalOfIntWiden { value } => collect_host_arg_strings(db, value, out),
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            collect_host_arg_strings(db, operand, out)
        }
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            collect_host_arg_strings(db, list, out);
            collect_host_arg_strings(db, elem, out);
        }
        Core::ListUpdate { list, index, elem } => {
            collect_host_arg_strings(db, list, out);
            collect_host_arg_strings(db, index, out);
            collect_host_arg_strings(db, elem, out);
        }
        Core::ListAt { list, index, .. } => {
            collect_host_arg_strings(db, list, out);
            collect_host_arg_strings(db, index, out);
        }
        Core::MapNew { entries, .. } => {
            for (k, v) in entries.iter().copied() {
                collect_host_arg_strings(db, k, out);
                collect_host_arg_strings(db, v, out);
            }
        }
        Core::MapInsert { map, key, val, .. } => {
            collect_host_arg_strings(db, map, out);
            collect_host_arg_strings(db, key, out);
            collect_host_arg_strings(db, val, out);
        }
        Core::MapLookup { map, key, .. } | Core::MapRemove { map, key, .. } => {
            collect_host_arg_strings(db, map, out);
            collect_host_arg_strings(db, key, out);
        }
        Core::MapSize { map } => collect_host_arg_strings(db, map, out),
        Core::SetOf { elems, .. } => {
            for &e in elems.iter() {
                collect_host_arg_strings(db, e, out);
            }
        }
        Core::SetContains { set, elem, .. }
        | Core::SetInsert { set, elem, .. }
        | Core::SetRemove { set, elem, .. } => {
            collect_host_arg_strings(db, set, out);
            collect_host_arg_strings(db, elem, out);
        }
        Core::SetLen { set } => collect_host_arg_strings(db, set, out),
        Core::SetToList { set, .. } => collect_host_arg_strings(db, set, out),
        Core::MapToList { map, .. } => collect_host_arg_strings(db, map, out),
        Core::SetAlgebra { lhs, rhs, .. } => {
            collect_host_arg_strings(db, lhs, out);
            collect_host_arg_strings(db, rhs, out);
        }
        Core::BytesAt { bytes, index, .. } => {
            collect_host_arg_strings(db, bytes, out);
            collect_host_arg_strings(db, index, out);
        }
        Core::StrAt { string, index, .. } => {
            collect_host_arg_strings(db, string, out);
            collect_host_arg_strings(db, index, out);
        }
        Core::StrScalarAt { operand, index, .. } => {
            collect_host_arg_strings(db, operand, out);
            collect_host_arg_strings(db, index, out);
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            collect_host_arg_strings(db, string, out);
            collect_host_arg_strings(db, start, out);
            collect_host_arg_strings(db, end, out);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            collect_host_arg_strings(db, bytes, out);
            collect_host_arg_strings(db, start, out);
            collect_host_arg_strings(db, len, out);
        }
        Core::BytesCompact { operand }
        | Core::Blake3Of { operand }
        | Core::AstPrint { operand, .. }
        | Core::AstEncode { operand, .. }
        | Core::AstDecode { operand, .. }
        | Core::StrFromBytes { bytes: operand, .. }
        | Core::StrToBytes { string: operand }
        | Core::NfcNormalize { string: operand }
        | Core::Convert { operand, .. }
        | Core::Not { operand }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        // `Value.encode`/`decode` marshal no constant host-arg string (the descriptor is baked as
        // per-byte `bytes-set`, not a data-segment `(ptr,len)`); walk the single operand for nested ones.
        | Core::ValueEncode { value: operand, .. }
        | Core::ValueDecode { bytes: operand, .. }
        | Core::StrScalarLen { operand } => collect_host_arg_strings(db, operand, out),
        Core::Match { scrutinee, arms } => {
            collect_host_arg_strings(db, scrutinee, out);
            for arm in arms {
                if let Some(g) = arm.guard {
                    collect_host_arg_strings(db, g, out);
                }
                collect_host_arg_strings(db, arm.body, out);
            }
        }
        Core::Record { fields } => {
            for value in fields.values() {
                collect_host_arg_strings(db, *value, out);
            }
        }
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            for &e in elems.iter() {
                collect_host_arg_strings(db, e, out);
            }
        }
        Core::BinBuild { segs } => {
            for s in segs {
                collect_host_arg_strings(db, s.value, out);
            }
        }
        Core::BinBitsBuild { fields } => {
            for f in fields {
                collect_host_arg_strings(db, f.value, out);
            }
        }
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            collect_host_arg_strings(db, bytes, out);
            if let Some(op) = off_plus {
                collect_host_arg_strings(db, op, out);
            }
        }
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            collect_host_arg_strings(db, bytes, out);
            if let Some(op) = off_plus {
                collect_host_arg_strings(db, op, out);
            }
            collect_host_arg_strings(db, len, out);
        }
        Core::Proj { operand, .. } => collect_host_arg_strings(db, operand, out),
        Core::SumNew { payloads, .. } => {
            for &p in payloads.iter() {
                collect_host_arg_strings(db, p, out);
            }
        }
        Core::MatchSum { scrutinee, root } => {
            collect_host_arg_strings(db, scrutinee, out);
            collect_cont_host_arg_strings(db, &root, out);
        }
        Core::MatchList { scrutinee, arms } => {
            collect_host_arg_strings(db, scrutinee, out);
            for arm in &arms {
                collect_host_arg_strings(db, arm.body, out);
            }
        }
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => {
            collect_host_arg_strings(db, scrutinee, out)
        }
        // Leaves / references carry no host-arg string.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
        | Core::Param { .. }
        | Core::Captured { .. }
        | Core::LocalRef { .. }
        | Core::Poison(_) => {}
    }
}

/// Walk a sum-match continuation for the host-arg strings its arm bodies carry — the analogue of
/// `collect_cont_host_imports` for the data-segment string pass.
pub(super) fn collect_cont_host_arg_strings(
    db: &mut Db,
    cont: &crate::core::SumCont,
    out: &mut Vec<String>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => collect_host_arg_strings(db, *body, out),
        crate::core::SumCont::Guarded { cond, body, els } => {
            collect_host_arg_strings(db, *cond, out);
            collect_host_arg_strings(db, *body, out);
            collect_cont_host_arg_strings(db, els, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_cont_host_arg_strings(db, then_, out);
            collect_cont_host_arg_strings(db, els, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for arm in arms {
                collect_cont_host_arg_strings(db, &arm.cont, out);
            }
        }
    }
}

/// Recursively lay a record host-arg's component `record` DEFINED types into `table` (in the order
/// `host_effect_instance_type` lays them — each entry becomes a DEFINE + EXPORT pair), CHILDREN BEFORE
/// PARENTS: a NESTED-record field is emitted first so the enclosing record's field can reference the child's
/// EXPORTED index. Returns THIS record's EXPORTED instance-type index. `list_idx` is the `(list u8)` index (a
/// `Bytes` field references it); `base` is the first record's DEFINE index (the count of shared prepends,
/// e.g. 1 when `(list u8)` is present). Entry `i` occupies define `base + 2i` / export `base + 2i + 1`, so a
/// child (appended earlier) has a lower index than its parent — the dependency order the component model
/// requires. Matches `wasm-tools component wit`'s own encoding of a nested-record import (verified by oracle).
pub(super) fn build_record_import_types(
    fields: &[(String, host::RecordFieldAbi)],
    list_idx: u32,
    base: u32,
    table: &mut Vec<Vec<u8>>,
) -> u32 {
    use crate::backend::wasm::wit_ctype::{CDef, CRef, emit_cdef};
    let mut cfields: Vec<(String, CRef)> = Vec::with_capacity(fields.len());
    for (name, abi) in fields {
        cfields.push((name.clone(), record_field_cref(abi, list_idx, base, table)));
    }
    let define_idx = base + 2 * table.len() as u32;
    table.push(emit_cdef(&CDef::Record(cfields)));
    define_idx + 1 // the EXPORT index (host_effect_instance_type exports right after the define)
}

/// The component-type `CRef` for one record-field ABI, laying any needed CHILDREN-FIRST defined types into
/// `table` (each a DEFINE + EXPORT pair; see [`build_record_import_types`]). A scalar is an inline primitive; a
/// `Bytes` refs the shared `(list u8)`; a nested record / result / list lays its child type(s) first and refs
/// the child's EXPORT index. Recursive so a `list<record>` / `list<list<T>>` field nests to arbitrary depth.
pub(super) fn record_field_cref(
    abi: &host::RecordFieldAbi,
    list_idx: u32,
    base: u32,
    table: &mut Vec<Vec<u8>>,
) -> crate::backend::wasm::wit_ctype::CRef {
    use crate::backend::wasm::wit_ctype::{CDef, CRef, emit_cdef};
    match abi {
        host::RecordFieldAbi::Scalar(v) => CRef::Prim(v.comp_byte()),
        host::RecordFieldAbi::Bytes => CRef::Idx(list_idx),
        host::RecordFieldAbi::Record(sub) => {
            // Emit the CHILD record first (children-first); its field references the child's EXPORT index.
            CRef::Idx(build_record_import_types(sub, list_idx, base, table))
        }
        // A `result<list<u8>, enum-or-variant>` field: lay the err type (nominal → define+export) then the
        // `result` defined type (its ok arm refs `(list u8)`; its err arm refs the err type's EXPORT index),
        // children-first, then reference the `result`'s EXPORT index. (Both are laid as uniform
        // define+export table entries like a record; exporting the structural `result` is harmless.) The err
        // arm's CONSTRUCTOR follows the host WIT — a payload-less `variant` when the WIT declares `variant`
        // (`err_is_variant`), else an `enum`: a `result<_, variant>` and a `result<_, enum>` are DISTINCT
        // component types, so a guest whose err arm mismatched the host's constructor silently failed to
        // instantiate (the deliver-response `answer: result<list<u8>, error>` shape, where the platform WIT
        // declares `variant error`).
        host::RecordFieldAbi::Result {
            err_cases,
            err_is_variant,
        } => {
            let err_def = base + 2 * table.len() as u32;
            let err_cdef = if *err_is_variant {
                CDef::Variant(err_cases.iter().map(|c| (c.clone(), None)).collect())
            } else {
                CDef::Enum(err_cases.clone())
            };
            table.push(emit_cdef(&err_cdef));
            let err_export = err_def + 1;
            let res_def = base + 2 * table.len() as u32;
            table.push(emit_cdef(&CDef::Result {
                ok: Some(CRef::Idx(list_idx)),
                err: Some(CRef::Idx(err_export)),
            }));
            CRef::Idx(res_def + 1)
        }
        // A `list<T>` field: build the element's CRef (children-first, recursing) then lay the `(list <elem>)`
        // DEFINED type and reference its EXPORT index (structural, but exported uniformly like the rest).
        host::RecordFieldAbi::List(elem) => {
            let elem_cref = record_field_cref(elem, list_idx, base, table);
            let list_def = base + 2 * table.len() as u32;
            table.push(emit_cdef(&CDef::List(elem_cref)));
            CRef::Idx(list_def + 1)
        }
        // A `tuple<…>` field: build each element's CRef (children-first) then lay the `(tuple <elem>…)` DEFINED
        // type and reference its index (structural, exported uniformly like the rest).
        host::RecordFieldAbi::Tuple(elems) => {
            let elem_crefs: Vec<CRef> = elems
                .iter()
                .map(|e| record_field_cref(e, list_idx, base, table))
                .collect();
            let tup_def = base + 2 * table.len() as u32;
            table.push(emit_cdef(&CDef::Tuple(elem_crefs)));
            CRef::Idx(tup_def + 1)
        }
        // An `option<T>` field: build the payload's CRef (children-first) then lay the `(option <T>)` DEFINED
        // type and reference its index.
        host::RecordFieldAbi::Option(payload) => {
            let payload_cref = record_field_cref(payload, list_idx, base, table);
            let opt_def = base + 2 * table.len() as u32;
            table.push(emit_cdef(&CDef::Option(payload_cref)));
            CRef::Idx(opt_def + 1)
        }
        // A general `variant` field: lay a `variant` DEFINED type (NOMINAL → the export-aware remap gives it
        // define+export, like a record/enum) over its cases (each an inline scalar payload CRef or none), and
        // reference its EXPORT index.
        host::RecordFieldAbi::Variant(cases) => {
            let vcases: Vec<(String, Option<CRef>)> = cases
                .iter()
                .map(|(name, p)| (name.clone(), p.map(|v| CRef::Prim(v.comp_byte()))))
                .collect();
            let var_def = base + 2 * table.len() as u32;
            table.push(emit_cdef(&CDef::Variant(vcases)));
            CRef::Idx(var_def + 1)
        }
    }
}

pub(super) fn host_op_comp_functype(
    h: &host::HostImport,
    list_type_idx: u32,
    nominal_type_idx: u32,
    // Per HostParam position: a `list<T>` arg's `(list <elem>)` DEFINED-type `CRef` (from
    // `build_host_result_types`); `None`/absent for a non-list param.
    list_param_crefs: &[Option<crate::backend::wasm::wit_ctype::CRef>],
    result_cref: Option<crate::backend::wasm::wit_ctype::CRef>,
) -> Vec<u8> {
    use host::HostParam;
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, p) in h.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&(pname.len() as u8).to_le_bytes());
        param_items.extend_from_slice(pname.as_bytes());
        match p {
            // A SCALAR / STRING param is an INLINE primitive valtype byte.
            HostParam::Scalar(v) => param_items.push(v.comp_byte()),
            HostParam::Str => param_items.push(wasm_abi::COMP_STRING),
            // A `list<u8>` (Bytes) param references the shared `(list u8)` DEFINED type by its
            // instance-type-local INDEX (uleb128), NOT an inline byte — mirrors the export-side
            // `comp_functype`'s `BoundaryResult::Bytes` result encoding (`envelope::comp_functype`). The
            // caller prepends `list_u8_defined_type()` to the import instance-type and passes its index as
            // `list_type_idx`; `0` is a safe placeholder while no host set produces a Bytes param yet
            // (`collect_host_imports` does not push `HostParam::Bytes` until the emit brick).
            HostParam::Bytes => encode::uleb128(list_type_idx as u64, &mut param_items),
            // A RECORD param (shape d) references the record's EXPORTED type in the import instance-type. A
            // NOMINAL type used by an import func must be exported (component-model rule), so
            // `host_effect_instance_type` lays the record as DEFINE + EXPORT and the func references the
            // EXPORTED index — `record_type_idx`, computed by the caller from the instance-type layout
            // (`(list u8)?` prepend count + 1: index 1 for an all-scalar record with no list, 2 when a
            // `list<u8>` param/field forces the `(list u8)` type at index 0). This arm is only reached on the
            // reducer path (the boundary guard declines a record arg elsewhere).
            HostParam::Record(_) => encode::uleb128(nominal_type_idx as u64, &mut param_items),
            // An ENUM param references its EXPORTED `enum` DEFINED type (a nominal type an import func uses
            // must be exported, like a record) by the SAME `nominal_type_idx` — an op carries at most one
            // nominal param type this slice (single record OR single enum). Its discriminant crosses as one
            // i32 core slot (serialize.rs).
            HostParam::Enum(_) => encode::uleb128(nominal_type_idx as u64, &mut param_items),
            // A `list<T>` param references its `(list <elem>)` DEFINED type by the per-param `CRef` the caller
            // computed (`build_host_result_types`), like a spilled result references its type.
            HostParam::List(_) => {
                let cref = list_param_crefs
                    .get(i)
                    .cloned()
                    .flatten()
                    .unwrap_or(crate::backend::wasm::wit_ctype::CRef::Idx(list_type_idx));
                crate::backend::wasm::wit_ctype::encode_cref(&cref, &mut param_items);
            }
            // A bare VARIANT param references its EXPORTED `variant` DEFINED type (a nominal type an import
            // func uses must be exported, like a record/enum) by the SAME `nominal_type_idx` — an op carries
            // at most one nominal param type this slice. Its `(disc, payload)` crosses as the flattened core
            // slots (serialize.rs); the defined type is built by `build_host_group` (the variant_params branch).
            HostParam::Variant(_) => encode::uleb128(nominal_type_idx as u64, &mut param_items),
        }
    }
    item.extend_from_slice(&encode::wasm_vec(h.params.len(), &param_items));
    // The result valtype: a SPILLED compound references its component DEFINED type by the `result_cref` the
    // caller computed via the GENERAL `wit_ctype::add_wit_type_deduped` over the op's WIT result type — a
    // single `CRef` (an inline primitive OR a type-section index), so option<list<u8>> / list<tuple<…>> /
    // bare list<u8> / list<list<u8>> (graph.neighbors) all encode uniformly with no per-shape branch. A
    // non-spilled op has `result_cref == None`: a scalar result inline, a Unit result void.
    if let Some(cref) = &result_cref {
        item.push(0x00); // one result, unnamed
        crate::backend::wasm::wit_ctype::encode_cref(cref, &mut item);
    } else {
        match h.result {
            Some(r) => item.extend_from_slice(&[0x00, r.comp_byte()]),
            None => item.extend_from_slice(&[0x01, 0x00]),
        }
    }
    item
}

/// Build the SPILLED-RESULT component defined types for a host-import set, GENERALLY, from each op's carried
/// WIT result type — the single mechanism that replaces the former per-shape (`option`/`pairs`/`bytes`)
/// blocks. Returns `(needs_list, result_defs, result_crefs)`:
///  • `needs_list` — whether the import instance-type must prepend the shared `(list u8)` defined type at
///    index 0 (a `list<u8>` PARAM, or any spilled result — every admitted result bottoms out at `list<u8>`).
///  • `result_defs` — the component defined-type item bytes for the RESULT types, laid RIGHT AFTER `(list u8)`
///    (instance-type indices `1..`), children-first and DEDUPED (`wit_ctype::add_wit_type_deduped` interns
///    every `list<u8>` leaf to the shared index 0, and shares structurally-equal subtypes).
///  • `result_crefs` — per host op (index-aligned with `host_imports`), the `CRef` its functype result
///    references (an inline primitive OR a defined-type index), or `None` for a scalar/unit result.
/// The indices in `result_crefs`/`result_defs` are ABSOLUTE instance-type type indices (index 0 = the shared
/// `(list u8)`), so `host_op_comp_functype` and `host_effect_instance_type` agree without threading ad-hoc
/// per-shape offsets. Reproduces the former fixed indices for the three original shapes (bare list<u8> → 0,
/// option → 1, list<tuple> → 2+has_option) and generalizes to any structural list/tuple nesting (e.g.
/// graph.neighbors' `list<list<u8>>`).
#[allow(clippy::type_complexity)] // (needs_list, [(def, nominal)], [result CRef], [[list-arg CRef]])
pub(super) fn build_host_result_types(
    db: &mut Db,
    host_imports: &[host::HostImport],
) -> (
    bool,
    Vec<(Vec<u8>, bool)>,
    Vec<Option<crate::backend::wasm::wit_ctype::CRef>>,
    // Per op, per HostParam position: the `CRef` a `list<T>` ARG references (its `(list <elem>)` DEFINED type
    // in the SAME instance-type table as the results, deduped). `None` for a non-list param.
    Vec<Vec<Option<crate::backend::wasm::wit_ctype::CRef>>>,
) {
    use crate::backend::wasm::wit_ctype::{CDef, CRef, add_wit_type_deduped, emit_cdef};
    use crate::wit_world::WitType;
    // A `list<u8>` PARAM (Bytes), any spilled result, a `list<T>` param whose element reaches `list<u8>`, OR a
    // RECORD param whose field reaches `list<u8>` (a `Bytes`/`option<bytes>`/`list<bytes>` field) all need the
    // shared `(list u8)` at index 0 (the record field's cref references it).
    let has_list_param = host_imports.iter().any(|h| {
        h.params.iter().any(|p| match p {
            host::HostParam::Bytes => true,
            host::HostParam::List(e) => host::record_field_abi_reaches_bytes(e),
            host::HostParam::Record(fields) => fields
                .iter()
                .any(|(_, f)| host::record_field_abi_reaches_bytes(f)),
            _ => false,
        })
    });
    let has_spilled = host_imports.iter().any(|h| h.spilled_result.is_some());
    let needs_list = has_list_param || has_spilled;
    let mut table: Vec<CDef> = Vec::new();
    let mut memo: Vec<(WitType, CRef)> = Vec::new();
    if needs_list {
        // `(list u8)` is instance-type index 0 — shared by every `list<u8>` PARAM and every `list<u8>` leaf
        // of a result. Seed the dedup memo so a result's `list<u8>` sub-type interns to it rather than
        // re-defining it.
        table.push(CDef::List(CRef::Prim(wasm_abi::COMP_U8)));
        memo.push((WitType::List(Box::new(WitType::U8)), CRef::Idx(0)));
    }
    let mut result_crefs = Vec::with_capacity(host_imports.len());
    // Per op, per HostParam position: a `list<T>` arg's `(list <elem>)` CRef (built into the SAME table, so a
    // `list<list<u8>>` arg + a `list<list<u8>>` result share one defined type). `None` for a non-list param.
    let mut arg_list_crefs: Vec<Vec<Option<CRef>>> = Vec::with_capacity(host_imports.len());
    for h in host_imports {
        let cref = h.spilled_result.as_ref().and_then(|ty| {
            // Prefer the WORLD's declared result WitType (the authoritative host contract) so a nominal err
            // arm follows the host's `variant`/`enum` CONSTRUCTOR (the #3228 rule, result-side — else
            // `result<_, enum>` silently fails to instantiate against a host `result<_, variant>`). Falls back
            // to the guest-`Ty`-derived type when the world is absent/undecodable; byte-neutral for a
            // STRUCTURAL result (the two views coincide) and corrective only for a variant/enum arm.
            let wt = host::wit_op_result_type(db, &h.effect, &h.op)
                .or_else(|| host::spilled_result_wit_type(db, ty))?;
            add_wit_type_deduped(&wt, &mut table, &mut memo)
        });
        // A payloadless `enum` RESULT (by-value i32) references its `enum` DEFINED type by the SAME
        // `result_cref` mechanism a spilled compound uses — prefer the WORLD's declared result type (so the
        // host's `enum`/`variant` CONSTRUCTOR is followed), else the guest case names. The core functype
        // still returns i32 (serialize `host_import_functype`); only the COMPONENT result type is nominal.
        let cref = cref.or_else(|| {
            let cases = h.enum_result.as_ref()?;
            let wt = host::wit_op_result_type(db, &h.effect, &h.op)
                .unwrap_or_else(|| WitType::Enum(cases.clone()));
            add_wit_type_deduped(&wt, &mut table, &mut memo)
        });
        result_crefs.push(cref);
        // A `list<T>` ARG references a `(list <elem>)` DEFINED type — build it from the WORLD's declared param
        // WIT type (the authoritative host contract), aligned with the HostParam positions (a `Unit` arg is
        // elided from BOTH the WIT params and the HostParams, so positions stay 1:1). Built into the shared
        // table (deduped with results + each other).
        let wit_params = host::wit_op_param_types(db, &h.effect, &h.op);
        let mut per_param: Vec<Option<CRef>> = vec![None; h.params.len()];
        for (i, p) in h.params.iter().enumerate() {
            if matches!(p, host::HostParam::List(_))
                && let Some(pw) = wit_params.as_ref().and_then(|ps| ps.get(i))
            {
                per_param[i] = add_wit_type_deduped(pw, &mut table, &mut memo);
            }
        }
        arg_list_crefs.push(per_param);
    }
    // EXPORT-AWARE INDEXING: a NOMINAL defined type (`variant`/`enum`/`record`) that an import func's type
    // references must be EXPORTED from the instance-type (component-model rule — the same the ARG-side records
    // obey; a structural `list`/`option`/`result`/`tuple` is anonymous-allowed). `add_wit_type_deduped` laid
    // the table FLAT (one index per def, `CRef::Idx(i)` = table position `i`); a nominal def instead takes TWO
    // instance-type slots (define + export), so it SHIFTS every later index. Compute each entry's REFERENCE
    // index (the export index for a nominal, the define index otherwise), then REMAP every `CRef::Idx` in the
    // defs + the per-op result `CRef`s so a result's err-enum reference points at the EXPORTED enum. Without
    // this a `result<list<u8>, enum>` (run.run) emits an unexported enum → "instance not valid as import".
    let mut ref_idx = vec![0u32; table.len()];
    let mut nominal = vec![false; table.len()];
    let mut cur = 0u32;
    for (i, def) in table.iter().enumerate() {
        let is_nom = cdef_is_nominal(def);
        nominal[i] = is_nom;
        if is_nom {
            ref_idx[i] = cur + 1; // define at `cur`, export at `cur+1` (the index references use)
            cur += 2;
        } else {
            ref_idx[i] = cur;
            cur += 1;
        }
    }
    let remap = |c: &CRef| match c {
        CRef::Idx(i) => CRef::Idx(ref_idx[*i as usize]),
        CRef::Prim(b) => CRef::Prim(*b),
    };
    for c in result_crefs.iter_mut().flatten() {
        *c = remap(c);
    }
    for c in arg_list_crefs.iter_mut().flatten().flatten() {
        *c = remap(c);
    }
    // The instance-type DEFINED types are `table[start..]` (index 0 is the shared `(list u8)` the instance-type
    // lays as its own prepend) — spilled-result types AND `list<T>`-arg types (deduped together). Emit each
    // with its CRefs remapped to the export-aware indices; pair with whether it is nominal (define+export).
    let start = if needs_list { 1 } else { 0 };
    let result_defs: Vec<(Vec<u8>, bool)> = table[start..]
        .iter()
        .enumerate()
        .map(|(k, def)| (emit_cdef(&remap_cdef(def, &remap)), nominal[start + k]))
        .collect();
    (needs_list, result_defs, result_crefs, arg_list_crefs)
}

/// Whether a component defined type is NOMINAL — a `record`/`variant`/`enum`/`flags` that, when used by an
/// import func's type, MUST be exported from the instance-type (unlike an anonymous-allowed structural
/// `list`/`option`/`result`/`tuple`). The ARG-side records obey the same rule (they lay define+export).
pub(super) fn cdef_is_nominal(def: &crate::backend::wasm::wit_ctype::CDef) -> bool {
    use crate::backend::wasm::wit_ctype::CDef;
    matches!(
        def,
        CDef::Record(_) | CDef::Variant(_) | CDef::Enum(_) | CDef::Flags(_)
    )
}

/// Rewrite every `CRef::Idx` a `CDef` holds through `remap` (leaving `CRef::Prim` untouched) — used to shift
/// a defined type's child references onto the EXPORT-AWARE instance-type indices when a nominal def inserts an
/// extra export slot ahead of it.
pub(super) fn remap_cdef(
    def: &crate::backend::wasm::wit_ctype::CDef,
    remap: &impl Fn(&crate::backend::wasm::wit_ctype::CRef) -> crate::backend::wasm::wit_ctype::CRef,
) -> crate::backend::wasm::wit_ctype::CDef {
    use crate::backend::wasm::wit_ctype::CDef;
    match def {
        CDef::Record(fields) => {
            CDef::Record(fields.iter().map(|(n, c)| (n.clone(), remap(c))).collect())
        }
        CDef::Variant(cases) => CDef::Variant(
            cases
                .iter()
                .map(|(n, c)| (n.clone(), c.as_ref().map(remap)))
                .collect(),
        ),
        CDef::Tuple(elems) => CDef::Tuple(elems.iter().map(remap).collect()),
        CDef::Option(c) => CDef::Option(remap(c)),
        CDef::Result { ok, err } => CDef::Result {
            ok: ok.as_ref().map(remap),
            err: err.as_ref().map(remap),
        },
        CDef::List(c) => CDef::List(remap(c)),
        CDef::Enum(_) | CDef::Flags(_) => def.clone(),
    }
}

/// Build the [`envelope::HostGroup`] for ONE host interface's ops — its FQ WIT import name, the component
/// DEFINED types those ops reference (shared `(list u8)`, the spilled-result defs, and a single record OR enum
/// param type, all LOCAL to this interface's own instance-type index space), and each op's component functype
/// referencing those local indices. This is exactly the single-interface computation, SCOPED to one interface
/// — so a reducer performing ops from N interfaces gets N self-contained instance-types, each structurally
/// matching its host (no spurious cross-interface type). The per-interface decline conditions (multi-record,
/// record+enum mix, record+spilled-result, string+record) match the single-interface path, now evaluated PER
/// interface (so a record in one interface + an enum in another compose — they never share an instance-type).
pub(super) fn build_host_group(
    db: &mut Db,
    world_bytes: &[u8],
    effect: &str,
    group: &[host::HostImport],
) -> Result<envelope::HostGroup, Reject> {
    use crate::backend::common::export_name::kebab_extern_name;
    // The host interface's FQ WIT name (the import extern name): the world import whose last `/`-segment
    // kebab-matches the effect (the same match `is_world_import_op` uses).
    let host_iface = {
        let arenas = crate::codec::decode(world_bytes).ok_or_else(|| {
            Reject::decline("the target world did not decode for the host-import interface lookup")
        })?;
        let world = crate::wit_world::parse_target_world(&arenas, arenas.root)
            .ok_or_else(|| Reject::decline("the target world did not parse"))?;
        let ek = kebab_extern_name(effect);
        world
            .imports
            .iter()
            .find(|i| kebab_extern_name(i.name.rsplit('/').next().unwrap_or(&i.name)) == ek)
            .map(|i| i.name.clone())
            .ok_or_else(|| {
                Reject::decline(
                    "the performed host effect has no matching import interface in the world",
                )
            })?
    };
    let (needs_list, result_defs, result_crefs, arg_list_crefs) =
        build_host_result_types(db, group);
    let has_spilled_result = group.iter().any(|h| h.spilled_result.is_some());
    let record_params: Vec<&Vec<(String, host::RecordFieldAbi)>> = group
        .iter()
        .flat_map(|h| &h.params)
        .filter_map(|p| match p {
            host::HostParam::Record(fields) => Some(fields),
            _ => None,
        })
        .collect();
    let enum_params: Vec<&Vec<String>> = group
        .iter()
        .flat_map(|h| &h.params)
        .filter_map(|p| match p {
            host::HostParam::Enum(cases) => Some(cases),
            _ => None,
        })
        .collect();
    let variant_params: Vec<
        &Vec<(
            String,
            Option<crate::backend::wasm::runtime_abi::AbiValType>,
        )>,
    > = group
        .iter()
        .flat_map(|h| &h.params)
        .filter_map(|p| match p {
            host::HostParam::Variant(cases) => Some(cases),
            _ => None,
        })
        .collect();
    // NOMINAL param types laid AFTER `(list u8)` (index 0 if present) and the spilled-result / list-arg defs.
    // `base` is the first record-param type's instance-type index. `record_defs` accumulates EVERY record's
    // defined types (children-first, laid as define+export by `host_effect_instance_type`); `op_nominal[i]` is
    // op `i`'s nominal EXPORTED type index (`0` = no nominal param — unused by `host_op_comp_functype`).
    // COUNT INSTANCE-TYPE SLOTS, not entries: `host_effect_instance_type` lays a NOMINAL result/list-arg def
    // as define+export (TWO slots) and a STRUCTURAL one as define-only (ONE slot). Using `result_defs.len()`
    // (an entry count) undercounts by one per nominal result-def, so the record-param path's `base + 2*i`
    // indices land one short — the record's field then references a nominal child's DEFINE index instead of
    // its EXPORT index (e.g. a `record{ v: variant }` param whose interface ALSO has a list<variant> arg:
    // the variant's export slot is uncounted → the field refs the raw variant, "instance not valid as import").
    let base = needs_list as u32
        + result_defs
            .iter()
            .map(|(_, is_nominal)| if *is_nominal { 2u32 } else { 1 })
            .sum::<u32>();
    let mut record_defs: Vec<Vec<u8>> = Vec::new();
    let mut op_nominal: Vec<u32> = vec![0; group.len()];
    // At most ONE nominal-param KIND per interface this slice (record OR enum OR bare-variant) — the shared
    // `op_nominal`/`nominal_type_idx` path carries a single nominal param type per op, and their defined types
    // would otherwise contend for the same `base` slots. Mixing declines rather than mis-laying the group.
    let nominal_kinds = [
        !record_params.is_empty(),
        !enum_params.is_empty(),
        !variant_params.is_empty(),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if nominal_kinds > 1 {
        return Err(Reject::unsupported(
            "a host interface mixing more than one nominal parameter kind (record / enum / bare-variant) is \
             not supported (one kind per interface)",
        ));
    } else if !record_params.is_empty() {
        let has_str_param = group
            .iter()
            .flat_map(|h| &h.params)
            .any(|p| matches!(p, host::HostParam::Str));
        // A record param whose field reaches `list<u8>` (a `Bytes`/`option<bytes>`/`list<bytes>` field) no
        // longer needs a SIBLING `list<u8>` param: `build_host_result_types`'s `has_list_param` now prepends
        // the shared `(list u8)` (index 0) for such a record, which the field's cref references. Only a string
        // param or a spilled compound result still declines the record-arg composition this slice.
        if has_str_param || has_spilled_result {
            return Err(Reject::unsupported(
                "a record host-argument composes only in a host interface with no string parameter and no \
                 option/list/bytes compound result",
            ));
        }
        // MULTIPLE record params per interface (deliver's message + response): lay EACH op's record type into
        // the SHARED table (its indices continue past the prior records', children-first), and thread THAT
        // op's own top-record EXPORT index into its functype. `build_record_import_types` computes indices
        // from `base + 2*record_defs.len()`, so accumulating across ops keeps every reference absolute.
        for (i, hi) in group.iter().enumerate() {
            if let Some(fields) = hi.params.iter().find_map(|p| match p {
                host::HostParam::Record(f) => Some(f),
                _ => None,
            }) {
                op_nominal[i] = build_record_import_types(fields, 0, base, &mut record_defs);
            }
        }
    } else if !enum_params.is_empty() {
        let distinct = enum_params.iter().all(|c| *c == enum_params[0]);
        if !distinct {
            return Err(Reject::unsupported(
                "a host interface with more than one distinct enum parameter type is not supported (one \
                 enum type per interface)",
            ));
        }
        // A SINGLE shared `enum` DEFINE (at `base`) + EXPORT (at `base+1`); every enum op references it.
        record_defs.push(crate::backend::wasm::wit_ctype::emit_cdef(
            &crate::backend::wasm::wit_ctype::CDef::Enum(enum_params[0].clone()),
        ));
        let enum_export = base + 1;
        for (i, hi) in group.iter().enumerate() {
            if hi
                .params
                .iter()
                .any(|p| matches!(p, host::HostParam::Enum(_)))
            {
                op_nominal[i] = enum_export;
            }
        }
    } else if !variant_params.is_empty() {
        let distinct = variant_params.iter().all(|c| *c == variant_params[0]);
        if !distinct {
            return Err(Reject::unsupported(
                "a host interface with more than one distinct bare-variant parameter type is not supported \
                 (one variant type per interface)",
            ));
        }
        // A SINGLE shared `variant` DEFINE (at `base`) + EXPORT (at `base+1`); every variant-param op refs it.
        // Each case is a nullary or a scalar-payload inline primitive CRef — the same `CDef::Variant` a
        // record-field variant lays (`record_field_cref`'s Variant arm), now at the top-level param position.
        let vcases: Vec<(String, Option<crate::backend::wasm::wit_ctype::CRef>)> = variant_params
            [0]
        .iter()
        .map(|(name, p)| {
            (
                name.clone(),
                p.map(|v| crate::backend::wasm::wit_ctype::CRef::Prim(v.comp_byte())),
            )
        })
        .collect();
        record_defs.push(crate::backend::wasm::wit_ctype::emit_cdef(
            &crate::backend::wasm::wit_ctype::CDef::Variant(vcases),
        ));
        let variant_export = base + 1;
        for (i, hi) in group.iter().enumerate() {
            if hi
                .params
                .iter()
                .any(|p| matches!(p, host::HostParam::Variant(_)))
            {
                op_nominal[i] = variant_export;
            }
        }
    }
    let host_fns: Vec<envelope::HostFn> = group
        .iter()
        .enumerate()
        .map(|(i, hi)| envelope::HostFn {
            op: hi.op.clone(),
            comp_functype: host_op_comp_functype(
                hi,
                0,
                op_nominal[i],
                &arg_list_crefs[i],
                result_crefs[i].clone(),
            ),
            has_list_param: hi
                .params
                .iter()
                .any(|p| matches!(p, host::HostParam::Bytes)),
            core_functype: Vec::new(),
        })
        .collect();
    Ok(envelope::HostGroup {
        effect_iface: host_iface,
        host_fns,
        needs_list,
        result_defs,
        record_defs,
    })
}

/// Declare into `used` the value-heap runtime ops that `select::emit_result_lift` emits when lifting a
/// SPILLED-COMPOUND host result of type `ty` — in lockstep with that recursion, so the import section
/// carries exactly what the lift calls (a missing op resolves to an out-of-range func index → invalid
/// module). A `Bytes`/`String` leaf copies via `bytes-alloc`/`bytes-set`; a `List` allocs + `vec-of-arr`s;
/// a `Tuple`/`Record` allocs an aggregate; an option-shaped sum builds via `sum-new`. Recurses into element/
/// field/payload types. Replaces the former per-shape (`option`/`list-pairs`) op declarations.
pub(super) fn declare_result_lift_ops(
    db: &mut Db,
    ty: &crate::ty::Ty,
    used: &mut std::collections::BTreeSet<&'static str>,
) {
    use crate::ty::Ty;
    match ty.strip_nominal().clone() {
        Ty::Bytes | Ty::String => {
            used.insert("bytes-alloc");
            used.insert("bytes-set");
        }
        Ty::List(e) => {
            used.insert("arr-alloc");
            used.insert("arr-set");
            used.insert("vec-of-arr");
            declare_result_lift_ops(db, &e, used);
        }
        Ty::Tuple(elems) => {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for e in elems.iter() {
                declare_result_lift_ops(db, e, used);
            }
        }
        Ty::Record(fields) => {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for f in fields.values() {
                declare_result_lift_ops(db, f, used);
            }
        }
        // A SCALAR leaf (bool/char/aliased int/float, or a `Qty` over one) — the lift boxes it into a
        // value-heap cell: bool → `box-bool`, int/char → `box-int`, f64 → `box-float`, f32 → `box-float32`.
        Ty::Bool => {
            used.insert("box-bool");
        }
        Ty::Char | Ty::Int(_) => {
            used.insert("box-int");
        }
        Ty::Float(ft) if ft.ground_width() == 64 => {
            used.insert("box-float");
        }
        Ty::Float(ft) if ft.ground_width() == 32 => {
            used.insert("box-float32");
        }
        Ty::Qty { inner, .. } => declare_result_lift_ops(db, &inner, used),
        // A `result<list<u8>, enum>` (run.run): `sum-new` builds both the Ok and Err value-heap arms;
        // `box-int` boxes the err enum's discriminant into an int cell (the guest's enum-disc-as-payload rep);
        // the Ok arm lifts a `Bytes` payload. Checked BEFORE the option shape (a result is not option-shaped).
        _ if host::result_bytes_enum(db, ty).is_some() => {
            used.insert("sum-new");
            used.insert("box-int");
            declare_result_lift_ops(db, &crate::ty::Ty::Bytes, used); // the Ok `list<u8>` payload
        }
        // An option-shaped sum (`option<T>`): `sum-new` for the Some/None construction + the Some arm's
        // payload lift ops, recursively (general over the payload, not pinned to `Bytes`).
        _ if host::option_payload_ty(db, ty).is_some() => {
            if let Some(payload) = host::option_payload_ty(db, ty) {
                used.insert("sum-new");
                declare_result_lift_ops(db, &payload, used);
            }
        }
        // A general VARIANT result (`emit_variant_sum_lift`): `sum-new` builds every case's arm; each payload
        // case's payload is lifted by its OWN ops (recursed via `declare_result_lift_ops` — a scalar boxes, a
        // compound `list<u8>`/`record`/… recurses its own alloc/set/copy ops). Mirrors the lift so its
        // `CallImport`s resolve (else u32::MAX → an invalid module — the arg-side trap's twin).
        _ => {
            if let Some(cases) = host::variant_liftable_payload_cases(db, ty) {
                used.insert("sum-new");
                for (i, (_, has_payload)) in cases.iter().enumerate() {
                    if *has_payload
                        && let Some(pt) =
                            crate::backend::wasm::select::variant_payload_ty_at(db, ty, i as u32)
                    {
                        declare_result_lift_ops(db, &pt, used);
                    }
                }
            }
        }
    }
}

/// Convert a host-op boundary parameter to the peer-boundary `AbiValType` (U2) — a scalar crosses by its
/// scalar rep; a `value`-handle (a compound) already carries its `AbiValType::U32`. A host STRING param
/// (`HostParam::Str`, the `(ptr,len)` shared-memory shape) has no peer form yet → `None` (a String-param
/// peer op declines this increment, like the extern side; a compound String crosses as a `u32` handle,
/// which `collect_host_imports` already records as a scalar-ABI param when the type is a heap value).
pub(super) fn host_param_abi(p: &host::HostParam) -> Option<runtime_abi::AbiValType> {
    match p {
        host::HostParam::Scalar(v) => Some(*v),
        // Str and Bytes both use the `(ptr,len)` shared-memory shape, which has no scalar peer-ABI form —
        // a String/Bytes-param PEER op declines this increment (the host-arg support is host-only). A RECORD
        // param (shape d) likewise has no scalar peer-ABI form — a record crosses a PEER boundary as its
        // `u32` heap handle, not this native host flatten (and the classifier only produces `Record` for a
        // non-peer-bound op), so it declines here too.
        // An ENUM param likewise has no scalar peer-ABI form (a peer-bound enum crosses as its `u32` handle,
        // and the classifier only produces `Enum` for a non-peer-bound host op) → declines here.
        // A `list<T>` param likewise has no scalar peer-ABI form (a peer-bound list crosses as its `u32`
        // handle, and the classifier only produces `List` for a non-peer-bound host op) → declines here.
        host::HostParam::Str
        | host::HostParam::Bytes
        | host::HostParam::Record(_)
        | host::HostParam::Enum(_)
        | host::HostParam::List(_)
        | host::HostParam::Variant(_) => None,
    }
}

/// Convert a host-import set into the `ExternImport`-shaped slice the resource core-module builder
/// (`runtime_resource_core_module_form_ex2`) takes for its LEADING ops — the host effect name stands in for
/// the peer interface, and each scalar param maps through `host_param_abi`. Used by the host-resource-escape
/// arms with `leading_is_host = true`, which flips the import module to `"host"`. BYTE-IDENTICAL to a host
/// functype for a SCALAR op (a String param would be dropped by `host_param_abi` → the caller declines a
/// string-param host op up front via `set_needs_memory`, so no arity mismatch reaches here).
pub(super) fn host_as_extern_for(host_imports: &[host::HostImport]) -> Vec<host::ExternImport> {
    host_imports
        .iter()
        .map(|hi| host::ExternImport {
            interface: hi.effect.clone(),
            op: hi.op.clone(),
            params: hi.params.iter().filter_map(host_param_abi).collect(),
            result: hi.result,
        })
        .collect()
}

/// The COMPONENT functype of a cross-component extern op (X4b) — the shape declared in the peer
/// interface's instance-type AND in the boundary the consumer imports against. Param NAMES are `p0,p1,…`
/// (the convention `assemble_extern` + a matching provider both use — a component-model interface import
/// checks param names structurally). Scalar params/result this increment (a `value` handle is X5).
pub(super) fn extern_op_comp_functype(e: &host::ExternImport) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, p) in e.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&(pname.len() as u8).to_le_bytes());
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(p.comp_byte());
    }
    item.extend_from_slice(&encode::wasm_vec(e.params.len(), &param_items));
    match e.result {
        Some(r) => item.extend_from_slice(&[0x00, r.comp_byte()]),
        None => item.extend_from_slice(&[0x01, 0x00]),
    }
    item
}
