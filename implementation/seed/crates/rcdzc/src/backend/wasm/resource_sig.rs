//! Distinct-signature / roundtrip / bytes closure-resource emitters (peeled from `mod.rs`, a pure code
//! move — the 512 KiB file-size mandate split; behavior byte-identical). Each emits one resource-escape
//! component variant: `emit_distinct_sig_resource` (several closure exports of DIFFERENT signatures, one
//! resource type per distinct signature), `emit_roundtrip_resource` (a closure handed back in + applied),
//! `emit_distinct_sig_roundtrip_resource` (the roundtrip form partitioned by signature), and
//! `emit_runtime_bytes_resource` (the `list<u8>`-provider resource). Called from `mod.rs`'s `emit`
//! routing and `closure_resource.rs` (hence `pub(super)`).

use super::*;

/// Emit the DISTINCT-SIGNATURE multi-export component: several closure exports of DIFFERENT signatures
/// cross as G resource types (one per distinct signature), each with its own `make-<name>`(s) + `call-<g>`.
/// Generalizes `emit_multi_closure_resource` (which requires ONE shared signature). Exports are GROUPED by
/// their solved closure signature; each group becomes a `SigGroup` (serializer) + `SigGroupAbi` (envelope).
/// A group's representative `call_indirect` functype is the FIRST lifted lambda whose valtype shape matches
/// that signature (all closures of one signature share the shape, so slot choice within a group is
/// immaterial). The `distinct_signature_…` oracle + the distinct-sig serializer seam proved the pieces.
pub(super) fn emit_distinct_sig_resource(
    db: &mut Db,
    layout: &Layout,
    _spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    use crate::backend::wasm::lir::{ValType, valtype_of};
    // GROUP the CLOSURE exports (result `Ty::Fn`) by their signature (first-seen order). Each group is a
    // distinct resource type. PLAIN (non-closure) exports are collected separately and published as ordinary
    // top-level component funcs alongside the resource interface (the distinct-sig case of the mixed shape).
    // `sigs` holds the group signatures in order; `export_group[i]` is closure export i's group.
    let mut sigs: Vec<crate::ty::Ty> = Vec::new();
    let mut closure_exports: Vec<&crate::layout::ExportPlan> = Vec::new();
    let mut export_group: Vec<usize> = Vec::new();
    let mut plain_exports: Vec<&crate::layout::ExportPlan> = Vec::new();
    for e in &layout.exports {
        if !matches!(e.result, crate::ty::Ty::Fn(_, _)) {
            plain_exports.push(e);
            continue;
        }
        let gi = sigs.iter().position(|s| s == &e.result).unwrap_or_else(|| {
            sigs.push(e.result.clone());
            sigs.len() - 1
        });
        closure_exports.push(e);
        export_group.push(gi);
    }
    // Per group: flatten its signature → arg/ret types + validate scalar boundary bytes.
    struct GroupInfo {
        arg_vts: Vec<ValType>,
        ret_vt: ValType,
        arg_bytes: Vec<u8>,
        result_byte: u8,
        ret_is_bytes: bool,
        ret_template: Option<crate::lower::ValueFormTemplate>,
        ret_descriptor: Option<Vec<u8>>,
        /// The direct-call compound ARG for this group (a single fixed-shape scalar tuple/record, SOLE or among
        /// scalar args): the tuple's per-field component bytes + prefix scalar bytes + suffix scalar bytes + the
        /// `TupleArgRebuild` (with `base_param`). Set for BOTH a flat AND a nested tuple arg (the rebuild is
        /// recursive for nested); for a nested arg the `field_bytes` are unused (the `nested_shape` drives the
        /// envelope mint). `None` = scalar args.
        tuple_arg: Option<GroupCompoundArg>,
        /// `Some(shape)` when this group's sole arg is a NESTED fixed-shape compound (a tuple/record with a
        /// tuple/record field): the recursive `TupleFieldShape` the per-group `call-<g>` envelope mints the
        /// inner `tuple<…>` types from (by index). `None` for a flat all-scalar-field tuple (which uses the
        /// flat `tuple_arg` field bytes). When `Some`, `tuple_arg` also carries the recursive rebuild.
        nested_shape: Option<Vec<crate::backend::wasm::envelope::TupleFieldShape>>,
        /// The LIFTED lambda's OWN param valtypes for this group — used to match a representative lifted slot.
        /// For a scalar-arg group this equals `arg_vts`; for a TUPLE-arg group `arg_vts` is the FLATTENED
        /// fields but the lifted lambda takes ONE i32 tuple-cell handle, so this is `[I32]` (the cell), NOT the
        /// flattened fields. The `call-<g>` wrapper flattens/rebuilds between the boundary and the lambda.
        match_vts: Vec<ValType>,
        /// N-COMPOUND-ARGS for this group: `Some((slots, rebuilds))` when its closure takes ≥2 fixed-shape
        /// tuple/record args. `slots` drives the per-group `call-<g>` functype mint (the `ArgSlot` model);
        /// `rebuilds` is one `TupleArgRebuild` per tuple, threaded into the core's `SigGroup.tuples`. `None`
        /// unless ≥2 tuple args (the ≤1-tuple cases stay `tuple_arg`/`nested_shape`).
        #[allow(clippy::type_complexity)]
        multi_args: Option<(
            Vec<crate::backend::wasm::envelope::ArgSlot>,
            Vec<crate::backend::wasm::serialize::TupleArgRebuild>,
        )>,
        /// A SOLE `(Option/Result scalar)` arg for this group: `Some((slot, rebuild))` drives the per-group
        /// `call-<g>` mint (`option<…>`/`result<…>` via the `ArgSlot`) + the guest sum-cell rebuild. `None`
        /// unless the sole arg is such a sum. Scalar-result groups only (a list result over a sum declines).
        sum_arg: Option<(
            crate::backend::wasm::envelope::ArgSlot,
            crate::backend::wasm::serialize::SumArgRebuild,
        )>,
    }
    let mut ginfos: Vec<GroupInfo> = Vec::new();
    for sig in &sigs {
        let mut arg_tys = Vec::new();
        let mut cur = sig.clone();
        while let crate::ty::Ty::Fn(dom, rng) = cur {
            arg_tys.push((*dom).clone());
            cur = *rng;
        }
        let ret_ty = cur;
        // DIRECT-CALL COMPOUND ARG (distinct-sig): a single fixed-shape scalar tuple/record arg — the SOLE arg
        // OR among aliased-width scalars — crosses as a native component `tuple<…>` the shared `call-<g>`
        // rebuilds (interleaving prefix/suffix scalars via `emit_closure_call_args`). Detected per group so the
        // scalar `arg_bytes` decline doesn't reject it. 5-tuple = (field bytes, full flattened vts, prefix,
        // suffix, rebuild).
        let group_tuple_arg: Option<CompoundArgBoundary> = if arg_tys.len() == 1 {
            fixed_shape_scalar_tuple_arg(&arg_tys[0])
                .map(|(fb, fv, rb)| (fb, fv, Vec::new(), Vec::new(), rb))
        } else {
            single_compound_among_scalars(arg_tys.as_slice())
        };
        // A NESTED fixed-shape compound arg (a tuple/record with a tuple/record field) — SOLE or among scalars:
        // detected when the flat `group_tuple_arg` is None. The per-group `call-<g>` rebuilds the nested cell
        // recursively (interleaving prefix/suffix scalars); the per-group envelope mints the inner `tuple<…>`
        // types by index from `shape` + interleaves the prefix/suffix boundary bytes. `NestedCompoundArgBoundary`
        // = (leaf_bytes [unused], full flattened vts, rebuild, shape, prefix bytes, suffix bytes).
        let group_nested: Option<NestedCompoundArgBoundary> = if group_tuple_arg.is_some() {
            None
        } else {
            nested_sole_or_among_scalars(arg_tys.as_slice())
        };
        // ≥2 fixed-shape tuple/record args for this group (the N-compound-args path): the per-group `call-<g>`
        // rebuilds each cell (a slice of `TupleArgRebuild`) + mints N `tuple<…>` types via the `ArgSlot` model.
        // Detected only when neither single-tuple classifier fired.
        #[allow(clippy::type_complexity)]
        let group_multi_args: Option<(
            Vec<crate::backend::wasm::envelope::ArgSlot>,
            Vec<crate::backend::wasm::lir::ValType>,
            Vec<crate::backend::wasm::serialize::TupleArgRebuild>,
        )> = if group_tuple_arg.is_none() && group_nested.is_none() {
            multi_compound_args(arg_tys.as_slice())
        } else {
            None
        };
        // `arg_bytes` is empty for a compound/sum arg (its boundary is the minted tuple/option/result type via
        // the slot list). A sum arg is classified AFTER the result shape (below), but it is detected the same
        // way: a single-arg group whose sole arg is neither a scalar nor a tuple. So compute it lazily below,
        // after `group_sum_arg` — here just handle the tuple cases + the plain-scalar fallback.
        let arg_is_sum = group_tuple_arg.is_none()
            && group_nested.is_none()
            && group_multi_args.is_none()
            && arg_tys.len() == 1
            && (fixed_shape_option_scalar_arg(db, &arg_tys[0]).is_some()
                || fixed_shape_result_compound_arg(db, &arg_tys[0]).is_some());
        let arg_bytes: Vec<u8> = if group_tuple_arg.is_some()
            || group_nested.is_some()
            || group_multi_args.is_some()
            || arg_is_sum
        {
            Vec::new() // the flattened fields are carried by tuple_arg/nested_shape/multi_args/sum_arg
        } else {
            arg_tys
                .iter()
                .map(|t| {
                    closure_boundary_byte(t)
                        .ok_or_else(|| closure_boundary_reject("argument", t, &db.name_ctx()))
                })
                .collect::<Result<_, _>>()?
        };
        // A byte-rope (`Bytes`/`String`) result crosses `call-<g>` as `list<u8>` (not an inline scalar), so
        // it skips the scalar-boundary-byte check; `result_byte` is a placeholder (unused for byte-rope).
        let ret_is_bytes = matches!(
            ret_ty.strip_nominal(),
            crate::ty::Ty::Bytes | crate::ty::Ty::String
        );
        // A fixed-shape COMPOUND result crosses `call-<g>` as `list<u8>` carrying the value form (its own
        // per-group template, since each group's result type may differ). `None` for byte-rope/scalar.
        let ret_template = if ret_is_bytes || closure_boundary_byte(&ret_ty).is_some() {
            None
        } else {
            crate::lower::runtime_value_form_template(ret_ty.strip_nominal(), &db.name_ctx())
        };
        // A VARIABLE-LENGTH collection (List/Map/Set) result crosses `call-<g>` as `list<u8>` too, rendered
        // via `value-encode(rep, desc)` against this group's own shape descriptor.
        let ret_descriptor =
            if ret_is_bytes || ret_template.is_some() || closure_boundary_byte(&ret_ty).is_some() {
                None
            } else {
                // Any other value-encodable result (collection, sum, or compound-containing-collection) →
                // the runtime `value-encode` descriptor path; `sum_shape_descriptor` returns `None` for a
                // scalar or unrenderable shape. (A fixed-shape compound took the static `ret_template` path.)
                crate::lower::sum_shape_descriptor(db, ret_ty.strip_nominal())
            };
        let ret_is_list = ret_is_bytes || ret_template.is_some() || ret_descriptor.is_some();
        let result_byte = if ret_is_list {
            0
        } else {
            closure_boundary_byte(&ret_ty)
                .ok_or_else(|| closure_boundary_reject("result", &ret_ty, &db.name_ctx()))?
        };
        // A SOLE `(Option/Result scalar)` arg for this group (SCALAR result only — the per-group list cores
        // thread tuples, not sums). Classified only when no tuple classifier fired + the result is scalar.
        let group_sum_arg: Option<(
            crate::backend::wasm::envelope::ArgSlot,
            Vec<crate::backend::wasm::lir::ValType>,
            crate::backend::wasm::serialize::SumArgRebuild,
        )> = if group_tuple_arg.is_none()
            && group_nested.is_none()
            && group_multi_args.is_none()
            && !ret_is_list
            && arg_tys.len() == 1
        {
            fixed_shape_option_scalar_arg(db, &arg_tys[0])
                .or_else(|| fixed_shape_result_compound_arg(db, &arg_tys[0]))
        } else {
            None
        };
        // Core call-arg valtypes: the FULL flattened core param list when a tuple arg (prefix scalars, tuple
        // fields, suffix scalars), a sum arg's `(disc, payload)`, else each arg's own valtype.
        let arg_vts: Vec<ValType> = if let Some((_, all_vts, _, _, _)) = &group_tuple_arg {
            all_vts.clone()
        } else if let Some((_, all_vts, _, _, _, _)) = &group_nested {
            all_vts.clone() // prefix scalars, then the nested tuple's depth-first leaves, then suffix scalars
        } else if let Some((_, all_vts, _)) = &group_multi_args {
            all_vts.clone() // the flattened leaves of EVERY tuple/scalar arg, in order (N-compound-args)
        } else if let Some((_, payload_vts, _)) = &group_sum_arg {
            // a sum flattens to (disc: i32, <payload leaves…>) — scalar = 1 leaf, compound = its leaves.
            let mut vts = vec![ValType::I32];
            vts.extend(payload_vts.iter().copied());
            vts
        } else {
            arg_tys
                .iter()
                .map(|t| {
                    valtype_of(t)
                        .ok_or_else(|| Reject::decline("closure arg has no machine valtype"))
                })
                .collect::<Result<_, _>>()?
        };
        let ret_vt = valtype_of(&ret_ty)
            .ok_or_else(|| Reject::decline("closure result has no machine valtype"))?;
        // A tuple arg now composes with EVERY result shape per group — scalar, byte-rope, fixed-compound, and
        // collection: the per-group `call-<g>` bodies (all four branches) + the per-group envelope functypes
        // thread the `TupleArgRebuild`. No result-shape decline remains for a distinct-sig tuple arg.
        // The lifted lambda's own param shape: it takes each ARG's OWN valtype — a tuple arg is ONE i32
        // tuple-cell handle (the `call-<g>` wrapper rebuilds it from the flattened fields), scalars are
        // themselves. So `match_vts` is per-arg (NOT the flattened boundary fields in `arg_vts`).
        let match_vts: Vec<ValType> = if group_sum_arg.is_some() {
            // A sum arg is ONE i32 sum-cell handle the `call-<g>` wrapper rebuilds; the lifted lambda takes it.
            vec![ValType::I32]
        } else if group_tuple_arg.is_some() || group_nested.is_some() || group_multi_args.is_some()
        {
            // Each ARG's OWN lambda-param valtype: a fixed-shape tuple/record (flat OR nested) is ONE i32 cell
            // handle the `call-<g>` wrapper rebuilds; a scalar is its own valtype.
            arg_tys
                .iter()
                .map(|t| {
                    if tuple_field_abi(t).is_some() || nested_fixed_shape_tuple_arg(t).is_some() {
                        Some(ValType::I32)
                    } else {
                        valtype_of(t)
                    }
                    .ok_or_else(|| Reject::decline("closure arg has no machine valtype"))
                })
                .collect::<Result<_, _>>()?
        } else {
            arg_vts.clone()
        };
        // A nested group carries its recursive rebuild + prefix/suffix in `tuple_arg` (field_bytes unused) + its
        // shape in `nested_shape`; a flat group carries the field bytes + rebuild in `tuple_arg`, `nested_shape`
        // None.
        let nested_shape = group_nested
            .as_ref()
            .map(|(_, _, _, shape, _, _)| shape.clone());
        let tuple_arg = group_tuple_arg
            .map(|(fb, _, pre, suf, rb)| (fb, pre, suf, rb))
            .or_else(|| group_nested.map(|(_, _, rb, _, pre, suf)| (Vec::new(), pre, suf, rb)));
        // ≥2 tuple args: carry the slot list (for the per-group envelope mint) + the rebuilds (for the core).
        let multi_args = group_multi_args.map(|(slots, _, rebuilds)| (slots, rebuilds));
        // A sum arg: carry the slot (for the per-group envelope mint) + the rebuild (for the core).
        let sum_arg = group_sum_arg.map(|(slot, _, rebuild)| (slot, rebuild));
        ginfos.push(GroupInfo {
            arg_vts,
            ret_vt,
            arg_bytes,
            result_byte,
            ret_is_bytes,
            ret_template,
            ret_descriptor,
            tuple_arg,
            nested_shape,
            match_vts,
            multi_args,
            sum_arg,
        });
    }
    // Effect-escape fence: no lifted body may perform a host effect.
    {
        let mut escaping = Vec::new();
        for l in &layout.lifted {
            host::collect_host_imports(db, l.body, &mut escaping);
        }
        if let Some(h) = escaping.first() {
            return Err(Reject::coded(
                crate::diag::Code::ClosureEscapesEffect,
                format!(
                    "a closure that performs an effect ({}.{}) cannot cross the host boundary — the \
                     closure's handler context does not travel with it (closures escaping effects are \
                     not supported)",
                    h.effect, h.op
                ),
            ));
        }
    }
    // Per-export make spec (name + params), collected BEFORE the build moves the layout.
    struct MakeSpec {
        def: usize,
        group: usize,
        name: String,
        param_vts: Vec<ValType>,
        param_bytes: Vec<u8>,
    }
    let mut make_specs: Vec<MakeSpec> = Vec::new();
    for (ei, e) in closure_exports.iter().enumerate() {
        let param_vts: Vec<_> = e
            .params
            .iter()
            .map(|(_, t)| {
                valtype_of(t).ok_or_else(|| Reject::decline("closure export param has no valtype"))
            })
            .collect::<Result<_, _>>()?;
        let param_bytes: Vec<u8> = e
            .params
            .iter()
            .map(|(_, t)| {
                closure_boundary_byte(t)
                    .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
            })
            .collect::<Result<_, _>>()?;
        make_specs.push(MakeSpec {
            def: e.def,
            group: export_group[ei],
            name: format!("make-{}", e.name),
            param_vts,
            param_bytes,
        });
    }
    // Per PLAIN export: source name (core + kebab boundary name), param bytes, scalar result byte.
    struct PlainSpec {
        def: usize,
        name: String,
        param_bytes: Vec<u8>,
        result_byte: u8,
    }
    let mut plain_specs: Vec<PlainSpec> = Vec::new();
    for e in &plain_exports {
        let param_bytes: Vec<u8> = e
            .params
            .iter()
            .map(|(_, t)| {
                closure_boundary_byte(t)
                    .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
            })
            .collect::<Result<_, _>>()?;
        let result_byte = closure_boundary_byte(&e.result).ok_or_else(|| {
            Reject::declined(
                crate::diag::DeclineId::WasmCompoundResultWithClosureExport,
                format!(
                    "a plain export `{}` returning {} has no scalar host-boundary representation \
                 (a compound result alongside a closure export needs the compound-boundary emit)",
                    e.name,
                    e.result.render_name(&db.name_ctx())
                ),
            )
        })?;
        plain_specs.push(PlainSpec {
            def: e.def,
            name: e.name.clone(),
            param_bytes,
            result_byte,
        });
    }

    // Collect lifted-body ops, build, append lifted bodies (same as the multi-export path).
    let lifted_bodies: Vec<crate::ast::StructId> = layout
        .lifted
        .iter()
        .enumerate()
        .filter(|(code, _)| layout.lifted_reached.get(*code).copied().unwrap_or(true))
        .map(|(_, l)| l.body)
        .collect();
    let mut lifted_ops: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();
    for &body in &lifted_bodies {
        select::collect_used_ops(db, body, &mut lifted_ops);
    }
    // Snapshot each lifted lambda's valtype shape BEFORE the build moves the layout (to match a group's
    // signature to a representative slot).
    let lifted_shapes: Vec<(Vec<ValType>, Option<ValType>)> = layout
        .lifted
        .iter()
        .map(|l| {
            let ps: Vec<ValType> = l.params.iter().filter_map(|(_, t)| valtype_of(t)).collect();
            (ps, valtype_of(&l.ret_ty))
        })
        .collect();
    // G resource types → the envelope prepends 2*G resource intrinsics before the defined funcs, so fix
    // `import_base` accordingly (else `abs`/`lifted_abs`/the element segment are off by 2*(G-1)).
    let intrinsics = (2 * sigs.len()) as u32;
    let any_bytes = ginfos.iter().any(|gi| gi.ret_is_bytes);
    let any_compound = ginfos.iter().any(|gi| gi.ret_template.is_some());
    let any_collection = ginfos.iter().any(|gi| gi.ret_descriptor.is_some());
    // A tuple-arg group's `call-<g>` rebuilds the flattened tuple cell (`arr-alloc` + per field box + `arr-set`
    // + `drop`). Collect the box ops the rebuilds actually reference (per field type) so they are imported.
    let tuple_box_ops: std::collections::BTreeSet<&'static str> = {
        let mut ops = std::collections::BTreeSet::new();
        for gi in &ginfos {
            if let Some((_, _, _, rb)) = gi.tuple_arg.as_ref() {
                for f in &rb.fields {
                    f.collect_box_ops(&mut |bop| {
                        ops.insert(bop);
                    });
                }
            }
            // ≥2 tuple args: each tuple's rebuild box ops (a Bool/Float field in any of them).
            if let Some((_, rebuilds)) = gi.multi_args.as_ref() {
                for rb in rebuilds {
                    for f in &rb.fields {
                        f.collect_box_ops(&mut |bop| {
                            ops.insert(bop);
                        });
                    }
                }
            }
        }
        ops
    };
    let any_tuple_arg = ginfos
        .iter()
        .any(|gi| gi.tuple_arg.is_some() || gi.multi_args.is_some());
    // A sum-arg group's `call-<g>` rebuilds the sum cell via `sum-new`, boxing each arm's payload. Collect
    // those box ops (a Bool/Float payload) so they are imported.
    let any_sum_arg = ginfos.iter().any(|gi| gi.sum_arg.is_some());
    let sum_box_ops: std::collections::BTreeSet<&'static str> = {
        let mut ops = std::collections::BTreeSet::new();
        for gi in &ginfos {
            if let Some((_, rb)) = gi.sum_arg.as_ref() {
                for arm in [&rb.arm_true, &rb.arm_false] {
                    arm.collect_ops(&mut |op| {
                        ops.insert(op);
                    });
                }
            }
        }
        ops
    };
    let (imports, mut funcs, layout) = resource_escape_build_n(db, layout, intrinsics, |used| {
        used.insert("arr-get");
        used.insert("get-int");
        used.insert("drop");
        if any_bytes {
            // A byte-rope group's `call-<g>` copies the closure's Bytes/String out via a `bytes-len`/
            // `bytes-get` loop into linear memory (the `list<u8>` payload).
            used.insert("bytes-len");
            used.insert("bytes-get");
        }
        if any_compound {
            // A compound group's `call-<g>` walks the returned handle to fill the value form — a Bool leaf
            // reads `get-bool` (int + nested `arr-get` already covered).
            used.insert("get-bool");
        }
        if any_collection {
            // A collection group's `call-<g>` renders via `value-encode(rep, desc)` (build the descriptor
            // Bytes + copy the doc out).
            for op in [
                "value-encode",
                "bytes-alloc",
                "bytes-set",
                "bytes-len",
                "bytes-get",
            ] {
                used.insert(op);
            }
        }
        if any_tuple_arg {
            // The tuple-arg cell rebuild: `arr-alloc N` + per field box + `arr-set` (+ `drop`, already above).
            used.insert("arr-alloc");
            used.insert("arr-set");
            for op in &tuple_box_ops {
                used.insert(op);
            }
        }
        if any_sum_arg {
            // A sum-arg group's `call-<g>` rebuilds the sum cell via `sum-new`, boxing each arm's payload.
            used.insert("sum-new");
            for op in &sum_box_ops {
                used.insert(op);
            }
        }
        used.extend(lifted_ops.iter().copied());
    })?;
    if layout.lifted.is_empty() {
        return Err(Reject::decline(
            "a distinct-signature closure program produced no lifted lambda",
        ));
    }
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
    // For each group, find a representative lifted SLOT whose shape matches (arg_vts + ret_vt). The lifted
    // lambda's env param (slot 0, an i32) is prepended at emission, so match the lambda's OWN params.
    let group_slot = |gi: usize| -> Option<usize> {
        let ginfo = &ginfos[gi];
        // Match on the lifted lambda's OWN param shape (`match_vts`) — for a tuple-arg group this is the ONE
        // i32 tuple-cell handle the lambda takes, NOT the flattened boundary fields in `arg_vts`.
        lifted_shapes.iter().position(|(ps, rv)| {
            ps.as_slice() == ginfo.match_vts.as_slice() && *rv == Some(ginfo.ret_vt)
        })
    };

    // Build the serializer SigGroups + envelope SigGroupAbis, in group order.
    let mut ser_groups: Vec<serialize::SigGroup> = Vec::new();
    let mut abi_groups: Vec<envelope::SigGroupAbi> = Vec::new();
    #[allow(clippy::needless_range_loop)]
    // `gi` is a semantic GROUP id — indexes sigs/ginfos AND filters make_specs
    for gi in 0..sigs.len() {
        let slot = group_slot(gi).ok_or_else(|| {
            Reject::decline("a closure signature group has no matching lifted lambda")
        })?;
        let mut ser_makes = Vec::new();
        let mut abi_makes = Vec::new();
        for m in make_specs.iter().filter(|m| m.group == gi) {
            let export_abs = layout
                .abs(m.def)
                .ok_or_else(|| Reject::decline("a closure export is not in the emission order"))?;
            ser_makes.push(serialize::ClosureMake {
                export_name: m.name.clone(),
                export_abs,
                param_vts: m.param_vts.clone(),
            });
            abi_makes.push(envelope::ClosureMakeAbi {
                name: m.name.clone(),
                make_param_bytes: m.param_bytes.clone(),
            });
        }
        // The core's per-group tuple rebuilds: ≥2 args carries one rebuild per tuple; a single flat/nested arg
        // carries exactly one; a scalar-arg group carries none.
        let group_tuples: Vec<serialize::TupleArgRebuild> =
            if let Some((_, rebuilds)) = &ginfos[gi].multi_args {
                rebuilds.clone()
            } else if let Some((_, _, _, rb)) = &ginfos[gi].tuple_arg {
                vec![rb.clone()]
            } else {
                Vec::new()
            };
        // A sum-arg group carries one `SumArgRebuild`; others none.
        let group_sums: Vec<serialize::SumArgRebuild> = ginfos[gi]
            .sum_arg
            .as_ref()
            .map(|(_, rb)| vec![rb.clone()])
            .unwrap_or_default();
        ser_groups.push(serialize::SigGroup {
            makes: ser_makes,
            arg_vts: ginfos[gi].arg_vts.clone(),
            ret_vt: ginfos[gi].ret_vt,
            lifted_slot: slot,
            ret_is_bytes: ginfos[gi].ret_is_bytes,
            ret_template: ginfos[gi].ret_template.clone(),
            ret_descriptor: ginfos[gi].ret_descriptor.clone(),
            tuples: group_tuples,
            sums: group_sums,
        });
        abi_groups.push(envelope::SigGroupAbi {
            makes: abi_makes,
            arg_bytes: ginfos[gi].arg_bytes.clone(),
            result_byte: ginfos[gi].result_byte,
            // The envelope's `ret_is_bytes` means "crosses as list<u8>" — a byte-rope, a fixed-shape compound,
            // OR a variable-length collection.
            ret_is_bytes: ginfos[gi].ret_is_bytes
                || ginfos[gi].ret_template.is_some()
                || ginfos[gi].ret_descriptor.is_some(),
            tuple_arg_bytes: ginfos[gi]
                .tuple_arg
                .as_ref()
                .map(|(fb, _, _, _)| fb.clone()),
            tuple_prefix_bytes: ginfos[gi]
                .tuple_arg
                .as_ref()
                .map(|(_, pre, _, _)| pre.clone())
                .unwrap_or_default(),
            tuple_suffix_bytes: ginfos[gi]
                .tuple_arg
                .as_ref()
                .map(|(_, _, suf, _)| suf.clone())
                .unwrap_or_default(),
            tuple_shape: ginfos[gi].nested_shape.clone(),
            // ≥2 tuple args OR a sum arg → the slot list drives the per-group `call-<g>` mint (an `option<…>`/
            // `result<…>`/N-tuple type); ≤1-tuple groups leave it None (they use `tuple_arg_bytes`/`tuple_shape`).
            call_arg_slots: ginfos[gi]
                .multi_args
                .as_ref()
                .map(|(slots, _)| slots.clone())
                .or_else(|| {
                    ginfos[gi]
                        .sum_arg
                        .as_ref()
                        .map(|(slot, _)| vec![slot.clone()])
                }),
        });
    }

    // Plain-export specs: resolve each body's core-func index post-build.
    let ser_plain: Vec<serialize::PlainExport> = plain_specs
        .iter()
        .map(|p| {
            let body_abs = layout
                .abs(p.def)
                .ok_or_else(|| Reject::decline("a plain export is not in the emission order"))?;
            Ok(serialize::PlainExport {
                export_name: p.name.clone(),
                body_abs,
            })
        })
        .collect::<Result<_, Reject>>()?;
    let abi_plain: Vec<envelope::PlainExportAbi> = plain_specs
        .iter()
        .map(|p| envelope::PlainExportAbi {
            name: p.name.clone(),
            core_name: p.name.clone(),
            param_bytes: p.param_bytes.clone(),
            result_byte: p.result_byte,
        })
        .collect();
    // C-HOST-6: each group's per-signature `call-g<n>` takes `borrow<t_g>` (repeatable — the host keeps each
    // handle across calls; the `t-dtor` reclaims). Same borrow posture as the shared single/multi `call`s.
    let main_core = serialize::distinct_sig_resource_core_module(
        &funcs,
        &imports,
        &ser_groups,
        &ser_plain,
        &layout,
        true,
    )
    .map_err(Reject::decline)?;
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_distinct_sig_resource_mixed_borrow(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_groups,
        &abi_plain,
        true,
    ))
}

/// Emit the ROUND-TRIP closure-resource component (C-HOST-4, Direction 2): a program with PRODUCER exports
/// (result is a closure `(-> A… R)`) AND CONSUMER exports (a PARAMETER is a closure of that same signature)
/// — the host produces a closure handle from a producer, then threads it BACK into a consumer, which
/// applies it. Producers emit `make-<name>` (as in the multi-export path); consumers are selected NORMALLY
/// (their closure param is a plain CELL handle applied via `Core::CallClosure`), and the serializer's
/// consumer wrapper `resource.rep`s the boundary handle → cell before calling the body. All share the ONE
/// resource type + funcref table — the closure the host holds was lifted in THIS module, so the consumer's
/// `call_indirect` resolves against the same in-program lifted lambda by signature (the round-trip oracle's
/// key realization). First cut: scalar-aliased closure args/result; a consumer takes EXACTLY ONE closure
/// param (leading), optionally followed by scalar args.
pub(super) fn emit_roundtrip_resource(
    db: &mut Db,
    layout: &Layout,
    _spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    use crate::backend::wasm::lir::valtype_of;
    // Partition exports into producers (result Ty::Fn) and consumers (a Ty::Fn param). The shared closure
    // signature is a producer's result (all producers + all consumer closure-params must match it).
    let producers: Vec<&crate::layout::ExportPlan> = layout
        .exports
        .iter()
        .filter(|e| matches!(e.result, crate::ty::Ty::Fn(_, _)))
        .collect();
    let consumers: Vec<&crate::layout::ExportPlan> = layout
        .exports
        .iter()
        .filter(|e| {
            e.params
                .iter()
                .any(|(_, t)| matches!(t, crate::ty::Ty::Fn(_, _)))
        })
        .collect();
    // PLAIN exports: neither a producer (closure RESULT) nor a consumer (a closure PARAM). They ride
    // alongside the round trip as ordinary top-level funcs — WITHOUT this they were silently dropped.
    let plain_exports: Vec<&crate::layout::ExportPlan> = layout
        .exports
        .iter()
        .filter(|e| {
            !matches!(e.result, crate::ty::Ty::Fn(_, _))
                && !e
                    .params
                    .iter()
                    .any(|(_, t)| matches!(t, crate::ty::Ty::Fn(_, _)))
        })
        .collect();
    // An export that is BOTH a producer (closure RESULT) and a consumer (closure PARAM) — a closure
    // TRANSFORMER `(-> (-> A B) … (-> C D))`, e.g. `(def (twice (: g …)) (fn (x) (g (g x))))` — is out of
    // scope: the host would hand a closure IN and get one OUT of the same call, which needs the param to
    // cross as `own<t>` AND the result as `own<t>` in one boundary func (the producer path forwards its
    // params to `make`, which cannot take a closure param). Decline cleanly NAMING the shape, rather than
    // letting it fall through to the confusing internal "a producer parameter has no scalar representation"
    // (the `make`-forwarding site chokes on the closure param).
    if let Some(t) = layout.exports.iter().find(|e| {
        matches!(e.result, crate::ty::Ty::Fn(_, _))
            && e.params
                .iter()
                .any(|(_, p)| matches!(p, crate::ty::Ty::Fn(_, _)))
    }) {
        return Err(Reject::declined(
            crate::diag::DeclineId::WasmClosureTransformer,
            format!(
                "the export `{}` both RECEIVES a closure (a parameter) and RETURNS one (its result) — a \
             closure transformer. That is not supported: the host would pass a closure in and get one \
             out of the same call, which needs the closure to cross as `own<t>` in both directions of one \
             boundary function (DESIGN-closure-host-resource-rcdzc.md, closure transformers)",
                t.name
            ),
        ));
    }
    let sig = producers
        .first()
        .map(|p| p.result.clone())
        .ok_or_else(|| {
            Reject::decline(
                "a round-trip closure program needs at least one PRODUCER export (whose result is a \
                 closure) so the consumer has a closure to receive; a consumer-only program (the host \
                 fabricating a closure) is out of scope",
            )
        })?;
    // Every producer result AND every consumer closure-param must be the SAME signature (one resource
    // type + one lifted functype this increment).
    for p in &producers {
        if p.result != sig {
            return Err(Reject::unsupported(
                "a round-trip program mixing closures of DIFFERENT signatures is not supported \
                 (one resource type per signature)",
            ));
        }
    }
    for c in &consumers {
        let closure_params: Vec<&crate::ty::Ty> = c
            .params
            .iter()
            .filter_map(|(_, t)| matches!(t, crate::ty::Ty::Fn(_, _)).then_some(t))
            .collect();
        // A consumer may take SEVERAL closure params (each threaded back), but every one must be the
        // SAME signature as the produced closure — one resource type `t` this increment (distinct
        // signatures need N resource types, a later slice). A closure param may sit in any position; the
        // consumer functype follows source order.
        if closure_params.is_empty() {
            return Err(Reject::decline(
                "a round-trip consumer takes no closure parameter (nothing to thread back)",
            ));
        }
        if closure_params.iter().any(|t| *t != &sig) {
            return Err(Reject::unsupported(
                "a round-trip consumer's closure parameter has a different signature than the produced \
                 closure, which is not supported (mixed signatures)",
            ));
        }
    }
    // Flatten the shared signature → arg types + result (the closure `call`/consumer boundary shape).
    let mut arg_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = sig.clone();
    while let crate::ty::Ty::Fn(dom, rng) = cur {
        arg_tys.push((*dom).clone());
        cur = *rng;
    }
    let ret_ty = cur;
    // Effect-escape fence (same as the other closure paths): no lifted body may perform a host effect.
    {
        let mut escaping = Vec::new();
        for l in &layout.lifted {
            host::collect_host_imports(db, l.body, &mut escaping);
        }
        if let Some(h) = escaping.first() {
            return Err(Reject::coded(
                crate::diag::Code::ClosureEscapesEffect,
                format!(
                    "a closure that performs an effect ({}.{}) cannot cross the host boundary — the \
                     closure's handler context does not travel with it (closures escaping effects are \
                     not supported)",
                    h.effect, h.op
                ),
            ));
        }
    }
    // VALIDATE the shared closure signature is MACHINE-representable. On the ROUND-TRIP path the closure is
    // applied ENTIRELY IN-GUEST by a consumer (`(g …)` inside the consumer body) — its argument is BUILT in
    // the guest and never crosses the host boundary; only the closure HANDLE (an `own<t>` resource, i32) and
    // the consumer's OWN scalar params cross. So a closure ARGUMENT need only have a machine valtype (a
    // compound is an i32 heap handle in-guest) — NOT a scalar host-boundary byte. The bytes are not used
    // directly here: a producer's `make` functype takes the EXPORT's own params, a consumer's functype its
    // OWN params (`abi_params`); the closure signature only shapes the in-guest `call_indirect` (core
    // valtypes). This LIFTS the earlier scalar-arg fence for the round-trip: a `(-> (Tuple …) R)` closure
    // handed back and applied to a guest-built tuple now compiles. (A compound closure arg on the DIRECT-CALL
    // path — where the host supplies the arg — still declines: that needs host→guest decode.)
    for t in &arg_tys {
        valtype_of(t).ok_or_else(|| {
            Reject::decline(format!(
                "a closure argument of type {} has no machine representation",
                t.render_name(&db.name_ctx())
            ))
        })?;
    }
    // The closure RESULT is consumed by the consumer body (fed into whatever the consumer returns); the
    // consumer's OWN result type is validated separately (scalar/byte-rope/compound/collection below). So the
    // closure result need only be machine-representable too — a consumer applying a `(-> A (Tuple …))` closure
    // and returning that tuple crosses it as the consumer's compound result (already handled).
    valtype_of(&ret_ty).ok_or_else(|| {
        Reject::decline(format!(
            "a closure result of type {} has no machine representation",
            ret_ty.render_name(&db.name_ctx())
        ))
    })?;

    // Per-PRODUCER: its make spec (name `make-<export>`, param vts + bytes forwarded). Per-CONSUMER: its
    // consume spec (name = the export name, params classified Closure/Scalar, result vt + boundary shape).
    // Collected BEFORE `resource_escape_build` moves the layout.
    struct MakeSpec {
        def: usize,
        name: String,
        param_vts: Vec<crate::backend::wasm::lir::ValType>,
        param_bytes: Vec<u8>,
    }
    struct ConsumeSpec {
        def: usize,
        name: String,
        params: Vec<serialize::ConsumeParam>,
        ret_vt: crate::backend::wasm::lir::ValType,
        abi_params: Vec<envelope::ConsumeParamAbi>,
        result_byte: u8,
        ret_is_bytes: bool,
        ret_template: Option<crate::lower::ValueFormTemplate>,
        ret_descriptor: Option<Vec<u8>>,
    }
    let mut make_specs: Vec<MakeSpec> = Vec::new();
    for p in &producers {
        let param_vts: Vec<_> = p
            .params
            .iter()
            .map(|(_, t)| {
                valtype_of(t).ok_or_else(|| Reject::decline("producer param has no valtype"))
            })
            .collect::<Result<_, _>>()?;
        let param_bytes: Vec<u8> = p
            .params
            .iter()
            .map(|(_, t)| {
                closure_boundary_byte(t).ok_or_else(|| {
                    Reject::decline(format!(
                        "a producer parameter of type {} has no scalar host-boundary representation",
                        t.render_name(&db.name_ctx())
                    ))
                })
            })
            .collect::<Result<_, _>>()?;
        // In a round-trip the producer IS its export — the host calls it by the source export name (not a
        // `make-` prefix, which the multi-export path uses to distinguish N makes of ONE resource). So the
        // make function is exported under the producer's own name.
        make_specs.push(MakeSpec {
            def: p.def,
            name: p.name.clone(),
            param_vts,
            param_bytes,
        });
    }
    let mut consume_specs: Vec<ConsumeSpec> = Vec::new();
    for c in &consumers {
        // Classify each param IN SOURCE ORDER for BOTH the core (`ConsumeParam`: Closure → resource handle,
        // Scalar → its valtype) and the component boundary (`ConsumeParamAbi`: Closure → own<t>, Scalar →
        // its comp byte). A closure param may sit anywhere and there may be several (all same signature).
        let mut params = Vec::new();
        let mut abi_params = Vec::new();
        for (_, t) in &c.params {
            if matches!(t, crate::ty::Ty::Fn(_, _)) {
                params.push(serialize::ConsumeParam::Closure);
                abi_params.push(envelope::ConsumeParamAbi::Closure);
            } else {
                let vt = valtype_of(t)
                    .ok_or_else(|| Reject::decline("consumer scalar param has no valtype"))?;
                let byte = closure_boundary_byte(t).ok_or_else(|| {
                    Reject::decline(format!(
                        "a consumer scalar parameter of type {} has no scalar host-boundary representation",
                        t.render_name(&db.name_ctx())
                    ))
                })?;
                params.push(serialize::ConsumeParam::Scalar(vt));
                abi_params.push(envelope::ConsumeParamAbi::Scalar(byte));
            }
        }
        let ret_vt = valtype_of(&c.result)
            .ok_or_else(|| Reject::decline("consumer result has no machine valtype"))?;
        // The consumer's OWN result boundary shape — not the shared closure result. A consumer may return a
        // different type than the closure it applies (e.g. `(> (g x) 0)` → Bool). A byte-rope (`Bytes`/
        // `String`) result crosses as `list<u8>` (the compound consumer); a scalar takes its inline byte.
        let ret_is_bytes = matches!(
            c.result.strip_nominal(),
            crate::ty::Ty::Bytes | crate::ty::Ty::String
        );
        // A fixed-shape COMPOUND consumer result crosses as `list<u8>` carrying the value form (its own
        // template). `None` for byte-rope / scalar.
        let ret_template = if ret_is_bytes || closure_boundary_byte(&c.result).is_some() {
            None
        } else {
            crate::lower::runtime_value_form_template(c.result.strip_nominal(), &db.name_ctx())
        };
        // A VARIABLE-LENGTH collection consumer result crosses as `list<u8>` too, rendered via
        // `value-encode(rep, desc)` against its own shape descriptor. `None` for byte-rope/scalar/template.
        let ret_descriptor =
            if ret_is_bytes || ret_template.is_some() || closure_boundary_byte(&c.result).is_some()
            {
                None
            } else {
                // Any other value-encodable consumer result (a collection, a SUM — `Option`/`Result`/a user
                // sum — or a compound containing a variable-length element) crosses as `list<u8>` via the
                // runtime `value-encode` descriptor path. `sum_shape_descriptor` returns `None` for a scalar
                // (handled above) or an unrenderable shape. (A fixed-shape compound took `ret_template`.)
                crate::lower::sum_shape_descriptor(db, c.result.strip_nominal())
            };
        let consumer_result_byte =
            if ret_is_bytes || ret_template.is_some() || ret_descriptor.is_some() {
                0 // unused by the list-returning paths; the consumer returns list<u8>
            } else {
                closure_boundary_byte(&c.result).ok_or_else(|| {
                    Reject::decline(format!(
                        "a consumer result of type {} has no scalar host-boundary representation",
                        c.result.render_name(&db.name_ctx())
                    ))
                })?
            };
        consume_specs.push(ConsumeSpec {
            def: c.def,
            name: c.name.clone(),
            params,
            ret_vt,
            abi_params,
            result_byte: consumer_result_byte,
            ret_is_bytes,
            ret_template,
            ret_descriptor,
        });
    }
    // Per PLAIN export: source name (core + kebab boundary name), param bytes, scalar result byte.
    struct PlainSpec {
        def: usize,
        name: String,
        param_bytes: Vec<u8>,
        result_byte: u8,
    }
    let mut plain_specs: Vec<PlainSpec> = Vec::new();
    for e in &plain_exports {
        let param_bytes: Vec<u8> = e
            .params
            .iter()
            .map(|(_, t)| {
                closure_boundary_byte(t)
                    .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
            })
            .collect::<Result<_, _>>()?;
        let result_byte = closure_boundary_byte(&e.result).ok_or_else(|| {
            Reject::unsupported(format!(
                "a plain export `{}` returning {} has no scalar host-boundary representation \
                 (a compound result alongside a round-trip closure needs the compound-boundary emit)",
                e.name,
                e.result.render_name(&db.name_ctx())
            ))
        })?;
        plain_specs.push(PlainSpec {
            def: e.def,
            name: e.name.clone(),
            param_bytes,
            result_byte,
        });
    }

    // Lifted-body ops (a capturing producer closure reads its env in the lifted body).
    let lifted_bodies: Vec<crate::ast::StructId> = layout
        .lifted
        .iter()
        .enumerate()
        .filter(|(code, _)| layout.lifted_reached.get(*code).copied().unwrap_or(true))
        .map(|(_, l)| l.body)
        .collect();
    let mut lifted_ops: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();
    for &body in &lifted_bodies {
        select::collect_used_ops(db, body, &mut lifted_ops);
    }
    let any_bytes = consume_specs.iter().any(|c| c.ret_is_bytes);
    let any_compound = consume_specs.iter().any(|c| c.ret_template.is_some());
    let any_collection = consume_specs.iter().any(|c| c.ret_descriptor.is_some());
    let (imports, mut funcs, layout) = resource_escape_build(db, layout, |used| {
        used.insert("arr-get");
        used.insert("get-int");
        used.insert("drop");
        if any_bytes {
            // A byte-rope consumer copies its returned Bytes/String out via a `bytes-len`/`bytes-get` loop.
            used.insert("bytes-len");
            used.insert("bytes-get");
        }
        if any_compound {
            // A compound consumer walks its returned handle to fill the value form — a Bool leaf reads
            // `get-bool` (int + nested `arr-get` already covered).
            used.insert("get-bool");
        }
        if any_collection {
            // A collection consumer renders via `value-encode(rep, desc)` (build the descriptor Bytes + copy
            // the doc out).
            for op in [
                "value-encode",
                "bytes-alloc",
                "bytes-set",
                "bytes-len",
                "bytes-get",
            ] {
                used.insert(op);
            }
        }
        used.extend(lifted_ops.iter().copied());
    })?;
    if layout.lifted.is_empty() {
        return Err(Reject::decline(
            "a round-trip closure program produced no lifted lambda (the producer built no closure value)",
        ));
    }
    // APPEND the lifted closure bodies after the order defs.
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

    let ser_makes: Vec<serialize::ClosureMake> = make_specs
        .iter()
        .map(|m| {
            Ok(serialize::ClosureMake {
                export_name: m.name.clone(),
                export_abs: layout.abs(m.def).ok_or_else(|| {
                    Reject::decline("a producer export is not in the emission order")
                })?,
                param_vts: m.param_vts.clone(),
            })
        })
        .collect::<Result<_, Reject>>()?;
    let ser_consumers: Vec<serialize::ClosureConsume> = consume_specs
        .iter()
        .map(|c| {
            Ok(serialize::ClosureConsume {
                export_name: c.name.clone(),
                consume_abs: layout.abs(c.def).ok_or_else(|| {
                    Reject::decline("a consumer export is not in the emission order")
                })?,
                params: c.params.clone(),
                ret_vt: c.ret_vt,
                ret_is_bytes: c.ret_is_bytes,
                ret_template: c.ret_template.clone(),
                ret_descriptor: c.ret_descriptor.clone(),
            })
        })
        .collect::<Result<_, Reject>>()?;

    let ser_plain: Vec<serialize::PlainExport> = plain_specs
        .iter()
        .map(|p| {
            Ok(serialize::PlainExport {
                export_name: p.name.clone(),
                body_abs: layout.abs(p.def).ok_or_else(|| {
                    Reject::decline("a plain export is not in the emission order")
                })?,
            })
        })
        .collect::<Result<_, Reject>>()?;
    let main_core = serialize::roundtrip_resource_core_module(
        &funcs,
        &imports,
        &ser_makes,
        &ser_consumers,
        &ser_plain,
        lifted_type_idx,
        &layout,
    )
    .map_err(Reject::decline)?;
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    let abi_makes: Vec<envelope::ClosureMakeAbi> = make_specs
        .iter()
        .map(|m| envelope::ClosureMakeAbi {
            name: m.name.clone(),
            make_param_bytes: m.param_bytes.clone(),
        })
        .collect();
    let abi_consumers: Vec<envelope::ClosureConsumeAbi> = consume_specs
        .iter()
        .map(|c| envelope::ClosureConsumeAbi {
            name: c.name.clone(),
            params: c.abi_params.clone(),
            result_byte: c.result_byte,
            // The envelope's `ret_is_bytes` means "crosses as list<u8>" — byte-rope OR compound OR collection.
            ret_is_bytes: c.ret_is_bytes || c.ret_template.is_some() || c.ret_descriptor.is_some(),
        })
        .collect();
    let abi_plain: Vec<envelope::PlainExportAbi> = plain_specs
        .iter()
        .map(|p| envelope::PlainExportAbi {
            name: p.name.clone(),
            core_name: p.name.clone(),
            param_bytes: p.param_bytes.clone(),
            result_byte: p.result_byte,
        })
        .collect();
    Ok(envelope::assemble_roundtrip_resource_mixed(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_makes,
        &abi_consumers,
        &abi_plain,
    ))
}

/// Emit the DISTINCT-SIGNATURE ROUND-TRIP component: producers + consumers of G DIFFERENT closure
/// signatures, each crossing as its own resource type. The round-trip emit generalized to N groups: group
/// exports by signature (a producer by its result, a consumer by its closure-param type), then per group
/// build an `RtSigGroup` (serializer) + `RtSigGroupAbi` (envelope) carrying that group's makes + consumers.
/// Each group's `resource-new-<g>`/`resource-rep-<g>` intrinsics are supplied by the envelope. Same shape
/// as `emit_roundtrip_resource` but partitioned by signature (so a program mixing `(-> Int64 Int64)` and
/// `(-> Int64 Bool)` producers+consumers now compiles, where it used to decline "mixing DIFFERENT signatures").
pub(super) fn emit_distinct_sig_roundtrip_resource(
    db: &mut Db,
    layout: &Layout,
    _spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    use crate::backend::wasm::lir::valtype_of;
    // A producer's signature is its result; a consumer's is its (sole) closure-param type. Build the group
    // list (distinct signatures, first-seen order) and, per export, its group + role.
    let producer_sig = |e: &crate::layout::ExportPlan| -> Option<crate::ty::Ty> {
        matches!(e.result, crate::ty::Ty::Fn(_, _)).then(|| e.result.clone())
    };
    let consumer_sigs = |e: &crate::layout::ExportPlan| -> Vec<crate::ty::Ty> {
        e.params
            .iter()
            .filter(|(_, t)| matches!(t, crate::ty::Ty::Fn(_, _)))
            .map(|(_, t)| t.clone())
            .collect()
    };
    // A closure TRANSFORMER (both a closure result AND a closure param) is out of scope here too.
    if let Some(t) = layout
        .exports
        .iter()
        .find(|e| producer_sig(e).is_some() && !consumer_sigs(e).is_empty())
    {
        return Err(Reject::unsupported(format!(
            "the export `{}` both receives and returns a closure (a closure transformer) — the combined \
             receive-and-return closure boundary emit is unbuilt (DESIGN-closure-host-resource-rcdzc.md)",
            t.name
        )));
    }
    // Collect the distinct signatures (first-seen), and validate a consumer has exactly one closure param.
    let mut sigs: Vec<crate::ty::Ty> = Vec::new();
    let group_of = |s: &crate::ty::Ty, sigs: &mut Vec<crate::ty::Ty>| -> usize {
        sigs.iter().position(|x| x == s).unwrap_or_else(|| {
            sigs.push(s.clone());
            sigs.len() - 1
        })
    };
    for e in &layout.exports {
        if let Some(s) = producer_sig(e) {
            group_of(&s, &mut sigs);
        }
        let cs = consumer_sigs(e);
        if !cs.is_empty() {
            if cs.len() != 1 {
                return Err(Reject::unsupported(
                    "a distinct-signature round-trip consumer with more than one closure parameter is \
                     not supported",
                ));
            }
            group_of(&cs[0], &mut sigs);
        }
    }
    // Effect-escape fence.
    {
        let mut escaping = Vec::new();
        for l in &layout.lifted {
            host::collect_host_imports(db, l.body, &mut escaping);
        }
        if let Some(h) = escaping.first() {
            return Err(Reject::coded(
                crate::diag::Code::ClosureEscapesEffect,
                format!(
                    "a closure that performs an effect ({}.{}) cannot cross the host boundary — the \
                     closure's handler context does not travel with it (closures escaping effects are \
                     not supported)",
                    h.effect, h.op
                ),
            ));
        }
    }
    // Validate every group's signature is MACHINE-representable (arg + result). Like the single-sig
    // round-trip, a distinct-sig round-trip applies its handed-back closures ENTIRELY IN-GUEST (each `(g …)`
    // in a consumer body), so a closure ARGUMENT is built guest-side and never crosses the host boundary —
    // only the closure HANDLE (an `own<t_g>` resource, i32) + the consumer's own scalar params cross. So a
    // closure arg/result need only have a machine valtype (a value-heap compound is an i32 handle in-guest),
    // NOT a scalar host-boundary byte. The signature's ABI bytes are not used directly here (a make functype
    // takes the export's own params, a consumer's its own; the signature only shapes the in-guest
    // `call_indirect`). A compound closure arg on the DIRECT-CALL path still declines (host→guest decode).
    for s in &sigs {
        let mut cur = s.clone();
        while let crate::ty::Ty::Fn(dom, rng) = cur {
            valtype_of(&dom)
                .ok_or_else(|| closure_boundary_reject("argument", &dom, &db.name_ctx()))?;
            cur = *rng;
        }
        valtype_of(&cur).ok_or_else(|| closure_boundary_reject("result", &cur, &db.name_ctx()))?;
    }

    // Per export: its make/consume spec + which group. Collected before the build moves the layout.
    struct MakeS {
        def: usize,
        group: usize,
        name: String,
        param_vts: Vec<crate::backend::wasm::lir::ValType>,
        param_bytes: Vec<u8>,
    }
    struct ConsS {
        def: usize,
        group: usize,
        name: String,
        params: Vec<serialize::ConsumeParam>,
        abi_params: Vec<envelope::ConsumeParamAbi>,
        ret_vt: crate::backend::wasm::lir::ValType,
        result_byte: u8,
        ret_is_bytes: bool,
        ret_template: Option<crate::lower::ValueFormTemplate>,
        ret_descriptor: Option<Vec<u8>>,
    }
    struct PlainS {
        def: usize,
        name: String,
        param_bytes: Vec<u8>,
        result_byte: u8,
    }
    let mut makes: Vec<MakeS> = Vec::new();
    let mut cons: Vec<ConsS> = Vec::new();
    let mut plains: Vec<PlainS> = Vec::new();
    for e in &layout.exports {
        if let Some(s) = producer_sig(e) {
            let group = sigs.iter().position(|x| *x == s).unwrap();
            let param_vts: Vec<_> = e
                .params
                .iter()
                .map(|(_, t)| {
                    valtype_of(t).ok_or_else(|| Reject::decline("producer param has no valtype"))
                })
                .collect::<Result<_, _>>()?;
            let param_bytes: Vec<u8> = e
                .params
                .iter()
                .map(|(_, t)| {
                    closure_boundary_byte(t)
                        .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
                })
                .collect::<Result<_, _>>()?;
            makes.push(MakeS {
                def: e.def,
                group,
                name: e.name.clone(),
                param_vts,
                param_bytes,
            });
        } else if consumer_sigs(e).is_empty() {
            // A PLAIN (non-closure) export — rides alongside the round trip as an ordinary top-level func.
            let param_bytes: Vec<u8> = e
                .params
                .iter()
                .map(|(_, t)| {
                    closure_boundary_byte(t)
                        .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
                })
                .collect::<Result<_, _>>()?;
            let result_byte = closure_boundary_byte(&e.result).ok_or_else(|| {
                Reject::unsupported(format!(
                    "a plain export `{}` returning {} has no scalar host-boundary representation \
                     (a compound result alongside a round-trip closure needs the compound-boundary emit)",
                    e.name,
                    e.result.render_name(&db.name_ctx())
                ))
            })?;
            plains.push(PlainS {
                def: e.def,
                name: e.name.clone(),
                param_bytes,
                result_byte,
            });
        } else {
            let cs = consumer_sigs(e);
            let group = sigs.iter().position(|x| *x == cs[0]).unwrap();
            let mut params = Vec::new();
            let mut abi_params = Vec::new();
            for (_, t) in &e.params {
                if matches!(t, crate::ty::Ty::Fn(_, _)) {
                    params.push(serialize::ConsumeParam::Closure);
                    abi_params.push(envelope::ConsumeParamAbi::Closure);
                } else {
                    let vt = valtype_of(t)
                        .ok_or_else(|| Reject::decline("consumer scalar param has no valtype"))?;
                    let byte = closure_boundary_byte(t)
                        .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))?;
                    params.push(serialize::ConsumeParam::Scalar(vt));
                    abi_params.push(envelope::ConsumeParamAbi::Scalar(byte));
                }
            }
            let ret_vt = valtype_of(&e.result)
                .ok_or_else(|| Reject::decline("consumer result has no valtype"))?;
            // A byte-rope (`Bytes`/`String`) consumer result crosses as `list<u8>` (raw payload); a scalar
            // takes its inline byte; a fixed-shape COMPOUND crosses as `list<u8>` carrying the value form; a
            // VARIABLE-LENGTH collection (List/Map/Set) crosses as `list<u8>` rendered via `value-encode`.
            let ret_is_bytes = matches!(
                e.result.strip_nominal(),
                crate::ty::Ty::Bytes | crate::ty::Ty::String
            );
            let ret_template = if ret_is_bytes || closure_boundary_byte(&e.result).is_some() {
                None
            } else {
                crate::lower::runtime_value_form_template(e.result.strip_nominal(), &db.name_ctx())
            };
            let ret_descriptor = if ret_is_bytes
                || ret_template.is_some()
                || closure_boundary_byte(&e.result).is_some()
            {
                None
            } else {
                // Any other value-encodable consumer result (a collection, a SUM, or a compound containing a
                // variable-length element) → the runtime `value-encode` descriptor path. `sum_shape_descriptor`
                // returns `None` for a scalar or unrenderable shape. (A fixed-shape compound took `ret_template`.)
                crate::lower::sum_shape_descriptor(db, e.result.strip_nominal())
            };
            let result_byte = if ret_is_bytes || ret_template.is_some() || ret_descriptor.is_some()
            {
                0 // unused by the list-returning paths; the consumer returns list<u8>
            } else {
                closure_boundary_byte(&e.result)
                    .ok_or_else(|| closure_boundary_reject("result", &e.result, &db.name_ctx()))?
            };
            cons.push(ConsS {
                def: e.def,
                group,
                name: e.name.clone(),
                params,
                abi_params,
                ret_vt,
                result_byte,
                ret_is_bytes,
                ret_template,
                ret_descriptor,
            });
        }
    }
    // Require at least one producer per group (a consumer group with no producer would need a host-made
    // closure — out of scope).
    for gi in 0..sigs.len() {
        if !makes.iter().any(|m| m.group == gi) {
            return Err(Reject::decline(
                "a distinct-signature round-trip has a consumer whose closure signature no producer mints \
                 (a host-fabricated closure is out of scope)",
            ));
        }
    }

    // Lifted-body ops + build with 2*G intrinsics.
    let lifted_bodies: Vec<crate::ast::StructId> = layout
        .lifted
        .iter()
        .enumerate()
        .filter(|(code, _)| layout.lifted_reached.get(*code).copied().unwrap_or(true))
        .map(|(_, l)| l.body)
        .collect();
    let mut lifted_ops: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();
    for &body in &lifted_bodies {
        select::collect_used_ops(db, body, &mut lifted_ops);
    }
    let intrinsics = (2 * sigs.len()) as u32;
    let any_bytes = cons.iter().any(|c| c.ret_is_bytes);
    let any_compound = cons.iter().any(|c| c.ret_template.is_some());
    let any_collection = cons.iter().any(|c| c.ret_descriptor.is_some());
    let (imports, mut funcs, layout) = resource_escape_build_n(db, layout, intrinsics, |used| {
        used.insert("arr-get");
        used.insert("get-int");
        used.insert("drop");
        if any_bytes {
            // A byte-rope consumer copies its returned Bytes/String out via a `bytes-len`/`bytes-get` loop.
            used.insert("bytes-len");
            used.insert("bytes-get");
        }
        if any_compound {
            // A compound consumer walks its returned handle to fill the value form — a Bool leaf reads
            // `get-bool` (int + nested `arr-get` already covered).
            used.insert("get-bool");
        }
        if any_collection {
            // A collection consumer renders via `value-encode(rep, desc)` (build the descriptor Bytes + copy
            // the doc out).
            for op in [
                "value-encode",
                "bytes-alloc",
                "bytes-set",
                "bytes-len",
                "bytes-get",
            ] {
                used.insert(op);
            }
        }
        used.extend(lifted_ops.iter().copied());
    })?;
    if layout.lifted.is_empty() {
        return Err(Reject::decline(
            "a distinct-signature round-trip produced no lifted lambda",
        ));
    }
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

    // Build the per-group serializer + envelope specs, in group order.
    let mut ser_groups: Vec<serialize::RtSigGroup> = Vec::new();
    let mut abi_groups: Vec<envelope::RtSigGroupAbi> = Vec::new();
    for gi in 0..sigs.len() {
        let mut ser_makes = Vec::new();
        let mut abi_makes = Vec::new();
        for m in makes.iter().filter(|m| m.group == gi) {
            let export_abs = layout
                .abs(m.def)
                .ok_or_else(|| Reject::decline("a producer is not in the emission order"))?;
            ser_makes.push(serialize::ClosureMake {
                export_name: m.name.clone(),
                export_abs,
                param_vts: m.param_vts.clone(),
            });
            abi_makes.push(envelope::ClosureMakeAbi {
                name: m.name.clone(),
                make_param_bytes: m.param_bytes.clone(),
            });
        }
        let mut ser_cons = Vec::new();
        let mut abi_cons = Vec::new();
        for c in cons.iter().filter(|c| c.group == gi) {
            let consume_abs = layout
                .abs(c.def)
                .ok_or_else(|| Reject::decline("a consumer is not in the emission order"))?;
            ser_cons.push(serialize::ClosureConsume {
                export_name: c.name.clone(),
                consume_abs,
                params: c.params.clone(),
                ret_vt: c.ret_vt,
                ret_is_bytes: c.ret_is_bytes,
                ret_template: c.ret_template.clone(),
                ret_descriptor: c.ret_descriptor.clone(),
            });
            abi_cons.push(envelope::ClosureConsumeAbi {
                name: c.name.clone(),
                params: c.abi_params.clone(),
                result_byte: c.result_byte,
                // The envelope's `ret_is_bytes` means "crosses as list<u8>" — byte-rope OR compound OR
                // collection.
                ret_is_bytes: c.ret_is_bytes
                    || c.ret_template.is_some()
                    || c.ret_descriptor.is_some(),
            });
        }
        ser_groups.push(serialize::RtSigGroup {
            makes: ser_makes,
            consumers: ser_cons,
        });
        abi_groups.push(envelope::RtSigGroupAbi {
            makes: abi_makes,
            consumers: abi_cons,
        });
    }

    // Plain-export specs: resolve each body's core-func index post-build.
    let ser_plain: Vec<serialize::PlainExport> = plains
        .iter()
        .map(|p| {
            Ok(serialize::PlainExport {
                export_name: p.name.clone(),
                body_abs: layout.abs(p.def).ok_or_else(|| {
                    Reject::decline("a plain export is not in the emission order")
                })?,
            })
        })
        .collect::<Result<_, Reject>>()?;
    let abi_plain: Vec<envelope::PlainExportAbi> = plains
        .iter()
        .map(|p| envelope::PlainExportAbi {
            name: p.name.clone(),
            core_name: p.name.clone(),
            param_bytes: p.param_bytes.clone(),
            result_byte: p.result_byte,
        })
        .collect();
    let main_core = serialize::distinct_sig_roundtrip_core_module(
        &funcs,
        &imports,
        &ser_groups,
        &ser_plain,
        &layout,
    )
    .map_err(Reject::decline)?;
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_distinct_sig_roundtrip_resource_mixed(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_groups,
        &abi_plain,
    ))
}

/// Emit the runtime-import + resource escape component for a single nullary export returning a RUNTIME
/// `Bytes` (a `concat`/recursion-built sequence, not a compile-time constant). Mirrors
/// [`emit_runtime_resource`], but the escape form is [`serialize::EscapeForm::RuntimeBytes`] — its
/// `encode()` is the LOOPING walker (`encode_bytes_walk_body`) that writes a variable-length value form.
/// The walker's ops (`bytes-len`, `bytes-get`) appear only in the synthesized encode body, plus `drop`
/// for the `own<t>` release — added here since they are not in any reachable Core.
pub(super) fn emit_runtime_bytes_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    form: &crate::lower::RuntimeBytesForm,
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    // Scan the top-level defs AND the lambda-lifted closure bodies (see `append_lifted_bodies`) so an op
    // used only inside a closure is imported too — else its `CallImport` resolves to `u32::MAX` (invalid).
    collect_module_used_ops(db, layout, &mut used)?;
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

    // PEER-IN-RESOURCE-ESCAPE (task #6 — the STRING/Bytes result path, the full `(-> String String)` model
    // call where the peer's String completion IS the entrypoint result). Same fusion as the other three
    // resource-escape paths, but this one carries the value-resource METHODS (len/is-empty/to-bytes), so it
    // dispatches to the methods-carrying fused assembler.
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
    // HOST-effect × STRING/BYTES-resource-escape WITH-METHODS FUSION (the host-side mirror of the peer
    // with-methods path below). A host-delegated effect reached in a body whose String/Bytes result escapes
    // — `main(x) = host H in (Bytes.of (list (H.h x)))` — dispatches to `assemble_host_runtime_resource_with_
    // scalar_methods`, laying host ops as leading `"host"` imports (`leading_is_host = true`) + the make/
    // encode/len/is-empty/to-bytes methods. SCOPE: scalar/unit host ops (a STRING-param host op takes the
    // shared-memory `_mem` variant — a later increment). A host effect ALONGSIDE a peer effect, or MORE than
    // one host effect, is a further fusion — decline cleanly (mirrors the Flat/Sum/RecursiveSum host arms).
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
        // DECLINE-DON'T-MISCOMPILE (B2 pending): a COMPOUND host RESULT (string / bytes / list / tuple /
        // record / option / result / variant / enum — anything `spilled_result`/`enum_result`) reached in
        // this with-methods resource escape needs the B1 result-lift machinery this scalar-only assembler
        // does NOT yet declare; without it the lift op resolves to an out-of-range func index and the
        // component fails validation (CDZ0910 "unknown function"). Decline CLEANLY until B2 threads the
        // lift through the `assemble_host_runtime_resource*` sites. A scalar/unit host result IS supported.
        if host_imports
            .iter()
            .any(|h| h.spilled_result.is_some() || h.enum_result.is_some())
        {
            return Err(Reject::unsupported(
                "a host op with a compound result (string / bytes / list / record / option / variant / \
                 enum) escaping directly as a resource entrypoint is not supported (a scalar/unit host op \
                 result-escaping as a resource IS supported)",
            ));
        }
        let iface = host_imports[0].effect.clone();
        if host_imports.iter().any(|hi| hi.effect != iface) {
            return Err(Reject::declined(
                crate::diag::DeclineId::WasmMultiHostEffectDelegation,
                "delegating more than one host effect from a resource-escaping entrypoint is not \
                 supported (one interface per envelope)",
            ));
        }
        let h = host_imports.len() as u32;
        let k = imports.len() as u32;
        let host_order: Vec<(String, String)> = host_imports
            .iter()
            .map(|hi| (hi.effect.clone(), hi.op.clone()))
            .collect();
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
            Reject::decline("the escaping bytes export is not in the emission order")
        })?;

        let core_methods = [
            serialize::CoreMethod::Len,
            serialize::CoreMethod::IsEmpty,
            serialize::CoreMethod::ToBytes,
        ];
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
            serialize::EscapeForm::RuntimeBytes(form),
            &core_methods,
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
        let scalar_methods = [
            envelope::ScalarMethod {
                boundary_name: "len",
                core_export: "t-len",
                result: envelope::MethodResult::Scalar(crate::backend::wasm::wasm_abi::COMP_U32),
            },
            envelope::ScalarMethod {
                boundary_name: "is-empty",
                core_export: "t-is-empty",
                result: envelope::MethodResult::Scalar(crate::backend::wasm::wasm_abi::COMP_BOOL),
            },
            envelope::ScalarMethod {
                boundary_name: "to-bytes",
                core_export: "t-to-bytes",
                result: envelope::MethodResult::ListU8,
            },
        ];
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
        return Ok(
            envelope::assemble_host_runtime_resource_with_scalar_methods(
                &main_core,
                &dtor_core,
                &imports,
                &import_name,
                &iface,
                &host_fns,
                &make_param_bytes,
                &scalar_methods,
            ),
        );
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
        .ok_or_else(|| Reject::decline("the escaping bytes export is not in the emission order"))?;

    // VM-1/VM-3: a Bytes result crosses as a resource carrying make + encode + `len : borrow<t> -> u32`
    // (= `bytes-len(rep)`) + `is-empty : borrow<t> -> bool` (= `bytes-len == 0`) + `to-bytes : borrow<t>
    // -> list<u8>` (the RAW payload). The core emits `t-len`/`t-is-empty`/`t-to-bytes` (`bytes-len`/
    // `bytes-get` already imported for the encode walker), and the envelope lifts the three extra methods.
    let core_methods = [
        serialize::CoreMethod::Len,
        serialize::CoreMethod::IsEmpty,
        serialize::CoreMethod::ToBytes,
    ];
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
        serialize::EscapeForm::RuntimeBytes(form),
        &core_methods,
        &make_param_vts,
        &make_core_slots,
        &escape_lifted_table(layout),
        0, // build-once static compounds not threaded on this path (byte-identical; a follow-up increment)
        &[], // no static-compound init
    )
    .map_err(Reject::decline)?;
    // DEBUG: same as the flat/sum resource paths — the user bodies lead the escape core's code section,
    // so the `name` + `.debug_*` sections attribute correctly; the synthesized bytes walker has no
    // `src_body` and gets no row.
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    let scalar_methods = [
        envelope::ScalarMethod {
            boundary_name: "len",
            core_export: "t-len",
            result: envelope::MethodResult::Scalar(crate::backend::wasm::wasm_abi::COMP_U32),
        },
        envelope::ScalarMethod {
            boundary_name: "is-empty",
            core_export: "t-is-empty",
            result: envelope::MethodResult::Scalar(crate::backend::wasm::wasm_abi::COMP_BOOL),
        },
        envelope::ScalarMethod {
            boundary_name: "to-bytes",
            core_export: "t-to-bytes",
            result: envelope::MethodResult::ListU8,
        },
    ];
    if extern_imports.is_empty() {
        return Ok(envelope::assemble_runtime_resource_with_scalar_methods(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &scalar_methods,
        ));
    }
    // The FUSED with-methods envelope: a peer op is reached in a body whose STRING/Bytes result escapes
    // (the full `(-> String String)` model call). Supports multiple peer interfaces (grouped by op_ifaces).
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
    Ok(
        envelope::assemble_extern_runtime_resource_with_scalar_methods(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &peer_fns,
            &op_ifaces,
            &make_param_bytes,
            &scalar_methods,
        ),
    )
}
