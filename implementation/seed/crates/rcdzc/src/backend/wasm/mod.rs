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

pub mod dwarf;
pub mod encode;
pub mod envelope;
pub mod host;
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
use crate::backend::wasm::select::{SelectedFunc, select_function_of};
use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// Emit a WebAssembly component for the program in `db` under the boundary `layout`. Selects each
/// definition in the layout's emission order, serializes the core module, and assembles the envelope.
///
/// `spans` (Mode E of `DESIGN-debug-info-rcdzc.md`) — when `Some`, appends the wasm `name` (D0) + the
/// `.debug_*` DWARF (D2) custom sections to the embedded core module, drawing source positions from the
/// side-table. Inert and strippable, so a debug component stripped of custom sections is byte-identical
/// to the `None` component (§5). `None` is byte-for-byte today's output. BOTH the ordinary multi-export
/// path AND the resource-escape shapes (a nullary compound export) carry DWARF: each resource emitter
/// (`emit_runtime_resource`/`_sum_`/`_bytes_`) threads `spans` into `append_debug_sections` over its
/// user bodies — which lead the escape core's code section — so a compound-returning program is
/// debuggable too (the synthesized `make`/`t-encode` walker funcs have no `src_body`, so they get no
/// row). A fully-CONSTANT compound bakes its bytes with no user body, so it carries no `.debug_*`.
pub fn emit(
    db: &mut Db,
    layout: &Layout,
    spans: Option<&crate::spans::SpanData>,
    external_debug_info: Option<&str>,
) -> Result<Vec<u8>, Reject> {
    // The RESOURCE ESCAPE path (`DESIGN-value-heap-rcdzc.md` §3a), detected BEFORE selection: a single
    // nullary export returning a COMPOUND crosses as a component-model resource whose `encode() ->
    // list<u8>` yields the canonical binary value form. For a fully-CONSTANT compound (R1) the value is
    // known at compile time, so its bytes are baked into the resource core module (no runtime heap
    // construction, no selection of a compound-returning body — which would decline at `select`) and the
    // whole component takes the resource shape, a different envelope than the multi-export boundary. A
    // RUNTIME compound (elements computed at run time) crosses through the SAME resource shape but its
    // `encode()` WALKS the live handle from the value-form template (R2) instead of baking bytes; it is
    // routed just below. Only the single nullary-export compound case takes the resource shape; any
    // other compound host-return (multi-export, parameterized) falls through and declines below.
    if let [e] = &layout.exports[..]
        && e.params.is_empty()
        && matches!(
            e.result,
            crate::ty::Ty::Tuple(_)
                | crate::ty::Ty::Record(_)
                | crate::ty::Ty::Sum { .. }
                | crate::ty::Ty::List(_)
                // A MAP export crosses via the resource escape like any other heap value — its CONSTANT
                // bytes (the sorted canonical `(map (k v) …)` value form) baked into the resource module.
                // A constant `Map.insert` chain / `(map …)` literal folds to a `Core::MapNew` whose
                // `const_value_ast` renders sorted; a genuinely RUNTIME map has no constant form →
                // `constant_value_form` returns None → falls through to the decline below (the looping
                // map-escape walker, like a list's, is a later increment).
                | crate::ty::Ty::Map(_, _)
                // A SET export crosses via the resource escape too — its CONSTANT bytes (the sorted
                // canonical `(Set.of (list …))` value form) baked into the resource module; a runtime set
                // has no constant form and falls through to the decline (the looping walker is later).
                | crate::ty::Ty::Set(_)
                | crate::ty::Ty::Bytes
                // A bare `String` export has no scalar boundary valtype (a string is a heap value, not an
                // i32/i64/f64), so it crosses via the resource escape like any other heap value — its
                // CONSTANT bytes baked into the resource module (the value form `(: "…" String)`, the same
                // bytes a `(Some "hi")` payload already bakes). A RUNTIME string (a byte-rope built at run
                // time) has no constant form; `constant_value_form` returns None and it falls through to
                // the decline below (the looping string-escape walker is a later increment, like a list).
                | crate::ty::Ty::String
                // A NOMINAL newtype export crosses as its ERASED underlying value (the tag adds nothing to
                // the runtime representation): a nominal-over-COMPOUND takes this resource escape (its
                // constant/runtime value form is the underlying value, with `type_ast` tagging it under the
                // nominal NAME); a nominal-over-scalar has a scalar boundary valtype and is handled by the
                // scalar path below (where `export_result_valtype` strips the tag), so both cross faithfully.
                | crate::ty::Ty::Nominal { .. }
                // A QUANTITY export renders its FULL value form `(: (Qty.of <v> <unit>) (Qty T u))` so the
                // dimensional type is OBSERVABLE (units-of-measure.md — the corpus records the quantity, not
                // the bare erased number). Its inner value erases to a scalar, so `comp_valtype_of(Qty)`
                // would send it down the scalar path (losing the unit); routing it through the resource
                // escape here bakes the quantity's constant value form (the unit lives only in these baked
                // bytes — the emitted numeric is byte-identical, the type layer is compile-time-only).
                | crate::ty::Ty::Qty { .. }
        )
    {
        let body = def_body(db, e.def)?;
        if let Some(value_bytes) = crate::lower::constant_value_form(db, body) {
            let main_core = serialize::resource_core_module(&value_bytes);
            let dtor_core = serialize::resource_dtor_module();
            return Ok(envelope::assemble_resource(&main_core, &dtor_core));
        }
        // A LIST result whose value is NOT constant-foldable (a runtime-built list) has no baked-bytes
        // form, and there is no runtime value-form template for a list yet (its length is dynamic, so the
        // `encode()` walker would need to LOOP — a later increment). It is not a sum, and
        // `runtime_value_form_template` returns `None` for a list, so it falls through to the decline
        // below — an honest "runtime list return not yet supported", not a miscompile.
        // A SUM result crosses through the resource shape but its `encode()` SWITCHES on the runtime
        // discriminant (`sum-disc`) and renders the matching variant — a per-variant template, not a
        // single flat one. Route through the sum escape when the sum has a value-form (`None` — a
        // variant with a non-renderable payload — falls through to decline below).
        if let crate::ty::Ty::Sum { .. } = &e.result {
            if let Some(sum_tpl) = crate::lower::sum_form_template(db, &e.result) {
                return emit_runtime_sum_resource(db, layout, e.def, &sum_tpl, spans);
            }
            // A RECURSIVE sum (a self-referential payload — a linked list, a tree) has no fixed
            // per-variant template (`sum_form_template` returned `None`), so its `encode()` walks the heap
            // spine to a runtime-determined depth. Build the SHAPE DESCRIPTOR and route through the
            // runtime `value-encode` op: the encode body bakes the descriptor as a heap Bytes, calls
            // `value-encode(rep, desc)` to get the value-form document, and copies it out
            // (`DESIGN-recursive-sum-escape-walker.md`, approach C).
            if let Some(desc) = crate::lower::sum_shape_descriptor(db, &e.result) {
                return emit_recursive_sum_resource(db, layout, e.def, &desc, spans);
            }
        } else if matches!(e.result, crate::ty::Ty::Bytes) {
            // A RUNTIME `Bytes` result (a `concat`/recursion-built sequence — not a compile-time constant)
            // crosses through the resource shape, but its value form is VARIABLE-length: `encode()` LOOPS,
            // writing the static prefix, the runtime `bytes-len` as a LEB, a `bytes-get` copy loop, then
            // the static suffix (`DESIGN-runtime-bytes-escape-walker.md`). The FIRST looping walker.
            if let Some(form) = crate::lower::runtime_bytes_form(db) {
                return emit_runtime_bytes_resource(db, layout, e.def, &form, spans);
            }
        } else if let Some(tpl) = crate::lower::runtime_value_form_template(&e.result) {
            // A RUNTIME compound (not constant-foldable — a recursive return, a call whose result is
            // built on the heap) crosses through the SAME resource shape, but its `encode()` WALKS the
            // live handle rather than baking constant bytes (R2). Build the value-form TEMPLATE for the
            // result type; if it has one, route through `assemble_runtime_resource`.
            return emit_runtime_resource(db, layout, e.def, &tpl, spans);
        }
    }

    // CLOSURE-RESOURCE escape (`DESIGN-closure-host-resource-rcdzc.md`, C-HOST-1): a SINGLE export whose
    // RESULT is a closure type `(-> A… R)` crosses as a component-model resource with a `call` method the
    // host invokes. UNLIKE the value-escape above (a nullary-COMPOUND trigger), this fires on a `Ty::Fn`
    // RESULT — the export MAY take parameters (the closure is computed from the host's inputs), and its
    // body lowers NORMALLY (the `Core::Closure` builds the cell); the `resource.new` is spliced at the
    // boundary-return by the closure-resource core module. First cut: scalar-aliased closure args/result.
    if let [e] = &layout.exports[..]
        && let crate::ty::Ty::Fn(_, _) = &e.result
    {
        return emit_closure_resource(db, layout, e.def, &e.result, spans);
    }

    // DIRECTION 2 (host hands a closure BACK — not yet built): an export whose PARAMETER is a closure
    // type `(-> …)` — the host would pass a closure resource in, and the body applies it. This needs a
    // signature-derived `call_indirect` type (a host-origin closure has no in-program lifted lambda for
    // `closure_type_index` to match) + the `own<closure>` param ABI (recover the cell via `resource.rep`).
    // Until then, DECLINE with a message that names the feature (rather than the internal "no matching
    // function type" the select-time `Core::CallClosure` would surface). Applies to any export with a
    // closure-typed parameter (single- or multi-export). Reject-don't-miscompile: a clean Todo.
    if layout
        .exports
        .iter()
        .flat_map(|e| e.params.iter())
        .any(|(_, t)| matches!(t, crate::ty::Ty::Fn(_, _)))
    {
        return Err(Reject::decline(
            "a closure passed AS A PARAMETER to an export (the host handing a closure back) is not yet \
             supported — it needs a signature-derived indirect-call type and the own<closure> parameter \
             ABI (DESIGN-closure-host-resource-rcdzc.md, Direction 2 / C-HOST-4)",
        ));
    }

    // The per-program runtime IMPORT SET must be fixed BEFORE selection, because it determines both
    // `layout.import_base` (the shift a defined func's index takes) and the index a `CallImport`
    // resolves to. Walk every reachable body's core for the value-heap ops it will emit
    // (`collect_used_ops`, which mirrors `select`'s op choices exactly), collect them into a
    // deterministic sorted set, and resolve each to its generated `RtOp`. Empty for a program that uses
    // no runtime op — no import section, no shift → byte-identical to a runtime-free build.
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_module_used_ops(db, layout, &mut used)?;
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;

    // The per-program HOST-import set (E2h-2) — every host-delegated operation a reachable body performs
    // (a `Core::HostCall`), in first-encountered order. Like the runtime set, it must be fixed BEFORE
    // selection (it fixes the host-op call index a `Core::HostCall` resolves to) and it shifts the
    // defined-func index space. This increment supports HOST-ONLY programs: a program mixing host + value-
    // heap runtime imports declines below (the two index spaces compose in a later increment).
    let mut host_imports: Vec<host::HostImport> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        host::collect_host_imports(db, body, &mut host_imports);
    }
    if !host_imports.is_empty() && !imports.is_empty() {
        return Err(Reject::decline(
            "a program that both delegates a host effect AND uses the value-heap runtime is not yet \
             emitted (the host + runtime import index spaces compose in a later increment)",
        ));
    }

    // The layout with the import base fixed to the total import count (host + runtime) — a defined
    // function's absolute index is `import_base + its emission position` (imports occupy `0..import_base`).
    // The host-import ORDER (its `(effect, op)` name pairs) is recorded so a `Core::HostCall` resolves its
    // call index. Both are empty for an import-free program (byte-identical to before).
    let host_order: Vec<(String, String)> = host_imports
        .iter()
        .map(|h| (h.effect.clone(), h.op.clone()))
        .collect();
    // The host-call STRING constants + their data-segment offsets (E2h-string): a host call passing a
    // `string` arg emits `(ptr, len)` pointing into the program's memory, where the string's bytes lie.
    // Collect each distinct constant string a `Core::HostCall` passes and assign a running byte offset
    // (each string laid contiguously). Empty when no host call passes a string (the scalar shape).
    let mut host_strings: Vec<(String, u32)> = Vec::new();
    let mut next_offset: u32 = 0;
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let mut strs = Vec::new();
        collect_host_arg_strings(db, body, &mut strs);
        for s in strs {
            if !host_strings.iter().any(|(v, _)| *v == s) {
                let len = s.len() as u32;
                host_strings.push((s, next_offset));
                next_offset += len;
            }
        }
    }
    let layout = layout
        .with_import_base((imports.len() + host_imports.len()) as u32)
        .with_host_order(host_order)
        .with_host_strings(host_strings);
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
        let params = match layout.export_plan(def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, layout, Some(def))?);
    }
    // LAMBDA-LIFTED closures emit as standalone functions AFTER the def functions (their wasm indices
    // are `import_base + order.len() + slot`, which the funcref element section points at). Each is
    // UNIFORMLY an `(env, param) -> result` function: local slot 0 is the closure CELL (the env — read
    // by `Core::Captured` as `arr-get(local 0, 1+index)`), slot 1 is the lambda's own parameter. So the
    // params list PREPENDS an env parameter (an i32 handle) whose binder key is the lifted body itself
    // (a `StructId` nothing resolves to as a `Core::Param`, so it claims slot 0 without shadowing).
    // The lifted set is fixed at layout time (in table-slot order); empty for a closure-free program.
    for (code, lifted) in layout.lifted.clone().into_iter().enumerate() {
        // The env's type is any type whose machine rep is an i32 HANDLE (the closure cell) — `Ty::Bytes`
        // is a heap-handle leaf (`valtype_of` → i32), used here purely as the "i32 handle" marker for the
        // env slot. The env's slot-map KEY must be a node NOTHING in the body resolves to (the body reads
        // the env only via `Core::Captured` → `local.get 0`, never by name) — so a FRESH synthesized atom,
        // NOT the body occurrence (which would make `select`'s `slots.get(body)` return slot 0 and emit the
        // env instead of the body).
        let env_key = db.push_name("$closure-env");
        let mut params = vec![(env_key, crate::ty::Ty::Bytes)];
        params.extend(lifted.params.iter().cloned());
        // An UNREACHED lifted lambda (demanded during type-checking / a fold that erased it — no reachable
        // `Core::Closure` builds it) is emitted as an inert STUB with the same signature but a trivial body
        // (return a zero of the result type). It is never called (its funcref-table entry is omitted), so a
        // stub keeps the function-index space + type section consistent without carrying the dead lambda's
        // (possibly ill-formed) body. A REACHED lambda selects its real body.
        if layout.lifted_reached.get(code).copied().unwrap_or(true) {
            funcs.push(select_function_of(db, lifted.body, &params, layout, None)?);
        } else {
            funcs.push(select::stub_function(&params, &lifted.ret_ty));
        }
    }

    // Serialize the embedded core module (multi-export core module, functions in emission order). The
    // `name` + `.debug_*` sections are appended by `append_debug_sections` below (both paths, one place).
    // A HOST-delegating program threads its host imports through the core module's import section (from
    // module `"host"`, ahead of any runtime op); an ordinary program takes the runtime-only path
    // (byte-identical to before).
    let mut core = if host_imports.is_empty() {
        serialize::core_module(&funcs, &imports, layout).map_err(Reject::decline)?
    } else {
        serialize::core_module_with_host(&funcs, &imports, &host_imports, layout)
            .map_err(Reject::decline)?
    };

    // DEBUG (Mode E, D2): append the `.debug_*` DWARF custom sections to the embedded core module, so a
    // debugger can STEP through Cadenza source. Function-granularity: one line row + one subprogram DIE
    // per emitted function, its code-offset range from `code_ranges` (D1b) and its source line from the
    // `spans` side-table (D1a). Inert + strippable — appended after the executed sections, so `debug =
    // None` is byte-identical to today and `wasm-tools strip` recovers it. (`name` rode in `core_module`.)
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut core);

    // A lean component paired with a DETACHED DWARF sidecar (Mode S, `Emit(Wasm)` + `Emit(Dwarf)` in one
    // run) carries an `external_debug_info` custom section naming the sidecar, so a debugger auto-loads
    // it (no manual symbol-file flag). Also inert + strippable; appended after the executed sections.
    if let Some(path) = external_debug_info {
        core.extend_from_slice(&dwarf::external_debug_info_section(path));
    }

    // Build the component-boundary export list (each export's parameter + result valtypes) and
    // assemble the envelope. Export `k` in the layout lifts core func `k` (exports first, in order).
    let multi_export = layout.exports.len() > 1;
    let mut boundary: Vec<BoundaryExport> = Vec::new();
    for e in &layout.exports {
        // The export's RESULT crosses as a `BoundaryResult`: unit → None, a scalar → its primitive
        // byte. A COMPOUND host-return does not cross on THIS multi-export path — the single nullary
        // export case took the resource-escape shape above; a compound reaching here declines. The two
        // triggers are DISTINCT and the diagnosis names the actual one (the generic `export_result`
        // message can only say "multi-export", which misdiagnoses a single PARAMETERIZED export — the
        // resource-escape path covers only a NULLARY compound export, so a single export that takes a
        // parameter also declines here). Report the trigger that applies, using the context known here.
        if matches!(
            e.result,
            crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_) | crate::ty::Ty::Sum { .. }
        ) {
            // AMBIGUOUS TYPE first — a result whose payload/element type is an UNRESOLVED variable (a bare
            // `(None)` : `Option ?0`, an empty `(list)` : `List ?0`) has no defined serialization
            // REGARDLESS of export shape. A single NULLARY sum export with an unresolved payload reaches
            // here (the escape guard above tried and `sum_form_template` returned `None`), so it must NOT
            // be diagnosed as an export-shape problem — the shape is fine; the TYPE is undetermined. Report
            // a type error naming the annotation fix (CDZ0203, the type-determination fault code), NOT the
            // parameterized/multi-export message. `e.params.is_empty()` distinguishes it from a
            // parameterized export (whose free var, if any, would still be a shape issue at this stage).
            if e.result.has_free_var() && e.params.is_empty() && !multi_export {
                return Err(Reject::coded(
                    crate::diag::Code::TypeMismatch,
                    format!(
                        "the result type `{}` is not fully determined — annotate it \
                         (e.g. `(: <expr> (Option Int64))`) so its value has a defined form",
                        e.result.render_name()
                    ),
                ));
            }
            let why = if multi_export {
                "a compound result crosses the host boundary only as the single export's result (this program has multiple exports)"
            } else if !e.params.is_empty() {
                // A single PARAMETERIZED export — the resource-escape path covers only a NULLARY compound
                // export, so a compound return from a function that takes a parameter declines here.
                "a compound result escapes to the host as a resource only from a NULLARY export; this export takes a parameter (a parameterized compound return is not yet supported)"
            } else {
                // A single NULLARY export whose compound result reached here — the resource-escape path
                // above TRIED and its value-form template was `None`: the result has no runtime value form
                // yet. This is the RECURSIVE-sum / dynamic-shape case (a self-referential `(type IntList
                // (Cons (Tuple Int64 IntList)) Nil)` built at runtime has an UNBOUNDED static shape, so the
                // `encode()` walker would need to LOOP to a runtime-determined depth — the analogue of the
                // runtime-`Bytes` looping walker, a later increment). The honest reason is the missing
                // walker, NOT a (non-existent) parameter — `main` is nullary. Consuming such a value to a
                // scalar already works; only rendering it as the boundary result is deferred.
                "rendering this compound as the host result needs a value-form walker that loops to a runtime-determined depth (a recursive-sum / dynamic-shape result is not yet emitted); folding it to a scalar works"
            };
            return Err(Reject::decline(format!(
                "returning a {} from `{}`: {why}",
                e.result.render_name(),
                e.name
            )));
        }
        let result = serialize::export_result(&e.result).map_err(Reject::decline)?;
        // Each parameter's COMPONENT-boundary valtype (distinct from the core valtype — a signed 64
        // integer is `s64` at the boundary, `i64` in the core). A parameter is a scalar (a `list<u8>`
        // INPUT is not yet a surface type), so its faithful primitive byte is required.
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

    // A HOST-delegating program takes the host-import envelope shape (E2h-2): the delegated effect is a
    // component INTERFACE, its operations imported funcs the boundary resolves. This increment delegates a
    // SINGLE effect (every host import shares one effect name); a program delegating two distinct effects
    // declines (the multi-interface shape composes in a later increment).
    if !host_imports.is_empty() {
        let iface = host_imports[0].effect.clone();
        if host_imports.iter().any(|h| h.effect != iface) {
            return Err(Reject::decline(
                "delegating more than one host effect is not yet emitted (one interface per envelope; \
                 the multi-interface host shape is a later increment)",
            ));
        }
        let host_fns: Vec<envelope::HostFn> = host_imports
            .iter()
            .map(|h| envelope::HostFn {
                op: h.op.clone(),
                comp_functype: host_op_comp_functype(h),
                core_functype: Vec::new(), // unused by the envelope (the core module builds its own)
            })
            .collect();
        // A host op with a STRING parameter needs the shared-memory shape (the `(ptr,len)` a `string`
        // lowers to is read from a memory both the program and the op's canon-lower bind); a scalar-only
        // host set takes the memoryless shape (byte-identical to E2h-2).
        return Ok(if host::set_needs_memory(&host_imports) {
            envelope::assemble_host_mem(&core, &boundary, &iface, &host_fns)
        } else {
            envelope::assemble_host(&core, &boundary, &iface, &host_fns)
        });
    }

    // The versioned runtime import name (`cadenza:runtime/heap@0.0.0+<hash>`) — the name the runtime
    // component is imported under, carrying the content-address suffix `cdz-run` resolves it by. Unused
    // when `imports` is empty (the bare envelope). Built here (not in `envelope`) so the envelope stays
    // ABI-agnostic; the ABI identity lives in the generated `runtime_abi` table.
    let import_name = runtime_import_name();
    Ok(envelope::assemble(&core, &boundary, &imports, &import_name))
}

/// The component functype item (`0x40 <params> <result>`) for a host op — its parameter and result
/// COMPONENT valtype bytes (the faithful boundary primitives). Params are NAMED (`p0`, `p1`, …) as the
/// component model requires. A SCALAR param is its `AbiValType::comp_byte`; a STRING param is the
/// component `string` primitive (`COMP_STRING`). A `Unit` domain/result was already elided.
/// Collect the CONSTANT string values a `Core::HostCall` passes as a `string` argument, in encounter
/// order (E2h-string). Each becomes a data-segment entry the arg emit points `(ptr,len)` at. A non-string
/// arg is ignored; the walk descends every child so a host call nested anywhere is found. Mirrors
/// `host::collect_host_imports`' descent but gathers the arg strings rather than the op signatures.
fn collect_host_arg_strings(db: &mut Db, id: crate::ast::StructId, out: &mut Vec<String>) {
    use crate::core::Core;
    match crate::lower::core_of(db, id) {
        Core::HostCall { args, .. } => {
            for a in &args {
                if let Core::ConstStr(s) = crate::lower::core_of(db, *a) {
                    out.push(s);
                }
            }
            for a in args {
                collect_host_arg_strings(db, a, out);
            }
        }
        _ => {
            if let crate::ast::Struct::List(children) = db.ast.get(id).clone() {
                for c in children {
                    collect_host_arg_strings(db, c, out);
                }
            }
        }
    }
}

fn host_op_comp_functype(h: &host::HostImport) -> Vec<u8> {
    use host::HostParam;
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, p) in h.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&(pname.len() as u8).to_le_bytes());
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(match p {
            HostParam::Scalar(v) => v.comp_byte(),
            HostParam::Str => wasm_abi::COMP_STRING,
        });
    }
    item.extend_from_slice(&encode::wasm_vec(h.params.len(), &param_items));
    match h.result {
        Some(r) => item.extend_from_slice(&[0x00, r.comp_byte()]),
        None => item.extend_from_slice(&[0x01, 0x00]),
    }
    item
}

/// Append the `.debug_*` DWARF custom sections to an already-serialized core module `core`, when a
/// `spans` side-table is present (a debug build). Shared by the ordinary multi-export path and the
/// runtime resource-escape path: both lay the user function bodies FIRST in the code section, so
/// `code_ranges(funcs)` gives their correct payload-relative offsets and `code_section_payload_base`
/// walks the real bytes for the base — regardless of how many synthesized funcs (a resource walker's
/// `make`/`t-encode`) trail them. Inert + strippable (appended after the executed sections); a `None`
/// `spans` or a core with no code section leaves `core` untouched (byte-identical to a no-debug build).
fn append_debug_sections(
    db: &Db,
    layout: &Layout,
    funcs: &[SelectedFunc],
    imports: &[&runtime_abi::RtOp],
    spans: Option<&crate::spans::SpanData>,
    core: &mut Vec<u8>,
) {
    let Some(span_data) = spans else { return };

    // The wasm `name` custom section (D0): the module name (first export) + a function-name map. The
    // imported runtime ops are named at indices `0..imports.len()`; each program function is named at
    // its ABSOLUTE core index (`layout.abs` — which already accounts for the import shift, whether the
    // ordinary base `imports.len()` OR the resource path's `imports.len()+2`). Ascending by index, as
    // the name-map wire form requires (imports first, then defined funcs in emission order). Emitted for
    // BOTH paths from here (was previously only the ordinary path, inside `core_module`).
    let mut func_names: Vec<(u32, String)> = imports
        .iter()
        .enumerate()
        .map(|(i, o)| (i as u32, o.name.to_string()))
        .collect();
    for &def in &layout.order {
        let name = &db.defs[def].name;
        if let Some(abs) = layout.abs(def)
            && !name.is_empty()
        {
            func_names.push((abs, name.clone()));
        }
    }
    let module_name = layout
        .exports
        .first()
        .map(|e| e.name.as_str())
        .unwrap_or("main");
    core.extend_from_slice(&serialize::name_section(module_name, &func_names));

    // The `.debug_*` DWARF sections (D2/D3), when the core has a code section to reference.
    if let Some(code_base) = dwarf::code_section_payload_base(core) {
        let dwarf_funcs = dwarf_funcs_for(db, layout, funcs, imports, code_base, span_data);
        core.extend_from_slice(&dwarf::debug_sections(&span_data.module_path, &dwarf_funcs));
    }
}

/// Build the per-function DWARF descriptors for a module (shared by Mode E — embedded — and Mode S —
/// the sidecar `dwarf` file). `funcs`, `code_ranges(funcs, imports)`, and `layout.order` are all in the
/// SAME emission order and 1:1, so zip them: each function's def (→ source name), its code-offset range
/// (D1b), and the body span (→ line, D1a). `code_base` makes the range's payload-relative offsets
/// ABSOLUTE. A synthesized function (no `src_body`) is skipped — no misleading row. Because Mode E
/// appends debug sections AFTER the code section (inert), the code offsets are identical whether the
/// DWARF rides embedded or in a sidecar file, so both modes share this exact computation.
fn dwarf_funcs_for(
    db: &Db,
    layout: &Layout,
    funcs: &[SelectedFunc],
    imports: &[&runtime_abi::RtOp],
    code_base: u32,
    span_data: &crate::spans::SpanData,
) -> Vec<dwarf::DwarfFunc> {
    let ranges = serialize::code_ranges(funcs, imports);
    let mut out = Vec::new();
    for ((f, r), &def) in funcs.iter().zip(&ranges).zip(&layout.order) {
        let Some(src) = f.src_body else { continue };
        let line = span_data
            .range(src)
            .map(|(s, _)| span_data.line_at(s))
            .unwrap_or(1);
        // Scalar locals (D3) → `DW_TAG_variable` descriptors. A local whose type has no scalar base type
        // (a compound handle) is skipped — DWARF cannot describe the tagless heap (§3).
        let vars = f
            .locals
            .iter()
            .filter_map(|lv| {
                dwarf::base_type_of(&lv.ty).map(|base| dwarf::DwarfVar {
                    name: lv.name.clone(),
                    slot: lv.slot,
                    base,
                    is_param: lv.is_param,
                })
            })
            .collect();
        // Per-construct line rows: each `stmt_lines` marker `(Lir index, source occurrence)` → an
        // absolute code offset (`code_base + this fn's code_start + the instruction's byte offset in the
        // entry`) paired with the occurrence's source line. So a debugger steps line-by-line, not just
        // at the function entry. Empty `stmt_lines` (a single-construct body) leaves `rows` empty → the
        // line program falls back to one function-entry row (the function-granularity behavior).
        let instr_offs = serialize::instr_offsets(f, imports);
        let mut rows: Vec<(u32, u32, u32)> = f
            .stmt_lines
            .iter()
            .filter_map(|&(lir_ix, node)| {
                let byte_off = *instr_offs.get(lir_ix as usize)?;
                let (start, _) = span_data.range(node)?;
                Some((
                    code_base + r.code_start + byte_off,
                    span_data.line_at(start),
                    span_data.col_at(start),
                ))
            })
            .collect();
        // Ascending by offset (the line program emits rows in address order); dedup an offset two
        // constructs share (keep the first), then collapse consecutive rows at the SAME (line, column)
        // to keep only position TRANSITIONS — so the table has one row per distinct source position the
        // code visits (a column lets several constructs on one line stay distinct, the payoff over the
        // prior line-only collapse that folded a whole `(if c a b)` into a single row).
        rows.sort_by_key(|&(off, _, _)| off);
        rows.dedup_by_key(|&mut (off, _, _)| off);
        let mut prev_pos = (0u32, 0u32);
        rows.retain(|&(_, line, col)| {
            let keep = (line, col) != prev_pos;
            prev_pos = (line, col);
            keep
        });
        // Match-binder lexical scopes (D3): map each `[start_ix, end_ix)` Lir range to an ABSOLUTE
        // `[low_pc, high_pc)` code range. A start index reads its instruction's byte offset; an EXCLUSIVE
        // end at `code.len()` (past the last instruction) is the entry's full size (`code_end -
        // code_start`). A binder whose type has no scalar base type is skipped (DWARF can't describe the
        // heap, §3); a scope left with no describable var is dropped.
        let entry_len = r.code_end - r.code_start;
        let abs =
            |ix: u32| code_base + r.code_start + *instr_offs.get(ix as usize).unwrap_or(&entry_len);
        let scopes: Vec<dwarf::DwarfScope> = f
            .scopes
            .iter()
            .filter_map(|sc| {
                let vars: Vec<dwarf::DwarfVar> = sc
                    .vars
                    .iter()
                    .filter_map(|lv| {
                        dwarf::base_type_of(&lv.ty).map(|base| dwarf::DwarfVar {
                            name: lv.name.clone(),
                            slot: lv.slot,
                            base,
                            is_param: lv.is_param,
                        })
                    })
                    .collect();
                let (low_pc, high_pc) = (abs(sc.start_ix), abs(sc.end_ix));
                (!vars.is_empty() && high_pc > low_pc).then_some(dwarf::DwarfScope {
                    low_pc,
                    high_pc,
                    vars,
                })
            })
            .collect();
        out.push(dwarf::DwarfFunc {
            name: db.defs[def].name.clone(),
            low_pc: code_base + r.code_start,
            high_pc: code_base + r.code_end,
            line,
            vars,
            rows,
            scopes,
        });
    }
    out
}

/// Emit a standalone DWARF SIDECAR module (Mode S of `DESIGN-debug-info-rcdzc.md` §9.2) — a
/// `kind == "dwarf"` artifact separate from the runnable component. It is a minimal core wasm module
/// carrying ONLY the four `.debug_*` custom sections; the runnable component (emitted separately by a
/// sibling `Emit(WasmDebug)` or `Emit(Wasm)` request) stays lean, and a debugger loads this file
/// alongside it. Because Mode E appends its debug sections AFTER the code section (inert), a function's
/// code offset is the same whether DWARF is embedded or here — so this reuses the exact same
/// `core_module` + `code_ranges` + `code_section_payload_base` computation, then wraps the sections in
/// a bare module header instead of appending them to the runnable core.
///
/// Requires `spans` (guaranteed present by `compile`'s §9.4 check for a `needs_spans()` target). The
/// resource-escape shapes (a nullary compound export) are handled by `resource_escape_dwarf` FIRST — it
/// rebuilds the matching resource core (whose different layout Mode E attributes over identically) and
/// derives the sidecar from that, so a compound-returning export gets a sidecar whose offsets match the
/// embedded form. Only a runtime shape with no value-form walker yet (a runtime list) still declines.
/// DWARF descriptors for a nullary-compound RESOURCE-ESCAPE export, for the detached sidecar (Mode S).
///
/// A single nullary export returning a compound crosses through the resource-escape core, NOT the
/// ordinary multi-export core — so `emit_dwarf` cannot use its own ordinary-path core to place code
/// offsets. This mirrors [`emit`]'s resource dispatch exactly (constant / sum / runtime-Bytes / runtime
/// flat) to rebuild the SAME resource `main_core` the embedded Mode-E path attributes over, then derives
/// the per-function DWARF descriptors from it via the shared [`dwarf_funcs_for`]. Because the embedded
/// path appends its debug sections AFTER that core's code section (inert), a function's code offset is
/// identical embedded-vs-sidecar — so a debugger maps to the same instruction with either artifact.
///
/// Returns:
/// - `Ok(Some(funcs))` — the export IS a resource escape; `funcs` are its describable functions (EMPTY
///   for a fully-constant compound, whose resource core bakes bytes and has no user body — Mode E emits
///   no `.debug_*` there either, so an empty descriptor list yields a valid CU with no subprograms).
/// - `Ok(None)` — the export is NOT a resource escape; the caller uses the ordinary-path core.
/// - `Err(_)` — a genuine decline (a runtime compound shape with no value-form walker yet, matching the
///   embedded path's own decline for the same shape).
fn resource_escape_dwarf(
    db: &mut Db,
    layout: &Layout,
    span_data: &crate::spans::SpanData,
) -> Result<Option<Vec<dwarf::DwarfFunc>>, Reject> {
    // Only a single nullary export returning a compound takes the resource-escape core (mirrors `emit`).
    let [e] = &layout.exports[..] else {
        return Ok(None);
    };
    if !e.params.is_empty()
        || !matches!(
            e.result,
            crate::ty::Ty::Tuple(_)
                | crate::ty::Ty::Record(_)
                | crate::ty::Ty::Sum { .. }
                | crate::ty::Ty::List(_)
                | crate::ty::Ty::Map(_, _)
                | crate::ty::Ty::Set(_)
                | crate::ty::Ty::Bytes
                | crate::ty::Ty::String
                // A nominal-over-COMPOUND export takes the resource-escape core too (its erased value is
                // the underlying compound), so the sidecar-DWARF path declines it here alongside the other
                // compounds — parallel to Mode E's scope.
                | crate::ty::Ty::Nominal { .. }
        )
    {
        return Ok(None);
    }
    let export_def = e.def;
    let result = e.result.clone();
    let body = def_body(db, export_def)?;

    // A fully-CONSTANT compound bakes its bytes into a resource core with NO user function — nothing to
    // attribute. Mode E emits no `.debug_*` for it; the sidecar's CU therefore has no subprograms.
    if crate::lower::constant_value_form(db, body).is_some() {
        return Ok(Some(Vec::new()));
    }

    // A RUNTIME compound: reconstruct the resource core the embedded path builds for this variant, then
    // read its code offsets. Each arm computes the SAME (imports, funcs, main_core) triple its embedded
    // sibling does, so the offsets align. `resource_dwarf_from_core` finishes the shared tail.
    if let crate::ty::Ty::Sum { .. } = &result {
        let Some(tpl) = crate::lower::sum_form_template(db, &result) else {
            return Err(Reject::decline(
                "a DWARF sidecar for this sum-returning export is not yet supported (its variant \
                 payload has no value form — matches the embedded path's own decline)",
            ));
        };
        let (imports, funcs, layout) = resource_escape_build(db, layout, |used| {
            used.insert("sum-disc");
            let mut any_payload = false;
            let mut any_nested = false;
            for variant in &tpl.variants {
                for leaf in &variant.leaves {
                    any_payload |= leaf.via_sum_payload;
                    any_nested |= !leaf.path.is_empty();
                    match leaf.kind {
                        crate::lower::LeafFill::Int => used.insert("get-int"),
                        crate::lower::LeafFill::Bool => used.insert("get-bool"),
                    };
                }
            }
            if any_payload {
                used.insert("sum-payload");
            }
            if any_nested {
                used.insert("arr-get");
            }
            used.insert("drop");
        })?;
        let export_abs = layout.abs(export_def).ok_or_else(|| {
            Reject::decline("the escaping sum export is not in the emission order")
        })?;
        let main_core = serialize::runtime_resource_core_module_form(
            &funcs,
            &imports,
            export_abs,
            serialize::EscapeForm::Sum(&tpl),
        )
        .map_err(Reject::decline)?;
        return Ok(Some(resource_dwarf_from_core(
            db, &layout, &funcs, &imports, &main_core, span_data,
        )?));
    } else if matches!(result, crate::ty::Ty::Bytes) {
        let Some(form) = crate::lower::runtime_bytes_form(db) else {
            return Err(Reject::decline(
                "a DWARF sidecar for this runtime-Bytes export is not yet supported (no value form)",
            ));
        };
        let (imports, funcs, layout) = resource_escape_build(db, layout, |used| {
            used.insert("bytes-len");
            used.insert("bytes-get");
            used.insert("drop");
        })?;
        let export_abs = layout.abs(export_def).ok_or_else(|| {
            Reject::decline("the escaping bytes export is not in the emission order")
        })?;
        let main_core = serialize::runtime_resource_core_module_form(
            &funcs,
            &imports,
            export_abs,
            serialize::EscapeForm::RuntimeBytes(&form),
        )
        .map_err(Reject::decline)?;
        return Ok(Some(resource_dwarf_from_core(
            db, &layout, &funcs, &imports, &main_core, span_data,
        )?));
    } else if let Some(tpl) = crate::lower::runtime_value_form_template(&result) {
        let (imports, funcs, layout) = resource_escape_build(db, layout, |used| {
            if tpl.leaves.iter().any(|l| !l.path.is_empty()) {
                used.insert("arr-get");
            }
            for leaf in &tpl.leaves {
                match leaf.kind {
                    crate::lower::LeafFill::Int => used.insert("get-int"),
                    crate::lower::LeafFill::Bool => used.insert("get-bool"),
                };
            }
            used.insert("drop");
        })?;
        let export_abs = layout
            .abs(export_def)
            .ok_or_else(|| Reject::decline("the escaping export is not in the emission order"))?;
        let main_core = serialize::runtime_resource_core_module(&funcs, &imports, export_abs, &tpl)
            .map_err(Reject::decline)?;
        return Ok(Some(resource_dwarf_from_core(
            db, &layout, &funcs, &imports, &main_core, span_data,
        )?));
    }

    // A runtime compound with no value-form walker yet (e.g. a runtime list) — the embedded path
    // declines the same shape, so the sidecar does too rather than emit into a core it can't build.
    Err(Reject::decline(
        "a DWARF sidecar for this runtime compound-returning export is not yet supported (no value \
         form — matches the embedded path's own decline)",
    ))
}

/// Shared setup for a resource-escape sidecar: fix the import set (the reachable bodies' ops PLUS the
/// walker/dtor ops the `extra` closure adds), shift the layout's import base past `k + 2` (the ops +
/// `resource-new`/`resource-rep`, as every runtime-resource emitter does), and select every reachable
/// body. Returns the imports, the selected funcs, and the base-shifted layout — the inputs both the
/// resource core builder and `dwarf_funcs_for` need. Mirrors the head of `emit_runtime_resource`.
fn resource_escape_build(
    db: &mut Db,
    layout: &Layout,
    extra: impl FnOnce(&mut std::collections::BTreeSet<&'static str>),
) -> Result<(Vec<&'static runtime_abi::RtOp>, Vec<SelectedFunc>, Layout), Reject> {
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        select::collect_used_ops(db, body, &mut used);
    }
    extra(&mut used);
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;
    let layout = layout.with_import_base(imports.len() as u32 + 2);
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.export_plan(def) {
            Some(ep) => ep.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, &layout, Some(def))?);
    }
    Ok((imports, funcs, layout))
}

/// Derive the per-function DWARF descriptors from a resource-escape `main_core` — its code-section base
/// (the resource core is what the embedded Mode-E path attributes over, so the sidecar references the
/// same offsets). The shared tail of every resource-escape sidecar arm.
fn resource_dwarf_from_core(
    db: &Db,
    layout: &Layout,
    funcs: &[SelectedFunc],
    imports: &[&runtime_abi::RtOp],
    main_core: &[u8],
    span_data: &crate::spans::SpanData,
) -> Result<Vec<dwarf::DwarfFunc>, Reject> {
    let code_base = dwarf::code_section_payload_base(main_core)
        .ok_or_else(|| Reject::decline("the resource core has no code section to reference"))?;
    Ok(dwarf_funcs_for(
        db, layout, funcs, imports, code_base, span_data,
    ))
}

pub fn emit_dwarf(
    db: &mut Db,
    layout: &Layout,
    span_data: &crate::spans::SpanData,
) -> Result<Vec<u8>, Reject> {
    // A nullary-compound export crosses via the RESOURCE-ESCAPE core (a different layout than the
    // ordinary multi-export path). Its Mode-E component carries DWARF over the resource core's user
    // bodies; the sidecar must reference those SAME code offsets. `resource_escape_dwarf` recomputes the
    // matching resource core and derives the per-function descriptors from it — returning `Some(funcs)`
    // for a resource-escape export (possibly empty, for a fully-constant compound with no user bodies to
    // attribute — a valid CU with no subprograms, mirroring Mode E, which emits no `.debug_*` there) or
    // `None` when the export is NOT a resource escape (fall through to the ordinary path below).
    if let Some(dwarf_funcs) = resource_escape_dwarf(db, layout, span_data)? {
        let sections = dwarf::debug_sections(&span_data.module_path, &dwarf_funcs);
        return Ok(dwarf::standalone_dwarf_module(&sections));
    }

    // Recompute the exact core the runnable component embeds (imports + selection + serialize), so the
    // code offsets this DWARF references match that component byte-for-byte. Mirrors `emit`'s ordinary
    // multi-export path.
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_module_used_ops(db, layout, &mut used)?;
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;
    let layout = layout.with_import_base(imports.len() as u32);
    let layout = &layout;
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.export_plan(def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, layout, Some(def))?);
    }
    // The code-section base is where the RUNNABLE component's core module lays its code payload — the
    // undecorated core (no debug sections; the sidecar's addresses reference the runnable's code, which
    // carries no embedded debug). Serialize it just to measure that base.
    let core = serialize::core_module(&funcs, &imports, layout).map_err(Reject::decline)?;
    let code_base = dwarf::code_section_payload_base(&core)
        .ok_or_else(|| Reject::decline("the core module has no code section to reference"))?;

    let dwarf_funcs = dwarf_funcs_for(db, layout, &funcs, &imports, code_base, span_data);
    let sections = dwarf::debug_sections(&span_data.module_path, &dwarf_funcs);
    Ok(dwarf::standalone_dwarf_module(&sections))
}

/// Emit the COMBINED runtime-import + resource escape component (R2) for a single nullary export
/// returning a RUNTIME compound. The compound is built on the value heap by the export body, crosses as
/// a monomorphized resource, and its `encode()` WALKS the live handle to produce the canonical value
/// bytes (`tpl` — the value-form template for the result type). Unlike the constant escape (which bakes
/// the bytes), this emits the real program bodies + threads BOTH the runtime ops AND the resource
/// `new`/`rep` intrinsics ([[rcdzc-r1-resource-encode-linking-findings]] R2).
///
/// The used-op set fixes the import layout, so it is computed first and MUST include the ops the
/// synthesized `t-encode` walker calls (`arr-get` for any nested path, `get-int`/`get-bool` per leaf) —
/// those never appear in the reachable bodies (the export only CONSTRUCTS), so the template's holes add
/// them. `import_base` is `k + 2` (the `k` ops + `resource-new` + `resource-rep`), which shifts every
/// defined `Lir::Call` past the imports.
fn emit_runtime_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    tpl: &crate::lower::ValueFormTemplate,
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    // Ops the reachable bodies emit (construction: arr-alloc/arr-set/box-*), PLUS the ops the walker
    // `t-encode` calls (arr-get + get-int/get-bool per template leaf). The walker ops are added here
    // because they appear only in the synthesized encode body, not in any reachable Core.
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_module_used_ops(db, layout, &mut used)?;
    // The walker's ops: `arr-get` to descend a nested path, and per leaf its `get-*` accessor.
    if tpl.leaves.iter().any(|l| !l.path.is_empty()) {
        used.insert("arr-get");
    }
    for leaf in &tpl.leaves {
        match leaf.kind {
            crate::lower::LeafFill::Int => used.insert("get-int"),
            crate::lower::LeafFill::Bool => used.insert("get-bool"),
        };
    }
    // The resource DTOR calls `drop` to release the escaped compound's rc handle on host-drop (or when
    // `encode` consumes the `own<t>`). `drop` appears only in the synthesized dtor, never in a reachable
    // Core, so add it here — it becomes one of the lowered ops, and the envelope threads it into the
    // separate `heap-dtor` instance the dtor imports.
    used.insert("drop");
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;

    // Defined funcs' absolute indices are shifted past the `k` ops + the two resource intrinsics
    // (`resource-new`, `resource-rep`), so `import_base = k + 2`.
    let k = imports.len() as u32;
    let layout = layout.with_import_base(k + 2);
    let layout = &layout;

    // Select every reachable body (the export + its call-graph). The export body returns the compound's
    // heap handle (a `Ty::Tuple`/`Record` selects to an i32 handle — `valtype_of`), so it selects fine;
    // `make` will call it then `resource.new`.
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.export_plan(def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, layout, Some(def))?);
    }

    // The escaping export's absolute core-func index — `make` calls it to build the compound.
    let export_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the escaping export is not in the emission order"))?;

    let mut main_core = serialize::runtime_resource_core_module(&funcs, &imports, export_abs, tpl)
        .map_err(Reject::decline)?;
    // DEBUG: a compound-returning program is debuggable too. The user function bodies lead the escape
    // core's code section (the synthesized `make`/`t-encode`/`cabi_realloc` follow), so `code_ranges`
    // over `funcs` gives their correct payload-relative offsets and `code_section_payload_base` walks
    // the real bytes for the base — the same D2/D3 append as the ordinary path. `name` + `.debug_*`
    // ride in, inert + strippable. The synthesized walker funcs have no `src_body`, so they get no row.
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    // The RUNTIME escape uses the drop-calling dtor (releases the live rc handle), NOT the constant-path
    // stub — its handle is a genuine heap allocation the host must reclaim.
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_runtime_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
    ))
}

/// Emit the CLOSURE-RESOURCE component (C-HOST-1): an export whose RESULT is a closure `(-> A… R)` crosses
/// as a component resource with a `call` method the host invokes. The export body lowers NORMALLY (its
/// `Core::Closure` builds the cell + lifts the lambda into `layout.lifted`); the core's `make` calls the
/// export then `resource.new`s the cell, and `call` recovers it (`resource.rep`) + `call_indirect`s the
/// lifted body. Reuses the value-heap runtime (the cell is a heap allocation) via
/// `assemble_closure_resource`. First cut: the closure's args + result are the aliased scalar widths
/// (`abi_val_type`); a compound/closure arg or an un-representable result declines.
fn emit_closure_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    result: &crate::ty::Ty,
    _spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    use crate::backend::wasm::lir::valtype_of;
    // Flatten the curried closure type `(-> A (-> B R))` into its argument types `[A, B]` + the final
    // result `R`. Each must have a scalar boundary ABI type (this increment); else decline.
    let mut arg_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = result.clone();
    while let crate::ty::Ty::Fn(dom, rng) = cur {
        arg_tys.push((*dom).clone());
        cur = *rng;
    }
    let ret_ty = cur;
    // A closure that PERFORMS AN EFFECT cannot cross to the host (operator decision 2026-07-13). A
    // closure's handler context is the `(handle …)`/`(host …)` frame that was OPEN when the closure was
    // built; that frame is gone by the time the host later invokes `call()`, so an effect performed inside
    // the escaped body has no home. We reject this INTENTIONALLY rather than let it fall through to an
    // incidental decline (CDZ0401 "no home", or "not in the host-import set"). The detector: the export
    // body OR any lifted closure body reaches a `Core::HostCall` — a fully intra-program-HANDLED effect
    // leaves no `Core::HostCall` (the fold reduced it away), so a legitimate self-contained effect inside
    // the closure is NOT caught here; only an effect that would escape the boundary is.
    {
        let export_body = layout
            .exports
            .iter()
            .find(|e| e.def == export_def)
            .map(|e| e.body);
        let mut escaping = Vec::new();
        for l in &layout.lifted {
            host::collect_host_imports(db, l.body, &mut escaping);
        }
        if let Some(body) = export_body {
            host::collect_host_imports(db, body, &mut escaping);
        }
        if let Some(h) = escaping.first() {
            return Err(Reject::coded(
                crate::diag::Code::ClosureEscapesEffect,
                format!(
                    "a closure that performs an effect ({}.{}) cannot cross the host boundary — the \
                     closure's handler context does not travel with it, so the effect would have no home \
                     when the host invokes it (closures escaping effects are not supported)",
                    h.effect, h.op
                ),
            ));
        }
    }
    // Boundary bytes (component valtypes) for the `call` method's args + result — aliased scalar widths.
    let arg_bytes: Vec<u8> = arg_tys
        .iter()
        .map(|t| {
            host::abi_val_type(t)
                .map(|a| a.comp_byte())
                .ok_or_else(|| {
                    Reject::decline(format!(
                        "a closure argument of type {} has no scalar host-boundary representation \
                         (only aliased widths cross the closure `call` boundary yet)",
                        t.render_name()
                    ))
                })
        })
        .collect::<Result<_, _>>()?;
    let result_byte = host::abi_val_type(&ret_ty)
        .map(|a| a.comp_byte())
        .ok_or_else(|| {
            Reject::decline(format!(
                "a closure result of type {} has no scalar host-boundary representation",
                ret_ty.render_name()
            ))
        })?;
    // Core valtypes for the `call` method's args + result (used to build the core `call` signature +
    // the `call_indirect` lifted functype shape).
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = arg_tys
        .iter()
        .map(|t| valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype")))
        .collect::<Result<_, _>>()?;
    let ret_vt =
        valtype_of(&ret_ty).ok_or_else(|| Reject::decline("closure result has no machine valtype"))?;

    // The EXPORT's parameters (C-HOST-2): `make` forwards them so the host computes a distinct closure per
    // input (`(def (adder k) …)` → `make(k)`). Each must have a scalar boundary ABI type this increment.
    let export_params: Vec<(crate::ast::StructId, crate::ty::Ty)> = layout
        .exports
        .iter()
        .find(|e| e.def == export_def)
        .map(|e| e.params.clone())
        .unwrap_or_default();
    let make_param_vts: Vec<crate::backend::wasm::lir::ValType> = export_params
        .iter()
        .map(|(_, t)| {
            valtype_of(t).ok_or_else(|| Reject::decline("closure export param has no machine valtype"))
        })
        .collect::<Result<_, _>>()?;
    let make_param_bytes: Vec<u8> = export_params
        .iter()
        .map(|(_, t)| {
            host::abi_val_type(t).map(|a| a.comp_byte()).ok_or_else(|| {
                Reject::decline(format!(
                    "a closure-export parameter of type {} has no scalar host-boundary representation",
                    t.render_name()
                ))
            })
        })
        .collect::<Result<_, _>>()?;

    // Ops: the reachable bodies' ops (cell build: arr-alloc/arr-set/box-int) — plus the LIFTED closure
    // bodies' ops (a CAPTURING closure reads its env via `get-int`/`get-bool` etc., which appear ONLY in
    // the lifted body, not any top-level def), PLUS the `call` method's ops (arr-get + get-int to read the
    // code slot) + `drop` for the dtor. `resource_escape_build` walks only `layout.order`, so the lifted
    // bodies are walked explicitly here (mirrors `collect_module_used_ops`).
    let lifted_bodies: Vec<crate::ast::StructId> = layout
        .lifted
        .iter()
        .enumerate()
        .filter(|(code, _)| layout.lifted_reached.get(*code).copied().unwrap_or(true))
        .map(|(_, l)| l.body)
        .collect();
    let mut lifted_ops: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for &body in &lifted_bodies {
        select::collect_used_ops(db, body, &mut lifted_ops);
    }
    let (imports, mut funcs, layout) = resource_escape_build(db, layout, |used| {
        used.insert("arr-get");
        used.insert("get-int");
        used.insert("drop");
        used.extend(lifted_ops.iter().copied());
    })?;
    let export_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the closure export is not in the emission order"))?;
    // The lifted closure the export built occupies table slot 0; its functype index in the core.
    if layout.lifted.is_empty() {
        return Err(Reject::decline(
            "a closure export produced no lifted lambda (the closure did not survive as a runtime value)",
        ));
    }
    // APPEND the lambda-lifted closure bodies AFTER the `order` defs (the funcref table's element section
    // points at `import_base + order.len() + slot`, so the lifted bodies must be the trailing funcs).
    // Each is `(env, param…) -> result`: prepend a fresh env param (an i32 handle, keyed by a synthesized
    // atom nothing resolves to). Mirrors the ordinary `emit` path's lifted-body selection.
    for (code, lifted) in layout.lifted.clone().into_iter().enumerate() {
        let env_key = db.push_name("$closure-env");
        let mut params = vec![(env_key, crate::ty::Ty::Bytes)];
        params.extend(lifted.params.iter().cloned());
        if layout.lifted_reached.get(code).copied().unwrap_or(true) {
            funcs.push(select_function_of(db, lifted.body, &params, &layout, None)?);
        } else {
            funcs.push(select::stub_function(&params, &lifted.ret_ty));
        }
    }
    let lifted_type_idx = layout.lifted_type_index(0, layout.import_base);
    let main_core = serialize::closure_resource_core_module(
        &funcs,
        &imports,
        export_abs,
        &arg_vts,
        ret_vt,
        &make_param_vts,
        lifted_type_idx,
        &layout,
    )
    .map_err(Reject::decline)?;
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_closure_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &make_param_bytes,
        &arg_bytes,
        result_byte,
    ))
}

/// Emit the runtime-import + resource escape component for a single nullary export returning a RUNTIME
/// `Bytes` (a `concat`/recursion-built sequence, not a compile-time constant). Mirrors
/// [`emit_runtime_resource`], but the escape form is [`serialize::EscapeForm::RuntimeBytes`] — its
/// `encode()` is the LOOPING walker (`encode_bytes_walk_body`) that writes a variable-length value form.
/// The walker's ops (`bytes-len`, `bytes-get`) appear only in the synthesized encode body, plus `drop`
/// for the `own<t>` release — added here since they are not in any reachable Core.
fn emit_runtime_bytes_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    form: &crate::lower::RuntimeBytesForm,
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        select::collect_used_ops(db, body, &mut used);
    }
    // The looping walker's ops: read the length and each byte, and release the handle.
    used.insert("bytes-len");
    used.insert("bytes-get");
    used.insert("drop");
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;

    let k = imports.len() as u32;
    let layout = layout.with_import_base(k + 2);
    let layout = &layout;

    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.export_plan(def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, layout, Some(def))?);
    }
    let export_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the escaping bytes export is not in the emission order"))?;

    let mut main_core = serialize::runtime_resource_core_module_form(
        &funcs,
        &imports,
        export_abs,
        serialize::EscapeForm::RuntimeBytes(form),
    )
    .map_err(Reject::decline)?;
    // DEBUG: same as the flat/sum resource paths — the user bodies lead the escape core's code section,
    // so the `name` + `.debug_*` sections attribute correctly; the synthesized bytes walker has no
    // `src_body` and gets no row.
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_runtime_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
    ))
}

/// Emit the runtime-import + resource escape component for a single nullary export returning a SUM. The
/// sum builds on the value heap (`sum-new`), crosses as a monomorphized resource, and its `encode()`
/// switches on `sum-disc` to render the matching variant (`tpl` — one value-form template per variant).
/// Mirrors [`emit_runtime_resource`] but the walker's ops include `sum-disc` (always) + `sum-payload`
/// (whenever any variant carries a payload leaf) alongside the per-leaf `get-*`/`arr-get`.
fn emit_runtime_sum_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    tpl: &crate::lower::SumFormTemplate,
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    // Ops the reachable bodies emit (construction: sum-new/arr-alloc/box-*), PLUS the ops the sum walker
    // calls: `sum-disc` (always), `sum-payload` (to reach a variant's payload), `arr-get` (a
    // multi-payload tuple index), and per leaf its `get-*`; and `drop` (the dtor + encode release).
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        select::collect_used_ops(db, body, &mut used);
    }
    used.insert("sum-disc");
    let mut any_payload_leaf = false;
    let mut any_nested_path = false;
    for variant in &tpl.variants {
        for leaf in &variant.leaves {
            if leaf.via_sum_payload {
                any_payload_leaf = true;
            }
            if !leaf.path.is_empty() {
                any_nested_path = true;
            }
            match leaf.kind {
                crate::lower::LeafFill::Int => used.insert("get-int"),
                crate::lower::LeafFill::Bool => used.insert("get-bool"),
            };
        }
    }
    if any_payload_leaf {
        used.insert("sum-payload");
    }
    if any_nested_path {
        used.insert("arr-get");
    }
    used.insert("drop");
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;

    // Same index-space shift as the flat runtime resource: `k` ops + `resource-new` + `resource-rep`.
    let k = imports.len() as u32;
    let layout = layout.with_import_base(k + 2);
    let layout = &layout;

    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.export_plan(def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, layout, Some(def))?);
    }
    let export_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the escaping sum export is not in the emission order"))?;

    let mut main_core = serialize::runtime_resource_core_module_form(
        &funcs,
        &imports,
        export_abs,
        serialize::EscapeForm::Sum(tpl),
    )
    .map_err(Reject::decline)?;
    // DEBUG: same as the flat resource path — the user bodies lead the code section, so the D2/D3
    // sections attribute correctly; the synthesized sum walker funcs have no `src_body` and get no row.
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_runtime_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
    ))
}

/// Emit the runtime-import + resource escape component for a single nullary export returning a RUNTIME
/// RECURSIVE sum (a linked list, a tree — a self-referential payload, so no fixed per-variant template).
/// Its `encode()` (`encode_recursive_sum_walk_body`) bakes the compiler-built shape `descriptor` as a
/// heap `Bytes`, calls the runtime `value-encode(rep, desc)` to render the value-form document, and
/// copies it out — the runtime owns the recursion + document assembly
/// (`DESIGN-recursive-sum-escape-walker.md`). The walker's ops (`value-encode`, `bytes-alloc`/`-set`
/// to build the descriptor, `bytes-len`/`-get` to copy the doc out, `drop` for the releases) appear only
/// in the synthesized encode body, so they are added here.
fn emit_recursive_sum_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    descriptor: &[u8],
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        select::collect_used_ops(db, body, &mut used);
    }
    // The recursive-sum walker's ops: render via `value-encode`, build the descriptor Bytes
    // (`bytes-alloc`/`bytes-set`), copy the document out (`bytes-len`/`bytes-get`), release the handles.
    for op in [
        "value-encode",
        "bytes-alloc",
        "bytes-set",
        "bytes-len",
        "bytes-get",
        "drop",
    ] {
        used.insert(op);
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

    let k = imports.len() as u32;
    let layout = layout.with_import_base(k + 2);
    let layout = &layout;

    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.export_plan(def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, layout, Some(def))?);
    }
    let export_abs = layout.abs(export_def).ok_or_else(|| {
        Reject::decline("the escaping recursive-sum export is not in the emission order")
    })?;

    let mut main_core = serialize::runtime_resource_core_module_form(
        &funcs,
        &imports,
        export_abs,
        serialize::EscapeForm::RecursiveSum(descriptor),
    )
    .map_err(Reject::decline)?;
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_runtime_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
    ))
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

/// The runtime ops every emitted function will call, into `used`. Walks BOTH the top-level defs
/// (`layout.order`) AND the lambda-lifted closure bodies (`layout.lifted`) — a lifted body is emitted as
/// its own wasm function, so an op used ONLY inside a closure (e.g. `get-bool` unboxing a captured
/// boolean, which no top-level def happens to use) must be collected too, or its `CallImport` resolves to
/// a bogus index and the module is invalid. A REACHED lifted body selects its real body; an unreached one
/// emits an inert stub that calls no runtime op, so only reached bodies are walked.
fn collect_module_used_ops(
    db: &mut Db,
    layout: &Layout,
    used: &mut std::collections::BTreeSet<&'static str>,
) -> Result<(), Reject> {
    for &def in &layout.order {
        let body = def_body(db, def)?;
        select::collect_used_ops(db, body, used);
    }
    for (code, lifted) in layout.lifted.clone().into_iter().enumerate() {
        if layout.lifted_reached.get(code).copied().unwrap_or(true) {
            select::collect_used_ops(db, lifted.body, used);
        }
    }
    Ok(())
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
        assert_eq!(
            wasm_abi::COMP_SEC_COMPONENT,
            ComponentSectionId::Component as u8
        );
        assert_eq!(
            wasm_abi::COMP_SEC_INSTANCE,
            ComponentSectionId::Instance as u8
        );
    }
}
