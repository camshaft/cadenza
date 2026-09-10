//! Multi-export closure-resource component emitters — `emit_multi_closure_resource` (several exports
//! whose closure results share ONE signature cross as one resource type with N `make` fns + one shared
//! `call`) and `emit_mixed_closure_resource`. Extracted verbatim from `backend/wasm/mod.rs` to keep it
//! under `xtask_support::MAX_SOURCE_BYTES` (512 KiB); pure code move, behavior-neutral. `use super::*`
//! brings the parent `backend::wasm` module items into scope, as the other `wasm` submodules do. The
//! moved fns are `pub(super)` so the parent's `use closure_resource::*;` re-imports them, leaving every
//! call site in `mod.rs` unchanged. Both are internal to `backend::wasm` (0 external references).
use super::*;

/// Emit the MULTI-EXPORT closure-resource component: several exports whose results are all closures of the
/// SAME signature `(-> A… R)` cross together as one resource type with N `make-<name>` functions sharing
/// ONE `call` (`DESIGN-closure-host-resource-rcdzc.md`, multi-export). Each export's body builds its own
/// closure cell (occupying its own funcref-table slot); its `make` calls that body + `resource.new`s the
/// handle. The shared `call` recovers the code slot from the rep at call time, so it dispatches whichever
/// closure a handle names (proven by the `multi_export_closures_share_one_call` oracle). Distinct
/// signatures (N resource types) are a later slice — declined by the caller.
pub(super) fn emit_multi_closure_resource(
    db: &mut Db,
    layout: &Layout,
    export_defs: &[usize],
    result: &crate::ty::Ty,
    _spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    use crate::backend::wasm::lir::valtype_of;
    // Flatten the shared closure signature → arg types + result. All exports share it (the caller checked).
    let mut arg_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = result.clone();
    while let crate::ty::Ty::Fn(dom, rng) = cur {
        arg_tys.push((*dom).clone());
        cur = *rng;
    }
    let ret_ty = cur;
    // Reject a closure escaping an effect (same rule as the single-export path): scan the lifted bodies.
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
                     closure's handler context does not travel with it, so the effect would have no home \
                     when the host invokes it (closures escaping effects are not supported)",
                    h.effect, h.op
                ),
            ));
        }
    }
    // DIRECT-CALL COMPOUND ARG (multi-export): a single fixed-shape scalar tuple/record arg shared by all
    // exports crosses as a native component `tuple<…>` the canonical ABI flattens; the shared `call` rebuilds
    // the cell from the flat fields (`TupleArgRebuild`). The tuple may be the SOLE arg OR sit among scalar args
    // (prefix/suffix). Detected here so the scalar `arg_bytes` decline below doesn't reject it. 5-tuple =
    // (tuple field bytes, full flattened core vts, prefix scalar bytes, suffix scalar bytes, rebuild).
    let tuple_arg: Option<CompoundArgBoundary> = if arg_tys.len() == 1 {
        fixed_shape_scalar_tuple_arg(&arg_tys[0])
            .map(|(fb, fv, rb)| (fb, fv, Vec::new(), Vec::new(), rb))
    } else {
        single_compound_among_scalars(arg_tys.as_slice())
    };
    // A fixed-shape compound arg with a NESTED compound field (shared by all closure exports) — SOLE or among
    // scalars. Detected when the flat `tuple_arg` is None.
    let nested_tuple: Option<NestedCompoundArgBoundary> = if tuple_arg.is_some() {
        None
    } else {
        nested_sole_or_among_scalars(arg_tys.as_slice())
    };
    // N-COMPOUND-ARGS (≥2 fixed-shape tuple/record args) shared by all closure exports: the `ArgSlot` slot
    // model (each tuple its own native `tuple<…>`, rebuilt in-guest from its `TupleArgRebuild`). Detected only
    // when neither single-tuple classifier fired. Scoped this increment: SCALAR result on the multi-export
    // shared `call` (a list result over ≥2 tuples on multi-export is a follow-on).
    let multi_args: Option<(
        Vec<crate::backend::wasm::envelope::ArgSlot>,
        Vec<crate::backend::wasm::lir::ValType>,
        Vec<crate::backend::wasm::serialize::TupleArgRebuild>,
    )> = if tuple_arg.is_none() && nested_tuple.is_none() {
        multi_compound_args(arg_tys.as_slice())
    } else {
        None
    };
    // A SOLE `(Option/Result scalar)` arg shared by all same-sig closures crosses as a native `option<…>`/
    // `result<…>` the ABI flattens to `(disc, payload)`; the shared `call` rebuilds the sum cell via
    // `SumArgRebuild`, the envelope mints the boundary type via the returned `ArgSlot`. Scoped: SCALAR result.
    let sum_arg: Option<(
        crate::backend::wasm::envelope::ArgSlot,
        Vec<crate::backend::wasm::lir::ValType>,
        crate::backend::wasm::serialize::SumArgRebuild,
    )> = if tuple_arg.is_none()
        && nested_tuple.is_none()
        && multi_args.is_none()
        && arg_tys.len() == 1
    {
        fixed_shape_option_scalar_arg(db, &arg_tys[0])
            .or_else(|| fixed_shape_result_compound_arg(db, &arg_tys[0]))
    } else {
        None
    };
    let arg_bytes: Vec<u8> = if tuple_arg.is_some()
        || nested_tuple.is_some()
        || multi_args.is_some()
        || sum_arg.is_some()
    {
        Vec::new() // the flattened fields are carried by tuple_arg/nested_tuple/multi_args/sum_arg, not arg_bytes
    } else {
        arg_tys
            .iter()
            .map(|t| {
                closure_boundary_byte(t)
                    .ok_or_else(|| closure_boundary_reject("argument", t, &db.name_ctx()))
            })
            .collect::<Result<_, _>>()?
    };
    // A byte-rope (`Bytes`/`String`) shared closure result crosses as `list<u8>` — the N-makes-one-`call`
    // memory/realloc list-`call` (`multi_closure_bytes_resource_core_module` + the bytes envelope). A scalar
    // result takes the by-value shared call. All exports share the signature, so one `ret_is_bytes` decides.
    let ret_is_bytes = matches!(
        ret_ty.strip_nominal(),
        crate::ty::Ty::Bytes | crate::ty::Ty::String
    );
    // A COMPOUND (tuple/record/sum) shared result crosses as `list<u8>` carrying the value form — the shared
    // `call` walks each closure's returned handle into the ONE value-form template (all exports share the
    // result type). `None` for a byte-rope (its own list path) / a scalar (by value) / a no-template compound.
    let ret_template = if ret_is_bytes || closure_boundary_byte(&ret_ty).is_some() {
        None
    } else {
        crate::lower::runtime_value_form_template(ret_ty.strip_nominal(), &db.name_ctx())
    };
    let ret_is_compound = ret_template.is_some();
    // A VARIABLE-LENGTH collection (List/Map/Set) shared result → the value-encode core (all exports share
    // the result type → the ONE shape descriptor). `None` for bytes/scalar/fixed-template.
    let ret_descriptor =
        if ret_is_bytes || ret_is_compound || closure_boundary_byte(&ret_ty).is_some() {
            None
        } else {
            // Any OTHER machine-representable result the runtime `value-encode` walker can render — a
            // variable-length collection (`List`/`Map`/`Set`), a SUM (`Option`/`Result`/a user sum), or a
            // compound (tuple/record) CONTAINING a variable-length element — escapes as `list<u8>` via a
            // compiler-baked shape DESCRIPTOR. `sum_shape_descriptor` returns `None` for a scalar (handled
            // above) or an unrenderable shape, so this is a safe general fallback beyond the fixed
            // List/Map/Set set. (A fixed-shape compound already took the cheaper static `ret_template` path.)
            crate::lower::sum_shape_descriptor(db, ret_ty.strip_nominal())
        };
    let ret_is_collection = ret_descriptor.is_some();
    // A fixed-shape compound ARG now composes with EVERY multi-export result shape too — scalar, byte-rope,
    // fixed-compound (value-form), collection (value-encode): all three multi list-result cores + the shared
    // multi list<u8> envelope thread the `TupleArgRebuild`. No result-shape decline remains for a multi-export
    // tuple arg. (A compound-arg-alongside-others / variable-length-field compound arg still declines at
    // detection.)
    let result_byte = if ret_is_bytes || ret_is_compound || ret_is_collection {
        0 // unused by the list-returning paths; `call` returns list<u8>
    } else {
        closure_boundary_byte(&ret_ty)
            .ok_or_else(|| closure_boundary_reject("result", &ret_ty, &db.name_ctx()))?
    };
    // Core call-arg valtypes: the FLATTENED tuple fields when a (flat or nested) tuple arg or ≥2 tuple args,
    // else each arg's own valtype.
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = if let Some((_, all_vts, _, _, _)) =
        &tuple_arg
    {
        all_vts.clone()
    } else if let Some((_, leaf_vts, _, _, _, _)) = &nested_tuple {
        leaf_vts.clone() // the DEPTH-FIRST flattened leaf params of a nested tuple arg
    } else if let Some((_, all_vts, _)) = &multi_args {
        all_vts.clone() // the flattened leaves of EVERY tuple/scalar arg, in order (N-compound-args)
    } else if let Some((_, payload_vts, _)) = &sum_arg {
        // sum flattens to (disc: i32, <payload leaves…>) — scalar payload = 1 leaf, compound = its leaves.
        let mut vts = vec![crate::backend::wasm::lir::ValType::I32];
        vts.extend(payload_vts.iter().copied());
        vts
    } else {
        arg_tys
            .iter()
            .map(|t| {
                valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype"))
            })
            .collect::<Result<_, _>>()?
    };
    let ret_vt = valtype_of(&ret_ty)
        .ok_or_else(|| Reject::decline("closure result has no machine valtype"))?;

    // Per export: its params (each `make` forwards them) as core valtypes + boundary bytes. Collected
    // BEFORE `resource_escape_build` moves the layout (params live on the pre-build `layout.exports`).
    struct MakeSpec {
        def: usize,
        name: String,
        param_vts: Vec<crate::backend::wasm::lir::ValType>,
        param_bytes: Vec<u8>,
    }
    let mut make_specs: Vec<MakeSpec> = Vec::new();
    for &def in export_defs {
        let export = layout
            .exports
            .iter()
            .find(|e| e.def == def)
            .ok_or_else(|| Reject::decline("a closure export is not in the layout"))?;
        let param_vts: Vec<_> = export
            .params
            .iter()
            .map(|(_, t)| {
                valtype_of(t)
                    .ok_or_else(|| Reject::decline("closure export param has no machine valtype"))
            })
            .collect::<Result<_, _>>()?;
        let param_bytes: Vec<u8> = export
            .params
            .iter()
            .map(|(_, t)| {
                closure_boundary_byte(t)
                    .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
            })
            .collect::<Result<_, _>>()?;
        make_specs.push(MakeSpec {
            def,
            name: format!("make-{}", export.name),
            param_vts,
            param_bytes,
        });
    }

    // Lifted-body ops (a capturing closure's env reads appear only in the lifted bodies).
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
    let (imports, mut funcs, layout) = resource_escape_build(db, layout, |used| {
        used.insert("arr-get");
        used.insert("get-int");
        used.insert("drop");
        if ret_is_bytes {
            used.insert("bytes-len");
            used.insert("bytes-get");
        }
        // A COMPOUND-result shared `call` walks the returned handle to fill the value form — a Bool leaf
        // reads `get-bool` (int leaves + nested `arr-get` already covered above).
        if ret_is_compound {
            used.insert("get-bool");
        }
        // A collection-result shared `call` renders via `value-encode(rep, desc)` (build the descriptor
        // Bytes + copy the doc out).
        if ret_is_collection {
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
        // A DIRECT-CALL fixed-shape scalar tuple/record ARG rebuilds its cell in the `call` body
        // (`emit_tuple_rebuild`): register the ops it emits — `arr-alloc`/`arr-set` + each field's box op
        // (`box-int`/`box-bool`/`box-float`/`box-float32`), which appear only in the synthesized rebuild.
        // Without the box op a Bool/Float field panicked ("rebuild op imported"); see `emit_closure_resource`.
        if let Some((_, _, _, _, rebuild)) = &tuple_arg {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for f in &rebuild.fields {
                f.collect_box_ops(&mut |bop| {
                    used.insert(bop);
                });
            }
        }
        if let Some((_, _, rebuild, _, _, _)) = &nested_tuple {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for f in &rebuild.fields {
                f.collect_box_ops(&mut |bop| {
                    used.insert(bop);
                });
            }
        }
        // Each of the ≥2 tuple args rebuilds its own cell in the shared `call`; register every tuple's box ops.
        if let Some((_, _, rebuilds)) = &multi_args {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for rebuild in rebuilds {
                for f in &rebuild.fields {
                    f.collect_box_ops(&mut |bop| {
                        used.insert(bop);
                    });
                }
            }
        }
        // A SOLE sum arg (Option/Result) shared by all makes: the shared `call` rebuilds the sum cell via
        // `sum-new` (branching on disc), boxing each arm's payload with its box op.
        if let Some((_, _, rebuild)) = &sum_arg {
            used.insert("sum-new");
            for arm in [&rebuild.arm_true, &rebuild.arm_false] {
                arm.collect_ops(&mut |op| {
                    used.insert(op);
                });
            }
        }
        used.extend(lifted_ops.iter().copied());
    })?;
    if layout.lifted.is_empty() {
        return Err(Reject::decline(
            "a multi-export closure program produced no lifted lambda",
        ));
    }
    // APPEND the lifted closure bodies after the order defs (trailing funcs, env-prepended params).
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
    // The shared call's `call_indirect` functype: slot 0's lifted type index. All exports share the
    // signature, so every lifted lambda has the same functype shape → one shared type index suffices.
    let lifted_type_idx = layout.lifted_type_index(0, layout.import_base);

    // Build the serializer's make specs (resolve each export body's core func index post-build).
    let ser_makes: Vec<serialize::ClosureMake> = make_specs
        .iter()
        .map(|m| {
            let export_abs = layout
                .abs(m.def)
                .ok_or_else(|| Reject::decline("a closure export is not in the emission order"))?;
            Ok(serialize::ClosureMake {
                export_name: m.name.clone(),
                export_abs,
                param_vts: m.param_vts.clone(),
            })
        })
        .collect::<Result<_, Reject>>()?;
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    let abi_makes: Vec<envelope::ClosureMakeAbi> = make_specs
        .iter()
        .map(|m| envelope::ClosureMakeAbi {
            name: m.name.clone(),
            make_param_bytes: m.param_bytes.clone(),
        })
        .collect();
    // A fixed-shape tuple ARG (shared by all makes): the shared list-`call` cores rebuild each arg cell from
    // the flattened fields, the shared list<u8> envelope mints a flat `tuple<…>` (from `tuple_bytes`), a
    // recursive NESTED one (from `shape`), OR N tuples (from `list_slots`). `list_rebuilds` = every tuple's
    // rebuild in arg order; `list_slots` is `Some` only for ≥2 tuple args (the single-tuple cases keep the
    // byte-identical `tuple_bytes`/`tuple_shape` mint). `None`/empty on the scalar-arg path.
    let list_rebuilds: Vec<serialize::TupleArgRebuild> = if let Some((_, _, rebuilds)) = &multi_args
    {
        rebuilds.clone()
    } else if let Some((_, _, _, _, rb)) = &tuple_arg {
        vec![rb.clone()]
    } else if let Some((_, _, rb, _, _, _)) = &nested_tuple {
        vec![rb.clone()]
    } else {
        Vec::new()
    };
    let list_slots: Option<&[crate::backend::wasm::envelope::ArgSlot]> =
        multi_args.as_ref().map(|(slots, _, _)| slots.as_slice());
    let tuple_bytes = tuple_arg.as_ref().map(|(fb, _, _, _, _)| fb.as_slice());
    let tuple_shape: Option<&[crate::backend::wasm::envelope::TupleFieldShape]> = nested_tuple
        .as_ref()
        .map(|(_, _, _, shape, _, _)| shape.as_slice());
    // Prefix/suffix scalar bytes when the tuple sits among scalars; empty for a sole tuple (flat OR nested).
    // Both the SCALAR and the three LIST-result multi cores now interleave these around the rebuilt tuple (via
    // the shared `serialize::emit_closure_call_args`); the shared `call` functype interleaves the scalar
    // boundary bytes around the `tuple<…>` type.
    let tpre = tuple_arg
        .as_ref()
        .map(|(_, _, pre, _, _)| pre.as_slice())
        .or_else(|| {
            nested_tuple
                .as_ref()
                .map(|(_, _, _, _, pre, _)| pre.as_slice())
        })
        .unwrap_or(&[]);
    let tsuf = tuple_arg
        .as_ref()
        .map(|(_, _, _, suf, _)| suf.as_slice())
        .or_else(|| {
            nested_tuple
                .as_ref()
                .map(|(_, _, _, _, _, suf)| suf.as_slice())
        })
        .unwrap_or(&[]);
    // A SOLE sum arg with a LIST result on the multi-export path declines: the multi list-result cores/envelope
    // thread tuples (`list_rebuilds`/`list_slots`) but NOT sums, so a sum + list result would fall into them
    // with a mismatched `arg_vts`. Decline HERE so it doesn't reach the single-tuple-oriented list routings.
    if sum_arg.is_some() && (ret_is_bytes || ret_is_compound || ret_is_collection) {
        return Err(Reject::unsupported(
            "a multi-export closure taking an Option/Result arg AND returning a byte-rope/compound/collection \
             is not supported (the multi list-result path threads tuples, not sums; scalar-result works)",
        ));
    }
    // A COMPOUND shared result → the N-makes-one-list-`call` VALUE-FORM core (walks each closure's returned
    // handle into the value-form template) + the SAME memory/realloc envelope as the bytes path. cdz-run
    // try-decodes the `list<u8>` result to the typed `(: value T)` form.
    if let Some(template) = &ret_template {
        // C-HOST-6: the shared list-`call` takes `borrow<t>` (repeatable); the value-form walk is unaffected.
        let main_core = serialize::multi_closure_value_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &[],
            &arg_vts,
            lifted_type_idx,
            template,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(
            envelope::assemble_multi_closure_bytes_resource_borrow_tuple(
                &main_core,
                &dtor_core,
                &imports,
                &import_name,
                &abi_makes,
                &arg_bytes,
                &[],
                true,
                tuple_bytes,
                tpre,
                tsuf,
                tuple_shape,
                list_slots,
            ),
        );
    }
    // A VARIABLE-LENGTH collection shared result → the N-makes-one-list-`call` VALUE-ENCODE core (each `call`
    // dispatches, then value-encodes the returned collection handle) + the SAME memory/realloc envelope.
    if let Some(descriptor) = &ret_descriptor {
        // C-HOST-6: the shared list-`call` takes `borrow<t>` (repeatable); the value-encode is unaffected.
        let main_core = serialize::multi_closure_value_encode_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &[],
            &arg_vts,
            lifted_type_idx,
            descriptor,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(
            envelope::assemble_multi_closure_bytes_resource_borrow_tuple(
                &main_core,
                &dtor_core,
                &imports,
                &import_name,
                &abi_makes,
                &arg_bytes,
                &[],
                true,
                tuple_bytes,
                tpre,
                tsuf,
                tuple_shape,
                list_slots,
            ),
        );
    }
    // A byte-rope shared result → the N-makes-one-list-`call` bytes core + memory/realloc envelope. No plain
    // (non-closure) exports on the pure multi-export path.
    if ret_is_bytes {
        // C-HOST-6: the shared list-`call` takes `borrow<t>` (repeatable); the byte-rope copy is unaffected.
        let main_core = serialize::multi_closure_bytes_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &[],
            &arg_vts,
            lifted_type_idx,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(
            envelope::assemble_multi_closure_bytes_resource_borrow_tuple(
                &main_core,
                &dtor_core,
                &imports,
                &import_name,
                &abi_makes,
                &arg_bytes,
                &[],
                true,
                tuple_bytes,
                tpre,
                tsuf,
                tuple_shape,
                list_slots,
            ),
        );
    }
    // N-COMPOUND-ARGS (multi-export, SCALAR result): N same-sig closures share one `call` taking ≥2 fixed-shape
    // tuple/record args. The shared `call` receives every arg's FLATTENED fields (`arg_vts`) and rebuilds each
    // cell (one `TupleArgRebuild` per tuple); the envelope's shared `call` functype mints N `tuple<…>` types via
    // the `ArgSlot` slot model. (A list result over ≥2 tuples on multi-export is a follow-on — declines here.)
    if let Some((slots, _all_vts, rebuilds)) = &multi_args {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &[], // no plain (non-closure) exports on the pure multi-export path
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            rebuilds,
            &[], // no sum arg (this is a tuple/multi-tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes, // empty — the flattened fields are carried by the slot list
            result_byte,
            &[], // no plain exports
            false,
            None, // single-tuple flat path unused
            &[],  // single-tuple prefix unused
            &[],  // single-tuple suffix unused
            None, // single-tuple nested shape unused
            Some(slots),
        ));
    }
    // DIRECT-CALL SUM ARG (multi-export, SCALAR result): N same-sig closures share one `call` taking an
    // `(Option/Result scalar)`. The shared `call` rebuilds the sum cell (branch on disc → `sum-new`) from the
    // flattened `(disc, payload)`; the envelope's shared `call` functype takes the `option<…>`/`result<…>`
    // boundary type via the classifier's `ArgSlot`. `own<t>` (single-use); the rebuilt cell drop is unconditional.
    if let Some((slot, _payload_vt, rebuild)) = &sum_arg {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &[], // no plain (non-closure) exports on the pure multi-export path
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            &[], // no tuple arg
            std::slice::from_ref(rebuild),
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            result_byte,
            &[], // no plain exports
            false,
            None,
            &[],
            &[],
            None,
            Some(std::slice::from_ref(slot)),
        ));
    }
    // DIRECT-CALL COMPOUND ARG (multi-export): N same-sig closures share one `call` whose single argument is
    // a fixed-shape scalar tuple/record. The shared `call` receives the FLATTENED fields (`arg_vts`) and
    // rebuilds the cell (`TupleArgRebuild`); the envelope's shared `call` functype takes a `tuple<…>` type.
    // `own<t>` (single-use) this cut — the rebuilt-arg cell drop is unconditional, so still leak-free.
    if let Some((field_bytes, _all_vts, tpre2, tsuf2, rebuild)) = &tuple_arg {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &[], // no plain (non-closure) exports on the pure multi-export path
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            std::slice::from_ref(rebuild),
            &[], // no sum arg (single flat/nested tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes, // empty — the tuple arg is carried by `field_bytes`
            result_byte,
            &[], // no plain exports
            false,
            Some(field_bytes),
            tpre2, // prefix scalar bytes (empty for a sole-tuple arg)
            tsuf2, // suffix scalar bytes
            None,  // an all-scalar-field tuple — no nested shape
            None,  // single tuple → not the N-compound slot model
        ));
    }
    // DIRECT-CALL NESTED COMPOUND ARG (multi-export, SCALAR result): N same-sig closures share one `call`
    // whose sole arg is a NESTED fixed-shape compound. The shared `call` rebuilds the nested cell recursively;
    // the envelope mints the inner `tuple<…>` types by index (`tuple_shape`). (A nested arg with a list result
    // was handled by the list-result routings above.)
    if let Some((_leaf_bytes, _leaf_vts, rebuild, shape, npre, nsuf)) = &nested_tuple {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &[], // no plain exports on the pure multi-export path
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            std::slice::from_ref(rebuild),
            &[], // no sum arg (single flat/nested tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes, // empty — the flattened leaves are carried by the shape
            result_byte,
            &[], // no plain exports
            false,
            None, // the flat all-scalar path is unused; the shape drives the mint
            npre, // prefix/suffix scalar bytes (empty for a sole nested arg, non-empty among scalars)
            nsuf,
            Some(shape),
            None, // single (nested) tuple → not the N-compound slot model
        ));
    }
    // C-HOST-6: the ONE shared scalar `call` takes `borrow<t>`, so each make's handle is repeatable (the
    // host keeps it across calls; the `t-dtor` reclaims). Same borrow posture as the single-export scalar
    // `call` — the value-form multi paths above keep own/self-drop (a later widening).
    let main_core = serialize::multi_closure_resource_core_module_borrow(
        &funcs,
        &imports,
        &ser_makes,
        &[], // no plain (non-closure) exports on the pure multi-export path
        &arg_vts,
        ret_vt,
        lifted_type_idx,
        &layout,
        true,
    )
    .map_err(Reject::decline)?;
    Ok(envelope::assemble_multi_closure_resource_borrow(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_makes,
        &arg_bytes,
        result_byte,
        true,
    ))
}

/// Emit a MIXED multi-export component: one or more CLOSURE exports of the SAME signature (crossing via the
/// resource envelope's `make-<name>` + shared `call`) ALONGSIDE one or more PLAIN (non-closure) exports
/// (each an ORDINARY top-level component func). The closure interface instance and the plain funcs coexist
/// in one component — the `oracle_mixed_component` byte anchor proved it. This increment's scope: the
/// closure exports share ONE signature (distinct closure signatures alongside a plain export decline), and
/// each plain export has an ALIASED-SCALAR param/result shape (a compound/closure plain result declines —
/// its `list<u8>` boundary would need the memory/realloc lift shape, a later widening).
pub(super) fn emit_mixed_closure_resource(
    db: &mut Db,
    layout: &Layout,
    _spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    use crate::backend::wasm::lir::valtype_of;
    // Partition the exports: CLOSURE exports (result `Ty::Fn`) vs PLAIN exports (everything else).
    let closure_defs: Vec<usize> = layout
        .exports
        .iter()
        .filter(|e| matches!(e.result, crate::ty::Ty::Fn(_, _)))
        .map(|e| e.def)
        .collect();
    let plain_exports: Vec<&crate::layout::ExportPlan> = layout
        .exports
        .iter()
        .filter(|e| !matches!(e.result, crate::ty::Ty::Fn(_, _)))
        .collect();
    // The closure exports must all share ONE signature (the shared `call` functype). Distinct closure
    // signatures alongside a plain export is a further widening (the distinct-sig envelope has no plain slot).
    let first_sig = &layout
        .exports
        .iter()
        .find(|e| matches!(e.result, crate::ty::Ty::Fn(_, _)))
        .ok_or_else(|| Reject::decline("a mixed closure program has no closure export"))?
        .result;
    if !layout
        .exports
        .iter()
        .filter(|e| matches!(e.result, crate::ty::Ty::Fn(_, _)))
        .all(|e| &e.result == first_sig)
    {
        // DISTINCT closure signatures alongside a plain export: the distinct-sig envelope now carries plain
        // exports too (`assemble_distinct_sig_resource_mixed`), so route there — it groups the closures by
        // signature into G resource types and publishes the plain exports as top-level funcs.
        return emit_distinct_sig_resource(db, layout, _spans);
    }

    // Flatten the shared closure signature → arg types + result.
    let mut arg_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = first_sig.clone();
    while let crate::ty::Ty::Fn(dom, rng) = cur {
        arg_tys.push((*dom).clone());
        cur = *rng;
    }
    let ret_ty = cur;
    // Reject a closure escaping an effect (same rule as the other closure paths): scan the lifted bodies.
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
                     closure's handler context does not travel with it, so the effect would have no home \
                     when the host invokes it (closures escaping effects are not supported)",
                    h.effect, h.op
                ),
            ));
        }
    }
    // DIRECT-CALL COMPOUND ARG (mixed): a single fixed-shape scalar tuple/record arg shared by all closure
    // exports crosses as a native component `tuple<…>` (the shared `call` rebuilds the cell via
    // `TupleArgRebuild`); the plain exports ride alongside. Detected here so the scalar `arg_bytes` decline
    // below doesn't reject it. The tuple may be the SOLE arg OR sit among aliased-width scalars (prefix/
    // suffix). 5-tuple = (tuple field bytes, full flattened core vts, prefix scalar bytes, suffix scalar
    // bytes, rebuild).
    let tuple_arg: Option<CompoundArgBoundary> = if arg_tys.len() == 1 {
        fixed_shape_scalar_tuple_arg(&arg_tys[0])
            .map(|(fb, fv, rb)| (fb, fv, Vec::new(), Vec::new(), rb))
    } else {
        single_compound_among_scalars(arg_tys.as_slice())
    };
    // A fixed-shape compound arg with a NESTED compound field (shared by all closure exports; the plain
    // exports ride alongside) — SOLE or among scalars. Detected when the flat `tuple_arg` is None.
    let nested_tuple: Option<NestedCompoundArgBoundary> = if tuple_arg.is_some() {
        None
    } else {
        nested_sole_or_among_scalars(arg_tys.as_slice())
    };
    // N-COMPOUND-ARGS (≥2 fixed-shape tuple/record args) shared by the closure exports, plain exports
    // alongside. Scoped this increment: SCALAR shared-`call` result (a list result over ≥2 tuples declines).
    let multi_args: Option<(
        Vec<crate::backend::wasm::envelope::ArgSlot>,
        Vec<crate::backend::wasm::lir::ValType>,
        Vec<crate::backend::wasm::serialize::TupleArgRebuild>,
    )> = if tuple_arg.is_none() && nested_tuple.is_none() {
        multi_compound_args(arg_tys.as_slice())
    } else {
        None
    };
    // A SOLE `(Option/Result scalar)` arg shared by the closure exports, plain exports alongside. Scoped:
    // SCALAR shared-`call` result.
    let sum_arg: Option<(
        crate::backend::wasm::envelope::ArgSlot,
        Vec<crate::backend::wasm::lir::ValType>,
        crate::backend::wasm::serialize::SumArgRebuild,
    )> = if tuple_arg.is_none()
        && nested_tuple.is_none()
        && multi_args.is_none()
        && arg_tys.len() == 1
    {
        fixed_shape_option_scalar_arg(db, &arg_tys[0])
            .or_else(|| fixed_shape_result_compound_arg(db, &arg_tys[0]))
    } else {
        None
    };
    let arg_bytes: Vec<u8> = if tuple_arg.is_some()
        || nested_tuple.is_some()
        || multi_args.is_some()
        || sum_arg.is_some()
    {
        Vec::new() // the flattened fields are carried by tuple_arg/nested_tuple/multi_args/sum_arg, not arg_bytes
    } else {
        arg_tys
            .iter()
            .map(|t| {
                closure_boundary_byte(t)
                    .ok_or_else(|| closure_boundary_reject("argument", t, &db.name_ctx()))
            })
            .collect::<Result<_, _>>()?
    };
    // A byte-rope (`Bytes`/`String`) shared closure result crosses as `list<u8>` (the mixed bytes envelope);
    // a scalar result takes the by-value shared `call`. All closure exports share the signature.
    let ret_is_bytes = matches!(
        ret_ty.strip_nominal(),
        crate::ty::Ty::Bytes | crate::ty::Ty::String
    );
    // A COMPOUND (tuple/record/sum) shared result crosses as `list<u8>` carrying the value form — the shared
    // `call` walks each closure's returned handle into the ONE value-form template (all closure exports share
    // the result type). `None` for a byte-rope / scalar / no-template-compound.
    let ret_template = if ret_is_bytes || closure_boundary_byte(&ret_ty).is_some() {
        None
    } else {
        crate::lower::runtime_value_form_template(ret_ty.strip_nominal(), &db.name_ctx())
    };
    let ret_is_compound = ret_template.is_some();
    // A VARIABLE-LENGTH collection (List/Map/Set) shared result → the value-encode core (all closure exports
    // share the result type → the ONE shape descriptor); the plain exports ride alongside. `None` for
    // bytes/scalar/fixed-template.
    let ret_descriptor =
        if ret_is_bytes || ret_is_compound || closure_boundary_byte(&ret_ty).is_some() {
            None
        } else {
            // Any OTHER machine-representable result the runtime `value-encode` walker can render — a
            // variable-length collection (`List`/`Map`/`Set`), a SUM (`Option`/`Result`/a user sum), or a
            // compound (tuple/record) CONTAINING a variable-length element — escapes as `list<u8>` via a
            // compiler-baked shape DESCRIPTOR. `sum_shape_descriptor` returns `None` for a scalar (handled
            // above) or an unrenderable shape, so this is a safe general fallback beyond the fixed
            // List/Map/Set set. (A fixed-shape compound already took the cheaper static `ret_template` path.)
            crate::lower::sum_shape_descriptor(db, ret_ty.strip_nominal())
        };
    let ret_is_collection = ret_descriptor.is_some();
    // A fixed-shape compound ARG now composes with EVERY mixed result shape too — scalar, byte-rope,
    // fixed-compound (value-form), collection (value-encode): the shared multi list-result cores + the shared
    // multi list<u8> tuple envelope thread the `TupleArgRebuild`, and the plain (non-closure) exports ride
    // alongside unaffected. No result-shape decline remains for a mixed tuple arg. (A compound-arg-alongside-
    // others / variable-length-field compound arg still declines at detection.)
    let result_byte = if ret_is_bytes || ret_is_compound || ret_is_collection {
        0 // unused by the list-returning paths; `call` returns list<u8>
    } else {
        closure_boundary_byte(&ret_ty)
            .ok_or_else(|| closure_boundary_reject("result", &ret_ty, &db.name_ctx()))?
    };
    // Core call-arg valtypes: the FULL flattened core param list when a (flat or nested) tuple arg, else each
    // arg's own valtype.
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = if let Some((_, all_vts, _, _, _)) =
        &tuple_arg
    {
        all_vts.clone()
    } else if let Some((_, leaf_vts, _, _, _, _)) = &nested_tuple {
        leaf_vts.clone() // the DEPTH-FIRST flattened leaf params of a nested tuple arg
    } else if let Some((_, all_vts, _)) = &multi_args {
        all_vts.clone() // the flattened leaves of EVERY tuple/scalar arg, in order (N-compound-args)
    } else if let Some((_, payload_vts, _)) = &sum_arg {
        // sum flattens to (disc: i32, <payload leaves…>) — scalar payload = 1 leaf, compound = its leaves.
        let mut vts = vec![crate::backend::wasm::lir::ValType::I32];
        vts.extend(payload_vts.iter().copied());
        vts
    } else {
        arg_tys
            .iter()
            .map(|t| {
                valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype"))
            })
            .collect::<Result<_, _>>()?
    };
    let ret_vt = valtype_of(&ret_ty)
        .ok_or_else(|| Reject::decline("closure result has no machine valtype"))?;

    // Per closure export: its params (each `make` forwards them) as core valtypes + boundary bytes.
    struct MakeSpec {
        def: usize,
        name: String,
        param_vts: Vec<crate::backend::wasm::lir::ValType>,
        param_bytes: Vec<u8>,
    }
    let mut make_specs: Vec<MakeSpec> = Vec::new();
    for &def in &closure_defs {
        let export = layout
            .exports
            .iter()
            .find(|e| e.def == def)
            .ok_or_else(|| Reject::decline("a closure export is not in the layout"))?;
        let param_vts: Vec<_> = export
            .params
            .iter()
            .map(|(_, t)| {
                valtype_of(t)
                    .ok_or_else(|| Reject::decline("closure export param has no machine valtype"))
            })
            .collect::<Result<_, _>>()?;
        let param_bytes: Vec<u8> = export
            .params
            .iter()
            .map(|(_, t)| {
                closure_boundary_byte(t)
                    .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
            })
            .collect::<Result<_, _>>()?;
        make_specs.push(MakeSpec {
            def,
            name: format!("make-{}", export.name),
            param_vts,
            param_bytes,
        });
    }

    // Per PLAIN export: its source name (both the core export name and — kebab-normalized — the public
    // boundary name), its param bytes, and its scalar result byte. A NULLARY export gives `()` params; a
    // compound/closure result has no `closure_boundary_byte` → declines (a later widening).
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

    // Lifted-body ops (a capturing closure's env reads appear only in the lifted bodies).
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
    let (imports, mut funcs, layout) = resource_escape_build(db, layout, |used| {
        used.insert("arr-get");
        used.insert("get-int");
        used.insert("drop");
        if ret_is_bytes {
            used.insert("bytes-len");
            used.insert("bytes-get");
        }
        // A COMPOUND-result shared `call` walks the returned handle to fill the value form — a Bool leaf
        // reads `get-bool` (int + nested `arr-get` already covered).
        if ret_is_compound {
            used.insert("get-bool");
        }
        // A collection-result shared `call` renders via `value-encode(rep, desc)`.
        if ret_is_collection {
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
        // A DIRECT-CALL fixed-shape scalar tuple/record ARG rebuilds its cell in the `call` body
        // (`emit_tuple_rebuild`): register the ops it emits — `arr-alloc`/`arr-set` + each field's box op
        // (`box-int`/`box-bool`/`box-float`/`box-float32`), which appear only in the synthesized rebuild.
        // Without the box op a Bool/Float field panicked ("rebuild op imported"); see `emit_closure_resource`.
        if let Some((_, _, _, _, rebuild)) = &tuple_arg {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for f in &rebuild.fields {
                f.collect_box_ops(&mut |bop| {
                    used.insert(bop);
                });
            }
        }
        if let Some((_, _, rebuild, _, _, _)) = &nested_tuple {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for f in &rebuild.fields {
                f.collect_box_ops(&mut |bop| {
                    used.insert(bop);
                });
            }
        }
        // ≥2 tuple args (N-compound, mixed): each rebuilds its own cell — register every tuple's box ops.
        if let Some((_, _, rebuilds)) = &multi_args {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for rebuild in rebuilds {
                for f in &rebuild.fields {
                    f.collect_box_ops(&mut |bop| {
                        used.insert(bop);
                    });
                }
            }
        }
        // A SOLE sum arg (Option/Result) shared by the closure exports: the shared `call` rebuilds the sum cell
        // via `sum-new`, boxing each arm's payload.
        if let Some((_, _, rebuild)) = &sum_arg {
            used.insert("sum-new");
            for arm in [&rebuild.arm_true, &rebuild.arm_false] {
                arm.collect_ops(&mut |op| {
                    used.insert(op);
                });
            }
        }
        used.extend(lifted_ops.iter().copied());
    })?;
    if layout.lifted.is_empty() {
        return Err(Reject::decline(
            "a mixed closure program produced no lifted lambda",
        ));
    }
    // APPEND the lifted closure bodies after the order defs (trailing funcs, env-prepended params).
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

    // Build the serializer's make specs + plain specs (resolve each body's core func index post-build).
    let ser_makes: Vec<serialize::ClosureMake> = make_specs
        .iter()
        .map(|m| {
            let export_abs = layout
                .abs(m.def)
                .ok_or_else(|| Reject::decline("a closure export is not in the emission order"))?;
            Ok(serialize::ClosureMake {
                export_name: m.name.clone(),
                export_abs,
                param_vts: m.param_vts.clone(),
            })
        })
        .collect::<Result<_, Reject>>()?;
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
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    let abi_makes: Vec<envelope::ClosureMakeAbi> = make_specs
        .iter()
        .map(|m| envelope::ClosureMakeAbi {
            name: m.name.clone(),
            make_param_bytes: m.param_bytes.clone(),
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
    // A fixed-shape tuple ARG (shared by all closure makes): the shared list-`call` cores rebuild the arg cell
    // from the flattened fields (interleaving prefix/suffix scalars via `emit_closure_call_args`), the shared
    // list<u8> tuple envelope emits the `tuple<…>` type; plain exports ride alongside. `None` on the scalar-arg
    // path. Prefix/suffix scalar bytes are empty for a sole tuple, non-empty when it sits among scalars.
    // Flat OR nested: a flat arg carries `tuple_bytes = Some` + `tuple_shape = None`; a nested one the reverse
    // (its shape is the SOLE arg → no prefix/suffix). `rebuild` falls back from flat → nested.
    // `list_rebuilds` = every tuple's rebuild in arg order (single-tuple / nested / ≥2 tuples); `list_slots`
    // is `Some` only for ≥2 tuple args (the single-tuple cases keep the byte-identical `tuple_bytes`/
    // `tuple_shape` mint). `None`/empty on the scalar-arg path.
    let list_rebuilds: Vec<serialize::TupleArgRebuild> = if let Some((_, _, rebuilds)) = &multi_args
    {
        rebuilds.clone()
    } else if let Some((_, _, _, _, rb)) = &tuple_arg {
        vec![rb.clone()]
    } else if let Some((_, _, rb, _, _, _)) = &nested_tuple {
        vec![rb.clone()]
    } else {
        Vec::new()
    };
    let list_slots: Option<&[crate::backend::wasm::envelope::ArgSlot]> =
        multi_args.as_ref().map(|(slots, _, _)| slots.as_slice());
    let tuple_bytes = tuple_arg.as_ref().map(|(fb, _, _, _, _)| fb.as_slice());
    let tuple_shape: Option<&[crate::backend::wasm::envelope::TupleFieldShape]> = nested_tuple
        .as_ref()
        .map(|(_, _, _, shape, _, _)| shape.as_slice());
    let tpre = tuple_arg
        .as_ref()
        .map(|(_, _, pre, _, _)| pre.as_slice())
        .or_else(|| {
            nested_tuple
                .as_ref()
                .map(|(_, _, _, _, pre, _)| pre.as_slice())
        })
        .unwrap_or(&[]);
    let tsuf = tuple_arg
        .as_ref()
        .map(|(_, _, _, suf, _)| suf.as_slice())
        .or_else(|| {
            nested_tuple
                .as_ref()
                .map(|(_, _, _, _, _, suf)| suf.as_slice())
        })
        .unwrap_or(&[]);
    // A SOLE sum arg with a LIST result on the mixed path declines (the mixed list-result cores thread tuples,
    // not sums) — decline HERE so it doesn't reach the single-tuple-oriented list routings below.
    if sum_arg.is_some() && (ret_is_bytes || ret_is_compound || ret_is_collection) {
        return Err(Reject::decline(
            "a mixed closure taking an Option/Result arg AND returning a byte-rope/compound/collection is not \
             supported on the mixed list-result path (which threads tuples, not sums; a scalar result is supported)",
        ));
    }
    // A COMPOUND shared closure result → the VALUE-FORM mixed core (N makes + shared list-`call` walking each
    // closure's returned handle into the value-form template + the plain exports as top-level funcs), same
    // `list<u8>` envelope as the bytes path. cdz-run try-decodes the result to the typed `(: value T)` form.
    if let Some(template) = &ret_template {
        // C-HOST-6: the shared list-`call` takes `borrow<t>` (repeatable); plain exports unaffected.
        let main_core = serialize::multi_closure_value_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &ser_plain,
            &arg_vts,
            lifted_type_idx,
            template,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(
            envelope::assemble_multi_closure_bytes_resource_borrow_tuple(
                &main_core,
                &dtor_core,
                &imports,
                &import_name,
                &abi_makes,
                &arg_bytes,
                &abi_plain,
                true,
                tuple_bytes,
                tpre,
                tsuf,
                tuple_shape,
                list_slots,
            ),
        );
    }
    // A VARIABLE-LENGTH collection shared closure result → the mixed VALUE-ENCODE core (N makes + shared
    // value-encode `call` + the plain exports as top-level funcs), same `list<u8>` envelope.
    if let Some(descriptor) = &ret_descriptor {
        // C-HOST-6: the shared list-`call` takes `borrow<t>` (repeatable); plain exports unaffected.
        let main_core = serialize::multi_closure_value_encode_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &ser_plain,
            &arg_vts,
            lifted_type_idx,
            descriptor,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(
            envelope::assemble_multi_closure_bytes_resource_borrow_tuple(
                &main_core,
                &dtor_core,
                &imports,
                &import_name,
                &abi_makes,
                &arg_bytes,
                &abi_plain,
                true,
                tuple_bytes,
                tpre,
                tsuf,
                tuple_shape,
                list_slots,
            ),
        );
    }
    // A byte-rope shared closure result → the mixed BYTES envelope (N makes + shared list-`call` + the plain
    // exports as top-level funcs). A scalar result takes the by-value mixed envelope.
    if ret_is_bytes {
        // C-HOST-6: the shared list-`call` takes `borrow<t>` (repeatable); plain exports unaffected.
        let main_core = serialize::multi_closure_bytes_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &ser_plain,
            &arg_vts,
            lifted_type_idx,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(
            envelope::assemble_multi_closure_bytes_resource_borrow_tuple(
                &main_core,
                &dtor_core,
                &imports,
                &import_name,
                &abi_makes,
                &arg_bytes,
                &abi_plain,
                true,
                tuple_bytes,
                tpre,
                tsuf,
                tuple_shape,
                list_slots,
            ),
        );
    }
    // N-COMPOUND-ARGS (mixed, SCALAR result): the shared `call` takes ≥2 fixed-shape tuple/record args, plain
    // exports alongside. The shared `call` rebuilds each cell (one `TupleArgRebuild` per tuple); the envelope's
    // shared `call` functype mints N `tuple<…>` types via the `ArgSlot` slot model.
    if let Some((slots, _all_vts, rebuilds)) = &multi_args {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &ser_plain,
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            rebuilds,
            &[], // no sum arg (this is a tuple/multi-tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes, // empty — the flattened fields are carried by the slot list
            result_byte,
            &abi_plain,
            false,
            None, // single-tuple flat path unused
            &[],  // single-tuple prefix unused
            &[],  // single-tuple suffix unused
            None, // single-tuple nested shape unused
            Some(slots),
        ));
    }
    // DIRECT-CALL SUM ARG (mixed, SCALAR result): the shared `call` takes an `(Option/Result scalar)`, plain
    // exports alongside. The shared `call` rebuilds the sum cell (branch on disc → `sum-new`); the envelope's
    // shared `call` functype takes the `option<…>`/`result<…>` boundary type via the classifier's `ArgSlot`.
    if let Some((slot, _payload_vt, rebuild)) = &sum_arg {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &ser_plain,
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            &[], // no tuple arg
            std::slice::from_ref(rebuild),
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            result_byte,
            &abi_plain,
            false,
            None,
            &[],
            &[],
            None,
            Some(std::slice::from_ref(slot)),
        ));
    }
    // DIRECT-CALL COMPOUND ARG (mixed): the shared `call`'s single arg is a fixed-shape scalar tuple/record.
    // The shared `call` receives the FLATTENED fields (`arg_vts`) + rebuilds the cell (`TupleArgRebuild`); the
    // envelope's shared `call` functype takes a `tuple<…>` type, and the plain exports ride alongside as
    // top-level funcs. `own<t>` (single-use) this cut — the rebuilt-arg cell drop is unconditional.
    if let Some((field_bytes, _all_vts, tpre2, tsuf2, rebuild)) = &tuple_arg {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &ser_plain,
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            std::slice::from_ref(rebuild),
            &[], // no sum arg (single flat/nested tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes, // empty — the tuple arg is carried by `field_bytes`
            result_byte,
            &abi_plain,
            false,
            Some(field_bytes),
            tpre2,
            tsuf2,
            None, // an all-scalar-field tuple — no nested shape
            None, // single tuple → not the N-compound slot model
        ));
    }
    // DIRECT-CALL NESTED COMPOUND ARG (mixed, SCALAR result): a SOLE nested fixed-shape compound arg shared by
    // the closure exports, with plain exports alongside. The shared `call` rebuilds the nested cell recursively;
    // the envelope mints the inner `tuple<…>` types by index (`tuple_shape`). (A nested arg with a list result
    // was handled by the list-result routings above.)
    if let Some((_leaf_bytes, _leaf_vts, rebuild, shape, npre, nsuf)) = &nested_tuple {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &ser_makes,
            &ser_plain,
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false,
            std::slice::from_ref(rebuild),
            &[], // no sum arg (single flat/nested tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_mixed_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes, // empty — the flattened leaves are carried by the shape
            result_byte,
            &abi_plain,
            false,
            None, // the flat all-scalar path is unused; the shape drives the mint
            npre, // prefix/suffix scalar bytes (empty for a sole nested arg, non-empty among scalars)
            nsuf,
            Some(shape),
            None, // single (nested) tuple → not the N-compound slot model
        ));
    }
    // C-HOST-6: the shared scalar `call` takes `borrow<t>` (repeatable — each make's handle survives across
    // calls; the `t-dtor` reclaims). The plain exports ride alongside unaffected.
    let main_core = serialize::multi_closure_resource_core_module_borrow(
        &funcs,
        &imports,
        &ser_makes,
        &ser_plain,
        &arg_vts,
        ret_vt,
        lifted_type_idx,
        &layout,
        true,
    )
    .map_err(Reject::decline)?;
    Ok(envelope::assemble_mixed_closure_resource_borrow(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_makes,
        &arg_bytes,
        result_byte,
        &abi_plain,
        true,
    ))
}
