//! Sum-typed runtime-resource emitters, peeled out of `mod.rs` to keep it under
//! `xtask_support::MAX_SOURCE_BYTES` (512 KiB); pure code move, behavior-neutral.
//! `use super::*` brings in the parent module’s helpers unchanged.
use super::*;

/// Emit the runtime-import + resource escape component for a single nullary export returning a SUM. The
/// sum builds on the value heap (`sum-new`), crosses as a monomorphized resource, and its `encode()`
/// switches on `sum-disc` to render the matching variant (`tpl` — one value-form template per variant).
/// Mirrors [`emit_runtime_resource`] but the walker's ops include `sum-disc` (always) + `sum-payload`
/// (whenever any variant carries a payload leaf) alongside the per-leaf `get-*`/`arr-get`.
pub(super) fn emit_runtime_sum_resource(
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
    // Scan the top-level defs AND the lambda-lifted closure bodies (see `append_lifted_bodies`) so an op
    // used only inside a closure is imported too — else its `CallImport` resolves to `u32::MAX` (invalid).
    collect_module_used_ops(db, layout, &mut used)?;
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

    // PEER-IN-RESOURCE-ESCAPE (task #6, increment 2 — the non-recursive SUM path, e.g. an Option result).
    // Same fusion as the flat/recursive-sum paths.
    let mut host_imports: Vec<host::HostImport> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        host::collect_host_imports(db, body, &mut host_imports);
    }
    let mut extern_imports: Vec<host::ExternImport> = Vec::new();
    if !db.effect_bindings.is_empty() {
        let bindings = db.effect_bindings.clone();
        host_imports.retain(|h| {
            if let Some(iface) = bindings.get(&h.effect) {
                extern_imports.push(host::ExternImport {
                    interface: iface.clone(),
                    op: h.op.clone(),
                    params: h.params.iter().filter_map(host_param_abi).collect(),
                    result: h.result,
                });
                false
            } else {
                true
            }
        });
    }
    // HOST-DELEGATED effect in a SUM resource escape — the host mirror (increment 2), same as the Flat site.
    // Scalar/unit host ops compose via `assemble_host_runtime_resource`; a String-param host op or a
    // host-alongside-peer shape declines.
    if !host_imports.is_empty() {
        if !extern_imports.is_empty() {
            return Err(Reject::declined(
                crate::diag::DeclineId::WasmHostPeerResourceFusion,
                "the host+peer+resource fusion — a host effect and a peer effect both composed with a \
                 resource-escaping entrypoint — needs the combined host-and-peer import-space emit \
                 alongside the resource escape",
            ));
        }
        if host::set_needs_memory(&host_imports) {
            return Err(Reject::unsupported(
                "a host op with a STRING parameter in a resource-escaping entrypoint is not supported \
                 (a scalar/unit host op result-escaping as a resource IS supported)",
            ));
        }
        let h = host_imports.len() as u32;
        let k = imports.len() as u32;
        let host_order: Vec<(String, String)> = host_imports
            .iter()
            .map(|hi| (hi.effect.clone(), hi.op.clone()))
            .collect();
        let iface = host_imports[0].effect.clone();
        // SINGLE effect only — `assemble_host_runtime_resource` imports ONE host interface, so >1 distinct
        // effect would be conflated + mis-serialized (PR #481). Decline the multi-effect shape cleanly.
        if host_imports.iter().any(|hi| hi.effect != iface) {
            return Err(Reject::declined(
                crate::diag::DeclineId::WasmMultiHostEffectDelegation,
                "delegating more than one host effect from a resource-escaping entrypoint is not \
                 supported (one interface per envelope)",
            ));
        }
        let host_layout = layout
            .with_import_base(h + k + 2)
            .with_host_order(host_order);
        let host_layout = &host_layout;
        let mut funcs: Vec<SelectedFunc> = Vec::new();
        for &def in &host_layout.order {
            let body = def_body(db, def)?;
            let params = match host_layout.export_plan(def) {
                Some(e) => e.params.clone(),
                None => crate::layout::def_params(db, def),
            };
            funcs.push(select_function_of(
                db,
                body,
                &params,
                host_layout,
                Some(def),
            )?);
        }
        append_lifted_bodies(db, &mut funcs, host_layout)?;
        let export_abs = host_layout.abs(export_def).ok_or_else(|| {
            Reject::decline("the escaping sum export is not in the emission order")
        })?;
        let (make_param_vts, make_param_bytes) =
            export_make_params(db, host_layout, export_def)?.scalars_only()?;
        let make_core_slots: Vec<serialize::MakeCoreSlot> = make_param_vts
            .iter()
            .map(|_| serialize::MakeCoreSlot::Scalar)
            .collect();
        let host_as_extern = host_as_extern_for(&host_imports);
        let mut main_core = serialize::runtime_resource_core_module_form_ex2(
            &funcs,
            &imports,
            &host_as_extern,
            true, // leading ops are HOST — import from "host"
            export_abs,
            serialize::EscapeForm::Sum(tpl),
            &[],
            &make_param_vts,
            &make_core_slots,
            &escape_lifted_table(host_layout),
            0, // build-once static compounds not threaded on this path (byte-identical; a follow-up increment)
            &[], // no static-compound init
        )
        .map_err(Reject::decline)?;
        append_debug_sections(db, host_layout, &funcs, &imports, spans, &mut main_core);
        let dtor_core = serialize::resource_dtor_module_with_drop();
        let import_name = runtime_import_name();
        let make_slots: Vec<envelope::ArgSlot> = make_param_bytes
            .iter()
            .map(|&b| envelope::ArgSlot::Scalar(b))
            .collect();
        let host_fns: Vec<envelope::HostFn> = host_imports
            .iter()
            .map(|hi| envelope::HostFn {
                op: hi.op.clone(),
                comp_functype: host_op_comp_functype(hi, 0, 0, &[], None),
                has_list_param: hi
                    .params
                    .iter()
                    .any(|p| matches!(p, host::HostParam::Bytes)),
                core_functype: Vec::new(),
            })
            .collect();
        return Ok(envelope::assemble_host_runtime_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &iface,
            &host_fns,
            &make_slots,
        ));
    }
    // The fused envelope supports MULTIPLE distinct peer interfaces (grouped into g imported instances).
    let p = extern_imports.len() as u32;
    let extern_order: Vec<(String, String)> = extern_imports
        .iter()
        .map(|e| (e.interface.clone(), e.op.clone()))
        .collect();

    // Same index-space shift as the flat runtime resource, plus the `p` peer ops: `import_base = p+k+2`.
    let k = imports.len() as u32;
    let layout = layout
        .with_import_base(p + k + 2)
        .with_extern_order(extern_order);
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
    append_lifted_bodies(db, &mut funcs, layout)?;
    let export_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the escaping sum export is not in the emission order"))?;
    let (make_param_vts, make_param_bytes) =
        export_make_params(db, layout, export_def)?.scalars_only()?;
    // Scalar-param-only shape: one `MakeCoreSlot::Scalar` per param, so `make` forwards each leaf directly.
    let make_core_slots: Vec<serialize::MakeCoreSlot> = make_param_vts
        .iter()
        .map(|_| serialize::MakeCoreSlot::Scalar)
        .collect();

    let mut main_core = serialize::runtime_resource_core_module_form_ex2(
        &funcs,
        &imports,
        &extern_imports,
        false, // leading ops are PEER (extern), not host — import from "peer"
        export_abs,
        serialize::EscapeForm::Sum(tpl),
        &[],
        &make_param_vts,
        &make_core_slots,
        &escape_lifted_table(layout),
        0, // build-once static compounds not threaded on this path (byte-identical; a follow-up increment)
        &[], // no static-compound init
    )
    .map_err(Reject::decline)?;
    // DEBUG: same as the flat resource path — the user bodies lead the code section, so the D2/D3
    // sections attribute correctly; the synthesized sum walker funcs have no `src_body` and get no row.
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    // The sum-result escape is scalar-param-only (`scalars_only` above), so every slot is a scalar byte.
    let make_slots: Vec<envelope::ArgSlot> = make_param_bytes
        .iter()
        .map(|&b| envelope::ArgSlot::Scalar(b))
        .collect();
    if extern_imports.is_empty() {
        return Ok(envelope::assemble_runtime_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_slots,
        ));
    }
    let op_ifaces: Vec<&str> = extern_imports
        .iter()
        .map(|e| e.interface.as_str())
        .collect();
    let peer_fns: Vec<envelope::HostFn> = extern_imports
        .iter()
        .map(|e| envelope::HostFn {
            op: e.op.clone(),
            comp_functype: extern_op_comp_functype(e),
            core_functype: Vec::new(),
            has_list_param: false,
        })
        .collect();
    Ok(envelope::assemble_extern_runtime_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &peer_fns,
        &op_ifaces,
        &make_slots,
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
pub(super) fn emit_recursive_sum_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    descriptor: &[u8],
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    // The `make`-forwarded params: a compound parameter is rebuilt in-guest from its flattened leaves, so
    // its `arr-alloc`/`arr-set`/box-* ops must join the import set BEFORE it is frozen below.
    let make_params = export_make_params(db, layout, export_def)?;

    // BUILD-ONCE STATIC COMPOUNDS (WIT static encoding follow-up): the escaping bodies of a List/Map/Set /
    // recursive-sum return may embed markable constant Tuple/Record/List/Map/Set literals (imc2's `(tuple 1 2)`
    // inside a returned list; irb1's constant lists). Collect them build-once exactly as `emit_runtime_resource`
    // does — the serializer plumbing (GLOBAL/START) is shared. byte_base 0: no static-bytes globals here.
    let static_compounds = collect_static_compounds(db, &layout.order);
    let static_compound_init = if static_compounds.is_empty() {
        Vec::new()
    } else {
        select::build_static_compound_init(db, &static_compounds, 0, layout)?
    };

    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    // Scan the top-level defs AND the lambda-lifted closure bodies (see `append_lifted_bodies`) so an op
    // used only inside a closure is imported too — else its `CallImport` resolves to `u32::MAX` (invalid).
    collect_module_used_ops(db, layout, &mut used)?;
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
    // The static-compound START init builds each immortal with arr-alloc/arr-set/box-*/mark-immortal[-deep]/
    // vec-of-arr/map-*/set-*; force the full init op set when the table is non-empty (a hoisted-only constant
    // leaves those ops in no body). Idempotent; no-op when there are no static compounds.
    if !static_compounds.is_empty() {
        for op in [
            "arr-alloc",
            "arr-set",
            "box-int",
            "box-bool",
            "bytes-alloc",
            "bytes-set",
            "mark-immortal",
            "vec-of-arr",
            "mark-immortal-deep",
            "map-empty",
            "map-insert",
            "set-empty",
            "set-insert",
            "sum-new", // hoisted nullary mixed-sum terminal builds via sum-new(disc, IMM_UNIT) in the init
            "value-canonicalize", // hoisted map/set with a LIST key canonicalizes it for CHAMP-slot exactness
            "bytes-compact", // hoisted map/set with a rope String/Bytes key compacts it (ikc1/itf2 fix)
        ] {
            used.insert(op);
        }
    }
    // A compound `make` param rebuilds each cell with `arr-alloc`/`arr-set` + a box op per scalar leaf.
    make_params.collect_rebuild_ops(&mut |op| {
        used.insert(op);
    });
    let imports: Vec<&runtime_abi::RtOp> = used
        .iter()
        .map(|name| {
            runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| Reject::decline(format!("runtime op `{name}` not in the ABI table")))
        })
        .collect::<Result<_, _>>()?;

    // PEER-IN-RESOURCE-ESCAPE (task #6, increment 2 — the recursive-sum / List/Map/Set path). Same fusion
    // as `emit_runtime_resource`: split peer-bound imports, thread `extern_order` + `import_base = p+k+2`,
    // and dispatch to the fused assembler when a peer op is reached in a body whose result escapes here.
    let mut host_imports: Vec<host::HostImport> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        host::collect_host_imports(db, body, &mut host_imports);
    }
    let mut extern_imports: Vec<host::ExternImport> = Vec::new();
    if !db.effect_bindings.is_empty() {
        let bindings = db.effect_bindings.clone();
        host_imports.retain(|h| {
            if let Some(iface) = bindings.get(&h.effect) {
                extern_imports.push(host::ExternImport {
                    interface: iface.clone(),
                    op: h.op.clone(),
                    params: h.params.iter().filter_map(host_param_abi).collect(),
                    result: h.result,
                });
                false
            } else {
                true
            }
        });
    }
    // HOST-DELEGATED effect in a RECURSIVE-SUM resource escape — the host mirror (increment 2). Scalar/unit
    // host ops compose via `assemble_host_runtime_resource`; a String-param or host-alongside-peer declines.
    if !host_imports.is_empty() {
        if !extern_imports.is_empty() {
            return Err(Reject::declined(
                crate::diag::DeclineId::WasmHostPeerResourceFusion,
                "the host+peer+resource fusion — a host effect and a peer effect both composed with a \
                 resource-escaping entrypoint — needs the combined host-and-peer import-space emit \
                 alongside the resource escape",
            ));
        }
        if host::set_needs_memory(&host_imports) {
            return Err(Reject::unsupported(
                "a host op with a STRING parameter in a resource-escaping entrypoint is not supported \
                 (a scalar/unit host op result-escaping as a resource IS supported)",
            ));
        }
        let h = host_imports.len() as u32;
        let k = imports.len() as u32;
        let host_order: Vec<(String, String)> = host_imports
            .iter()
            .map(|hi| (hi.effect.clone(), hi.op.clone()))
            .collect();
        let iface = host_imports[0].effect.clone();
        // SINGLE effect only — `assemble_host_runtime_resource` imports ONE host interface, so >1 distinct
        // effect would be conflated + mis-serialized (PR #481). Decline the multi-effect shape cleanly.
        if host_imports.iter().any(|hi| hi.effect != iface) {
            return Err(Reject::declined(
                crate::diag::DeclineId::WasmMultiHostEffectDelegation,
                "delegating more than one host effect from a resource-escaping entrypoint is not \
                 supported (one interface per envelope)",
            ));
        }
        let host_layout = layout
            .with_import_base(h + k + 2)
            .with_host_order(host_order);
        let host_layout = &host_layout;
        let mut funcs: Vec<SelectedFunc> = Vec::new();
        for &def in &host_layout.order {
            let body = def_body(db, def)?;
            let params = match host_layout.export_plan(def) {
                Some(e) => e.params.clone(),
                None => crate::layout::def_params(db, def),
            };
            funcs.push(select_function_of(
                db,
                body,
                &params,
                host_layout,
                Some(def),
            )?);
        }
        append_lifted_bodies(db, &mut funcs, host_layout)?;
        let export_abs = host_layout.abs(export_def).ok_or_else(|| {
            Reject::decline("the escaping recursive-sum export is not in the emission order")
        })?;
        let mut main_core = serialize::runtime_resource_core_module_form_ex2(
            &funcs,
            &imports,
            &host_as_extern_for(&host_imports),
            true, // leading ops are HOST — import from "host"
            export_abs,
            serialize::EscapeForm::RecursiveSum(descriptor),
            &[],
            &make_params.leaf_vts,
            &make_params.core_slots(),
            &escape_lifted_table(host_layout),
            0, // build-once static compounds not threaded on this path (byte-identical; a follow-up increment)
            &[], // no static-compound init
        )
        .map_err(Reject::decline)?;
        append_debug_sections(db, host_layout, &funcs, &imports, spans, &mut main_core);
        let dtor_core = serialize::resource_dtor_module_with_drop();
        let import_name = runtime_import_name();
        let host_fns: Vec<envelope::HostFn> = host_imports
            .iter()
            .map(|hi| envelope::HostFn {
                op: hi.op.clone(),
                comp_functype: host_op_comp_functype(hi, 0, 0, &[], None),
                has_list_param: hi
                    .params
                    .iter()
                    .any(|p| matches!(p, host::HostParam::Bytes)),
                core_functype: Vec::new(),
            })
            .collect();
        return Ok(envelope::assemble_host_runtime_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &iface,
            &host_fns,
            &make_params.boundary_slots(),
        ));
    }
    // The fused envelope supports MULTIPLE distinct peer interfaces (grouped into g imported instances).
    let p = extern_imports.len() as u32;
    let extern_order: Vec<(String, String)> = extern_imports
        .iter()
        .map(|e| (e.interface.clone(), e.op.clone()))
        .collect();

    let k = imports.len() as u32;
    let layout = layout
        .with_import_base(p + k + 2)
        .with_extern_order(extern_order)
        // Thread build-once static compounds so the body's Core::Tuple/List/… arms emit `global.get`.
        .with_static_compounds(static_compounds.clone(), static_compound_init.clone());
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
    append_lifted_bodies(db, &mut funcs, layout)?;
    let export_abs = layout.abs(export_def).ok_or_else(|| {
        Reject::decline("the escaping recursive-sum export is not in the emission order")
    })?;

    let mut main_core = serialize::runtime_resource_core_module_form_ex2(
        &funcs,
        &imports,
        &extern_imports,
        false, // leading ops are PEER (extern), not host — import from "peer"
        export_abs,
        serialize::EscapeForm::RecursiveSum(descriptor),
        &[],
        &make_params.leaf_vts,
        &make_params.core_slots(),
        &escape_lifted_table(layout),
        static_compounds.len(),
        &static_compound_init,
    )
    .map_err(Reject::decline)?;
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    if extern_imports.is_empty() {
        return Ok(envelope::assemble_runtime_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_params.boundary_slots(),
        ));
    }
    let op_ifaces: Vec<&str> = extern_imports
        .iter()
        .map(|e| e.interface.as_str())
        .collect();
    let peer_fns: Vec<envelope::HostFn> = extern_imports
        .iter()
        .map(|e| envelope::HostFn {
            op: e.op.clone(),
            comp_functype: extern_op_comp_functype(e),
            core_functype: Vec::new(),
            has_list_param: false,
        })
        .collect();
    Ok(envelope::assemble_extern_runtime_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &peer_fns,
        &op_ifaces,
        &make_params.boundary_slots(),
    ))
}
