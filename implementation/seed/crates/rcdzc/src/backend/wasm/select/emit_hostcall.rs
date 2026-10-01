//! Emit for the `Core::HostCall` node — the host/peer effect-op call lowering.
//!
//! Split out of `emit.rs` as a sibling module (same `use super::*` scope) to keep that file under
//! the per-file size mandate; this is the single largest `emit` match arm. The dispatch is a
//! one-line call from `emit`'s `Core::HostCall { .. }` arm into [`emit_host_call`], which
//! re-destructures the owned (`core_of` returns a cloned, memoized `Core`) node here. Behavior is
//! identical to the inlined arm — a peer-bound effect lowers to a `CallExternImport`, an unbound
//! effect stays a host call.
use super::*;

/// Emit a `Core::HostCall`: a peer-bound effect op → `CallExternImport`, else a host call.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_host_call(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let Core::HostCall {
        effect,
        op,
        args,
        result,
    } = core_of(db, id)
    else {
        unreachable!("emit_host_call invoked on a non-HostCall node");
    };
    // EFFECTS-UNIFICATION (U2): an escaping effect BOUND to a peer contract
    // (`db.effect_bindings`) is a PEER call — resolve it against the extern-import set and emit a
    // `CallExternImport`, exactly as a `Core::ExternCall` did. An unbound effect stays a host call.
    if let Some(iface) = db.effect_bindings.get(&*effect).cloned() {
        let index = layout.extern_index(&iface, &op).ok_or_else(|| {
                    // The peer op is not in `extern_order` — which the RESOURCE-ESCAPE emit paths
                    // (`emit_runtime_resource`/`emit_recursive_sum_resource`) do not populate: they carry
                    // the runtime import but not the peer extern envelope. So a peer-bound op reached in a
                    // body whose ENTRYPOINT RESULT escapes as a runtime resource (the entrypoint RETURNS
                    // the compound/Option a peer produced) has no import to call. This is a known gap (a
                    // resource×peer-extern envelope fusion is the fix); until then, the workaround is to
                    // consume the peer's value into a SCALAR the entrypoint returns (e.g. read the field/
                    // element and return it, or `List.len`) rather than returning the raw compound, OR
                    // handle the effect in-program instead of binding it to a peer.
                    Reject::unsupported(format!(
                        "a peer-bound effect op (`{op}` on `{iface}`) is reached in an entrypoint whose \
                         RESULT escapes as a runtime resource (it returns the compound/collection the peer \
                         produced) — the resource-escape boundary does not support carrying the peer import. \
                         Consume the peer's value into a scalar the entrypoint returns, or handle the \
                         effect in-program instead of binding it to a peer"
                    ))
                })?;
        for &arg in args.iter() {
            if matches!(crate::infer::type_of(db, arg), Ty::Unit) {
                continue;
            }
            emit(db, arg, slots, base, high, scratch_ty, layout, out)?;
        }
        out.push(Lir::CallExternImport(index));
        return Ok(());
    }
    // The core module lays imports peer-FIRST: peer ops `0..e`, then HOST ops `e..e+h`
    // (`core_module_impl`'s fixed order). So a host op's ABSOLUTE core-func index is its
    // `host_index` position SHIFTED past the `e` peer ops. `extern_order` is empty for every
    // non-fused path (host-only, or the host resource-escape whose leading block is host-only), so
    // the shift is `+0` there — byte-identical; it is non-zero only for a host+peer FUSION, where
    // it lands the `CallHostImport` in the host block instead of the peer block.
    let index = layout
        .host_index(&effect, &op)
        .map(|i| i + layout.extern_order.len())
        .ok_or_else(|| Reject::decline("a host call's operation is not in the host-import set"))?;
    // A runtime String/Bytes arg is marshalled into a scratch region of the shared `mem` (copy the
    // rope's logical bytes in, pass `(ptr,len)`). N such args in one call each need a DISJOINT region,
    // so a running CURSOR (a scratch i32 local) starts at the fixed scratch base and advances by each
    // marshalled arg's runtime length — the k-th runtime compound arg lands past the first k-1. The
    // cursor is reserved (and the per-arg `emit` floor raised past it) ONLY when a runtime compound arg
    // is actually present, so an all-scalar / const-string host call stays byte-identical to before.
    let has_runtime_compound = args.iter().any(|&a| {
                let at = crate::infer::type_of(db, a);
                (matches!(at, Ty::String | Ty::Bytes) && !matches!(core_of(db, a), Core::ConstStr(_)))
                    // A record arg with a `Bytes` FIELD (shape d2) also copies rope bytes into `mem`, so it
                    // needs the running scratch cursor reserved just like a Bytes arg.
                    || crate::backend::wasm::host::record_has_bytes_field(&at)
                    // A record arg with a `list<T>` FIELD marshals that list's backing into `mem` → cursor too.
                    || crate::backend::wasm::host::record_has_list_field(&at)
                    // A record arg with an `option<bytes>` FIELD copies the payload rope into `mem` → cursor.
                    || crate::backend::wasm::host::record_has_option_field_needing_mem(db, &at)
                    // A record arg with a `tuple<…>` FIELD may copy a Bytes element's rope → reserve the cursor.
                    || crate::backend::wasm::host::record_has_tuple_field(&at)
                    // A record arg with a `result<list<u8>, enum>` FIELD copies the Ok rope into `mem` on the Ok
                    // arm (`emit_record_arg_marshal`'s Result-field arm) → reserve the cursor (else that arm's
                    // `cursor.expect(...)` panics — SHAPE 215).
                    || crate::backend::wasm::host::record_has_result_field(db, &at)
                    // A record arg with a HETEROGENEOUS MIXED `variant` FIELD carrying a `Bytes`/`List` payload
                    // case rope-copies / marshals that case's payload into `mem` at the cursor
                    // (`emit_record_arg_marshal`'s VariantMemMixed arm → `emit_variant_mixed_arg_reg_flatten`'s
                    // Bytes/List arms) → reserve the cursor (else the field arm's `cursor.is_some()` guard would
                    // decline the whole record; the reservation is what lets the Bytes/List mixed-variant field cross).
                    || crate::backend::wasm::host::record_has_mem_mixed_variant_field(db, &at)
                    // A `list<T>` arg marshals into `mem` (its outer array + each element) → needs the cursor.
                    || matches!(at.strip_nominal(), Ty::List(_))
                    // A top-level `option<bytes>` arg copies the payload rope into `mem` on Some → needs the cursor.
                    || crate::backend::wasm::host::option_payload_ty(db, &at)
                        .is_some_and(|p| matches!(p, Ty::Bytes))
                    // A top-level `option<option<X>>` arg writes the INNER option's payload backing into `mem` on
                    // outer+inner Some iff `X` carries a `Bytes`/`list` leaf anywhere (`emit_option_reg_flatten`'s
                    // nested-option branch recurses into the inner option's mem-writing branch) → reserve the cursor
                    // iff the inner option's boundary abi needs `mem` (`record_field_abi_needs_memory`), in lockstep
                    // with the general nested-option admit gate. An all-scalar inner (option<option<tuple-of-scalars>>)
                    // needs no cursor.
                    || crate::backend::wasm::host::option_payload_ty(db, &at).is_some_and(|p| {
                        crate::backend::wasm::host::option_payload_ty(db, &p).is_some()
                            && crate::backend::wasm::host::field_boundary_abi(db, &p).is_some_and(
                                |abi| crate::backend::wasm::host::record_field_abi_needs_memory(&abi),
                            )
                    })
                    // A top-level `option<tuple>` arg copies a rope-bearing element (a `Bytes` element, or a
                    // record/list/mixed-variant-Bytes/List element ANYWHERE in the tuple tree) into `mem` on Some
                    // (`emit_option_reg_flatten`'s tuple branch → `emit_tuple_reg_flatten`) → needs the cursor.
                    // Uses `tuple_arg_needs_cursor` (the SAME broad recursion the direct `tuple<…>` arg + a
                    // `result<tuple, enum>` arg use, one line down), NOT the `Bytes`-only `tuple_has_bytes_element`
                    // — else an `option<tuple<mixed-variant-Bytes, …>>` would not reserve and the element arm's
                    // `cursor.is_some()` guard would decline the whole option.
                    || crate::backend::wasm::host::option_payload_ty(db, &at)
                        .is_some_and(|p| crate::backend::wasm::host::tuple_arg_needs_cursor(db, &p))
                    // A top-level `option<record>` arg whose payload record has a runtime-compound FIELD copies
                    // that field's bytes into `mem` on Some (`emit_option_reg_flatten`'s record branch →
                    // `emit_record_arg_marshal`) → needs the cursor, exactly like the direct `record` arg above.
                    // Cover every runtime-compound field the direct-record pre-scan reserves for (Bytes / list /
                    // option<bytes> / tuple), so an `option<record-with-list>` reserves the cursor its list-field
                    // marshal consumes (a missing reservation panics the marshal's cursor `expect`); an
                    // over-reservation is a harmless unused slot.
                    || crate::backend::wasm::host::option_payload_ty(db, &at).is_some_and(|p| {
                        crate::backend::wasm::host::record_has_bytes_field(&p)
                            || crate::backend::wasm::host::record_has_list_field(&p)
                            || crate::backend::wasm::host::record_has_option_field_needing_mem(db, &p)
                            || crate::backend::wasm::host::record_has_tuple_field(&p)
                            || crate::backend::wasm::host::record_has_result_field(db, &p)
                            // …or a HETEROGENEOUS MIXED `variant` FIELD with a `Bytes`/`List` payload case,
                            // which rope-copies / marshals into `mem` at the cursor when
                            // `emit_option_reg_flatten`'s record branch recurses `emit_record_arg_marshal`'s
                            // VariantMemMixed field arm (whose `cursor.is_some()` guard would else decline the
                            // whole option payload record) — the option<record> twin of the direct-record clause.
                            || crate::backend::wasm::host::record_has_mem_mixed_variant_field(db, &p)
                    })
                    // A top-level `option<list<T>>` arg marshals the payload list into `mem` on Some → cursor.
                    || crate::backend::wasm::host::option_payload_ty(db, &at)
                        .is_some_and(|p| matches!(p.strip_nominal(), Ty::List(_)))
                    // A top-level `option<mixed-variant-with-Bytes/List-case>` arg spills that case into `mem` at
                    // the cursor (`emit_option_reg_flatten`'s mixed-variant branch → `emit_variant_mixed_arg_reg_flatten`)
                    // → reserve it (else the rope copies to a bogus slot; `wasm-tools validate` misses that).
                    || crate::backend::wasm::host::option_mixed_variant_needs_cursor(db, &at)
                    // A top-level `result<list<u8>, enum>` arg copies the Ok `list<u8>` payload's rope into
                    // `mem` on the Ok arm (`emit_result_arg_reg_flatten`) → needs the running scratch cursor.
                    || crate::backend::wasm::host::result_bytes_enum(db, &at).is_some()
                    // A top-level `variant{…, bytes-case(s)}` arg copies a Bytes case's payload rope into `mem`
                    // (`emit_variant_bytes_arg_reg_flatten`) → needs the running scratch cursor.
                    || crate::backend::wasm::host::variant_bytes_payload_cases(db, &at).is_some()
                    // A top-level `variant{…, list<scalar>-case(s)}` arg marshals a list case's payload into `mem`
                    // (`emit_variant_list_arg_reg_flatten` → `emit_list_arg_marshal`) → needs the scratch cursor.
                    || crate::backend::wasm::host::variant_list_payload_cases(db, &at).is_some()
                    // A top-level MIXED `variant{…, bytes-case(s), …}` arg rope-copies a bytes case's payload into
                    // `mem` (`emit_variant_mixed_arg_reg_flatten`) → needs the scratch cursor.
                    || crate::backend::wasm::host::variant_mixed_payload_cases(db, &at).is_some()
                    // A top-level `result<list<scalar>, enum>` arg marshals the Ok payload list into `mem` on the
                    // Ok arm (`emit_result_list_arg_reg_flatten` → `emit_list_arg_marshal`) → needs the cursor too.
                    || crate::backend::wasm::host::result_list_enum(db, &at).is_some()
                    // A top-level `result<record, enum>` arg whose Ok record has a runtime-compound FIELD copies
                    // that field's bytes into `mem` on the Ok arm (`emit_result_record_arg_reg_flatten` →
                    // `emit_record_arg_marshal`) → needs the cursor, exactly like the direct `record` arg above.
                    || crate::backend::wasm::host::result_record_enum(db, &at).is_some_and(|(ok, _)| {
                        crate::backend::wasm::host::record_has_bytes_field(&ok)
                            || crate::backend::wasm::host::record_has_list_field(&ok)
                            || crate::backend::wasm::host::record_has_option_field_needing_mem(db, &ok)
                            || crate::backend::wasm::host::record_has_tuple_field(&ok)
                            || crate::backend::wasm::host::record_has_result_field(db, &ok)
                            // …or a mixed `variant` FIELD with a `Bytes`/`List` payload case (the result<record>
                            // twin of the option<record> clause above): `emit_result_record_arg_reg_flatten`'s
                            // Ok arm recurses `emit_record_arg_marshal`, whose VariantMemMixed field arm spills
                            // that case into `mem` at the cursor and else declines on `cursor.is_none()`.
                            || crate::backend::wasm::host::record_has_mem_mixed_variant_field(db, &ok)
                    })
                    // A top-level `result<tuple, enum>` arg whose Ok tuple has a runtime-compound ELEMENT
                    // (bytes/list/nested-compound) copies that element's bytes into `mem` on the Ok arm
                    // (`emit_result_tuple_arg_reg_flatten` → `emit_tuple_reg_flatten`) → needs the cursor, exactly
                    // like the direct `tuple<…>` arg above (a missing reservation panics the marshal's cursor).
                    || crate::backend::wasm::host::result_tuple_enum(db, &at)
                        .is_some_and(|(ok, _)| crate::backend::wasm::host::tuple_arg_needs_cursor(db, &ok))
                    // A top-level `tuple<…>` arg needs the cursor when SOME leaf (recursing nested tuples +
                    // record elements) copies runtime bytes into `mem` — a `Bytes` element, or a record
                    // element with a `Bytes` / `list` / `result` / `option<bytes>` field. Broader than the
                    // `Bytes`-only `tuple_has_bytes_element`, matching the record-element fields
                    // `tuple_arg_crosses` now admits (a missing reservation panics the marshal's cursor).
                    || crate::backend::wasm::host::tuple_arg_needs_cursor(db, &at)
            });
    let scratch_cursor_slot = if has_runtime_compound {
        let slot = base.max(*high);
        // The cursor OCCUPIES `slot`, so `*high` (the exclusive top of this call's declared scratch run,
        // which `select_function_of` turns into `declared = base..high`) must be `slot + 1` — else the
        // cursor slot is excluded from the declared locals and `coalesce_func`'s remap indexes out of
        // bounds (a `LocalSet(slot)` referencing an undeclared local → panic). A following arg's marshal
        // usually bumps `*high` past `slot` anyway, but a cursor-only reservation (no arg raises it
        // further) leaves it at `slot` without this `+ 1`.
        *high = (*high).max(slot + 1);
        scratch_ty.insert(slot, ValType::I32);
        // Seed the cursor at the fixed scratch base once, before any arg is marshalled.
        out.push(Lir::ConstI32(host_arg_scratch_base(layout) as i32));
        out.push(Lir::LocalSet(slot));
        Some(slot)
    } else {
        None
    };
    // Each arg is emitted above the scratch its predecessors consumed — `arg_base` rises to `*high`
    // after every arg (the same `arg_base = *high` threading the call/tail-call arg loops use —
    // `emit_call_args` and `emit_loop_iteration`). WITHOUT this, a runtime
    // String/Bytes marshal reserves i32 rope/len/pos slots at `base.max(*high)` and bumps `*high`, but
    // a FOLLOWING scalar arg emitted with the stale `base` would tee its i64 checked-arith guard into
    // that same slot — one wasm local declared at two widths → an invalid module (the marshalled-arg-
    // BEFORE-scalar order; the reverse worked only because the scalar bumped `*high` first).
    // When a scratch cursor is reserved, per-arg slots must start ABOVE it so no marshal/scalar arg
    // reuses (and re-types) the cursor slot, which must survive across the whole arg list.
    let mut arg_base = scratch_cursor_slot.map_or(base, |c| c + 1);
    // The op's declared param WIT types (declaration order) — a RECORD arg marshals its fields in the
    // host WIT record's field order, not the guest's name-lex order (else the component-linker's
    // structural match fails → silent no-instantiate). `arg_i` indexes it (args ↔ WIT params align).
    let wit_params = crate::backend::wasm::host::wit_op_param_types(db, &effect, &op);
    for (arg_i, &arg) in args.iter().enumerate() {
        let at = crate::infer::type_of(db, arg);
        match at {
            // A unit argument carries no boundary value.
            Ty::Unit => continue,
            // A STRING or BYTES argument crosses as `(ptr, len)` into the SHARED host memory
            // (`assemble_host_mem` provides it; `set_needs_memory` fires for any String/Bytes-param
            // op). A CONSTANT string's bytes were laid in the data segment at `host_string_offset` —
            // push that ptr + len. Everything else (a RUNTIME string rope OR any `Bytes` value, const
            // or runtime — a `Bytes.of`/slice-view byte-buffer) is MARSHALED here: copy its logical
            // bytes into a cursor-advanced scratch region of `mem` via the rep-agnostic
            // `bytes-len`/`bytes-get` walk (transparent through a rope OR a slice-view), then push
            // `(cursor, len)` — N runtime compound args per call each get a disjoint region. The
            // component boundary declares the Bytes param as `list<u8>` (a defined type-index) vs the
            // String's inline `string`; the CORE marshalling here is identical. adv-62b sibling: this
            // is the wasm side of the Bytes-host-arg reverse-parity gap (rust already crossed it).
            Ty::String | Ty::Bytes => match core_of(db, arg) {
                Core::ConstStr(s) => {
                    let offset = layout.host_string_offset(&s).ok_or_else(|| {
                        Reject::decline("a host-arg string was not laid in the data segment")
                    })?;
                    out.push(Lir::ConstI32(offset as i32));
                    out.push(Lir::ConstI32(s.len() as i32));
                }
                // RUNTIME string/Bytes arg → copy the rope into `mem` at the running cursor, push
                // `(cursor, len)`, then advance the cursor by `len` so a following runtime compound arg
                // lands in a DISJOINT region (N runtime compound args per call are supported; the cursor
                // starts at the fixed scratch base past the const-string data in the 1-page shared
                // `mem`). A host arg is consumed IMMEDIATELY by the call (not retained), so all the
                // marshalled args coexist in scratch only until the `CallHostImport`. The copy loop
                // mirrors `String.scalar-len`'s byte-scan (~7086) with `I32Store8` in place of the
                // counter. `bytes-len`/`bytes-get` are declared for this path in `collect_used_ops_into`
                // (else their `CallImport` resolves to u32::MAX).
                _ => {
                    let cursor = scratch_cursor_slot
                        .expect("a runtime compound arg reserves the scratch cursor (pre-scan)");
                    let rope_slot = arg_base.max(*high);
                    *high = (*high).max(rope_slot + 3);
                    scratch_ty.insert(rope_slot, ValType::I32);
                    let len_slot = rope_slot + 1;
                    scratch_ty.insert(len_slot, ValType::I32);
                    let pos_slot = rope_slot + 2;
                    scratch_ty.insert(pos_slot, ValType::I32);
                    emit(db, arg, slots, rope_slot + 3, high, scratch_ty, layout, out)?;
                    out.push(Lir::LocalSet(rope_slot));
                    out.push(Lir::LocalGet(rope_slot));
                    out.push(Lir::CallImport(OP_BYTES_LEN)); // [len:i32]
                    out.push(Lir::LocalSet(len_slot));
                    out.push(Lir::ConstI32(0));
                    out.push(Lir::LocalSet(pos_slot)); // pos = 0
                    // block { loop { br_out if pos>=len; mem[cursor+pos] = bytes-get(rope,pos);
                    //   pos++; br loop } }
                    out.push(Lir::Block(BlockType::Empty)); // $done
                    out.push(Lir::Loop(BlockType::Empty)); // $copy
                    out.push(Lir::LocalGet(pos_slot));
                    out.push(Lir::LocalGet(len_slot));
                    out.push(Lir::I32GeS);
                    out.push(Lir::BrIf(1)); // pos >= len → $done
                    out.push(Lir::LocalGet(cursor));
                    out.push(Lir::LocalGet(pos_slot));
                    out.push(Lir::I32Add); // [addr = cursor + pos]
                    out.push(Lir::LocalGet(rope_slot));
                    out.push(Lir::LocalGet(pos_slot));
                    out.push(Lir::CallImport(OP_BYTES_GET)); // [addr, byte:i32]
                    out.push(Lir::I32Store8 { offset: 0 }); // mem[cursor+pos] = byte
                    out.push(Lir::LocalGet(pos_slot));
                    out.push(Lir::ConstI32(1));
                    out.push(Lir::I32Add);
                    out.push(Lir::LocalSet(pos_slot)); // pos++
                    out.push(Lir::Br(0)); // → $copy
                    out.push(Lir::End); // end $copy
                    out.push(Lir::End); // end $done
                    // Push (ptr = cursor-before-advance, len), then advance cursor += len for the next
                    // runtime compound arg. The advance is stack-neutral (leaves the pushed (ptr,len)
                    // in place below it on the operand stack).
                    out.push(Lir::LocalGet(cursor));
                    out.push(Lir::LocalGet(len_slot)); // push (ptr, len)
                    out.push(Lir::LocalGet(cursor));
                    out.push(Lir::LocalGet(len_slot));
                    out.push(Lir::I32Add);
                    out.push(Lir::LocalSet(cursor)); // cursor += len
                    // MARSHALED-ARG RECLAIM (v-memory-safety, 04-capabilities host-arg leak): the
                    // runtime rope in `rope_slot` was COPIED byte-for-byte into shared `mem` above;
                    // the host reads `mem` (via `(ptr,len)`), NOT the guest handle, so the handle is
                    // DEAD after the copy loop. When the arg is a freshly-built OWNED producer (a
                    // `Bytes.of`/concat/slice rope — `heap_operand_ownership == Owned`), drop it here
                    // to reclaim it (else one buffer leaks per host call — the `(host (io) (io.op
                    // (Bytes.of …)))` shape). OWNED-BY-FLOW twin (04-capabilities:536, v-effects
                    // #048389-backstop refined so the `Some` shell now reclaims): a marshaled arg that
                    // is a CHILD-DUP site (`out.dup_sites.contains(&arg)` — a `SumPayload`/`Proj`
                    // extraction the shell-reclaim pass dup'd so the scrutinee's deep-drop cascade
                    // would not double-free it) likewise OWNS a reference in `rope_slot`. But the
                    // marshal only READS it (a byte-copy into `mem` — NOT a consuming op that would
                    // take the dup's ref, unlike `List.push`), so that child-dup is ORPHANED and
                    // leaks (the `(match (String.from-bytes …) ((Some s) (host (hs) (hs.h s))) …)`
                    // shape: `s` dup'd rc1->2, shell cascade nets 2->1, dup never freed). Drop it here
                    // — after the copy, before the shell deep-drop, so: dup (1->2), THIS drop (2->1),
                    // shell cascade (1->0), balanced. Stack-neutral: `(ptr,len)` were pushed above and
                    // stay below this `local.get; drop`. A plain BORROWED arg (a bare param whose
                    // owner reclaims it — NEITHER Owned NOR a dup site) is left untouched — leak-safe:
                    // never double-frees (leak-over-UAF). Import mirror in
                    // `collect_used_ops_into_seen`'s `HostCall` arm (same Owned || dup-site gate).
                    if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                        || out.dup_sites.contains(&arg)
                    {
                        out.push(Lir::LocalGet(rope_slot));
                        out.push(Lir::CallImport(OP_DROP));
                    }
                }
            },
            // A RECORD argument (shape d) crosses NATIVELY: the guest DECOMPOSES the value-heap record
            // field-by-field into the flattened core slots the component `record` param lowers to. The
            // fields are pushed in the host WIT record's DECLARATION order (`emit_record_arg_marshal`
            // reads the value-heap cell's name-lex position per WIT field), NOT the guest's name-lex
            // order — the component-linker requires the import's flattened args to match the host.
            //  • a SCALAR field reads back with `arr-get` + the field's wrap-free scalar get-op.
            //  • a BYTES field is copied rope→`mem` at the running cursor and pushed as `(ptr,len)`.
            // The reads BORROW the record (no consume), so the handle is not dropped here.
            // A record-of-bools arg whose imposed WIT param is `flags{…}` PACKS into the bitset word:
            // read each bool field (`arr-get`+`get-bool`, a borrow) and shift it into its label's bit
            // (`emit_flags_arg_pack`). A SEPARATE arm BEFORE the generic record arm (a flags arg has no
            // WIT record). The record is BORROWED (pure reads) — reclaim the OWNED handle after.
            Ty::Record(fields)
                if matches!(
                    wit_params.as_ref().and_then(|p| p.get(arg_i)),
                    Some(crate::wit_world::WitType::Flags(_))
                ) =>
            {
                let Some(crate::wit_world::WitType::Flags(labels)) =
                    wit_params.as_ref().and_then(|p| p.get(arg_i))
                else {
                    unreachable!("guarded by the arm")
                };
                let field_bits = crate::backend::wasm::host::flags_field_bits(&fields, labels)
                            .ok_or_else(|| {
                                Reject::decline(
                                    "a flags host-arg is not a matching record-of-bools (label/field mismatch \
                                     or >32 labels)",
                                )
                            })?;
                let rec_slot = arg_base.max(*high);
                scratch_ty.insert(rec_slot, ValType::I32);
                *high = (*high).max(rec_slot + 1);
                emit(db, arg, slots, rec_slot + 1, high, scratch_ty, layout, out)?; // [rec]
                out.push(Lir::LocalSet(rec_slot));
                let work_base = *high;
                emit_flags_arg_pack(rec_slot, &field_bits, work_base, high, scratch_ty, out);
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(rec_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            Ty::Record(fields) => {
                // The host WIT record type for THIS arg — required to order the fields (declaration
                // order); without it (world absent / arg not a WIT record) the marshal can't match the
                // host, so decline rather than emit a name-lex order that won't link.
                let Some(crate::wit_world::WitType::Record(_)) =
                    wit_params.as_ref().and_then(|p| p.get(arg_i))
                else {
                    return Err(Reject::decline(
                        "a record host-arg has no matching WIT record type in the target world (needed \
                                 to order its fields to the host's declaration order)",
                    ));
                };
                let wit = wit_params.as_ref().unwrap()[arg_i].clone();
                let rec_slot = arg_base.max(*high);
                scratch_ty.insert(rec_slot, ValType::I32);
                *high = (*high).max(rec_slot + 1);
                emit(db, arg, slots, rec_slot + 1, high, scratch_ty, layout, out)?; // [rec]
                out.push(Lir::LocalSet(rec_slot));
                let work_base = *high;
                emit_record_arg_marshal(
                    db,
                    rec_slot,
                    &fields,
                    &wit,
                    scratch_cursor_slot,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (v-memory-safety, compound analog of the String/Bytes host-arg
                // reclaim above): `emit_record_arg_marshal` READ each field (`arr-get` borrows) and
                // pushed the FLATTENED scalar/(ptr,len) leaves the component `record` param lowers to —
                // the host reads those, NOT the guest cell — so the record handle in `rec_slot` is DEAD
                // after the marshal. When the arg is a freshly-built OWNED record (a `#record(…)`
                // literal/builder — `heap_operand_ownership == Owned`) or a shell-reclaim CHILD-DUP site,
                // deep-drop it here to reclaim the shell + its owned fields: every field was COPIED to
                // the boundary (scalar get / rope→`mem`), NOT moved out as a live handle, so the cascade
                // is balanced (else one record cell leaks per host call — the `(host (io) (io.op
                // #record(…)))` shape, 28-wit cq04*). Stack-neutral: the flattened leaves stay below this
                // `local.get; drop`. A BORROWED record (a bare param whose owner reclaims it) is left
                // untouched — leak-over-UAF (never double-frees). Import mirror in
                // `collect_used_ops_into_seen`'s `HostCall` `Ty::Record` arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(rec_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A `list<T>` argument (`graph.set-edges`'s `targets: list<reducer-id>`) — the guest
            // MARSHALS the value-heap `List` into the shared `mem`: an outer array of `count` element
            // slots at the running cursor, each element canonical-encoded after it, then passes
            // `(outer-ptr, count)`. This slice marshals a `list<list<u8>>` (a `list<u8>` element =
            // `(ptr,len)`); a non-`list<u8>` element is a later increment (decline).
            Ty::List(elem) => {
                let elem = (*elem).clone();
                let cursor = scratch_cursor_slot
                    .expect("a list arg reserves the scratch cursor (has_runtime_compound)");
                // The element's declared WIT type (the list param's `WitType::List(elem)`) — threaded so
                // a RECORD element orders its fields to the host WIT declaration order. `None` for a
                // scalar/bytes element (offset-agnostic) or when no world declares this param.
                let elem_wit = match wit_params.as_ref().and_then(|p| p.get(arg_i)) {
                    Some(crate::wit_world::WitType::List(ew)) => Some(ew.as_ref()),
                    _ => None,
                };
                let list_slot = arg_base.max(*high);
                scratch_ty.insert(list_slot, ValType::I32);
                *high = (*high).max(list_slot + 1);
                emit(db, arg, slots, list_slot + 1, high, scratch_ty, layout, out)?; // [list]
                out.push(Lir::LocalSet(list_slot));
                let work_base = *high;
                emit_list_arg_marshal(
                    db, &elem, elem_wit, list_slot, cursor, work_base, high, scratch_ty, out,
                )?;
                // MARSHALED-ARG RECLAIM (v-memory-safety, list twin of the record/String/Bytes host-arg
                // reclaim): `emit_list_arg_marshal` walked the list (`vec-len`/`vec-get` borrow) and
                // COPIED each element into shared `mem` (scalar inline / rope→mem / nested record/tuple
                // via `emit_*_to_mem` — all pure-borrow, no `OP_DUP`, no element handle moved out), so
                // the list handle in `list_slot` is DEAD after the marshal. When the arg is a freshly-
                // built OWNED list (`heap_operand_ownership == Owned`) or a child-dup site, deep-drop it
                // — the cascade frees the spine + every element (all copied to the boundary, balanced;
                // else one list structure leaks per host call). A BORROWED list is left untouched
                // (leak-over-UAF). Import mirror in `collect_used_ops`'s `Ty::List` arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(list_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `tuple<scalar…>` argument: the guest emits the value-heap tuple HANDLE into a
            // slot, then decomposes it into the POSITIONALLY-flattened scalar core slots via
            // `emit_tuple_reg_flatten` (`arr-get i` + unbox per element, no disc). Checked before the
            // scalar `_` arm (a tuple's `emit` yields a HANDLE, not the flattened slots the built-in
            // `tuple` param expects); a compound-element tuple declines inside the flatten (lockstep with
            // the decline gate).
            Ty::Tuple(_) => {
                let tup_slot = arg_base.max(*high);
                scratch_ty.insert(tup_slot, ValType::I32);
                *high = (*high).max(tup_slot + 1);
                emit(db, arg, slots, tup_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(tup_slot));
                let work_base = *high;
                // The tuple's declared WIT type — a record element reorders its fields to WIT order.
                let tuple_wit = wit_params.as_ref().and_then(|p| p.get(arg_i));
                emit_tuple_reg_flatten(
                    db,
                    tup_slot,
                    &at,
                    tuple_wit,
                    scratch_cursor_slot,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (v-memory-safety, tuple twin of the record/option host-arg
                // reclaim): `emit_tuple_reg_flatten` read each element via borrowing `arr-get` + unbox —
                // pure-borrow, no dup, no handle moved out — so the tuple handle in `tup_slot` is DEAD
                // after the flatten. When the arg is a freshly-built OWNED tuple (`heap_operand_ownership
                // == Owned`) or a child-dup site, deep-drop it (each element was COPIED out as a scalar,
                // so the cascade is balanced; else the tuple shell leaks per host call). A BORROWED tuple
                // is left untouched (leak-over-UAF). Import mirror in `collect_used_ops`'s tuple-arg arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(tup_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `option<scalar>` argument: the guest emits the value-heap Option HANDLE into a
            // slot, then decomposes it into the canonical `(disc, payload)` register-flatten via
            // `emit_option_reg_flatten` (the register twin of the `option<scalar>` record-FIELD flatten),
            // mapping the guest some-disc to WIT `option` some=1 / none=0. Checked BEFORE the variant arm
            // (option is a Sum EXCLUDED from `variant_scalar_payload_cases`) and the scalar `_` arm (a
            // Sum's `emit` yields a HANDLE, not the flattened slots the built-in `option` param expects).
            _ if crate::backend::wasm::host::option_arg_crosses(db, &at) => {
                let opt_slot = arg_base.max(*high);
                scratch_ty.insert(opt_slot, ValType::I32);
                *high = (*high).max(opt_slot + 1);
                emit(db, arg, slots, opt_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(opt_slot));
                let work_base = *high;
                // The payload's declared WIT type — an `option<record>` marshal reorders the record's
                // name-lex fields to the host WIT declaration order, so thread the `option`'s inner WIT.
                let payload_wit = match wit_params.as_ref().and_then(|p| p.get(arg_i)) {
                    Some(crate::wit_world::WitType::Option(inner)) => Some(inner.as_ref()),
                    _ => None,
                };
                emit_option_reg_flatten(
                    db,
                    opt_slot,
                    &at,
                    payload_wit,
                    scratch_cursor_slot,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (v-memory-safety, option twin of the variant host-arg reclaim):
                // `emit_option_reg_flatten` read the disc + (on Some) unboxed the payload scalar via
                // borrowing `sum-disc`/`sum-payload` — pure-borrow, no dup, no handle moved out — so the
                // Option handle in `opt_slot` is DEAD after the flatten. When the arg is a freshly-built
                // OWNED option (`heap_operand_ownership == Owned`) or a child-dup site, deep-drop it (the
                // payload was COPIED out as a scalar, so the cascade is balanced; else the Option shell
                // leaks per host call). A BORROWED option is left untouched (leak-over-UAF). Import mirror
                // in `collect_used_ops`'s option-arg arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(opt_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A bare scalar-payload VARIANT argument (the top-level param position): the guest emits
            // the value-heap variant HANDLE into a slot, then decomposes it into the canonical
            // `(disc, payload)` register-flatten via the SAME `emit_variant_reg_flatten` the record-
            // field variant uses. Checked before the scalar `_` arm (a Sum's `emit` yields a HANDLE,
            // not the flattened slots the component `variant` param expects).
            _ if crate::backend::wasm::host::variant_scalar_payload_cases(db, &at).is_some() => {
                let var_slot = arg_base.max(*high);
                scratch_ty.insert(var_slot, ValType::I32);
                *high = (*high).max(var_slot + 1);
                emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(var_slot));
                let work_base = *high;
                emit_variant_reg_flatten(db, var_slot, &at, work_base, high, scratch_ty, out)?;
                // MARSHALED-ARG RECLAIM (v-memory-safety, variant twin of the record/list host-arg
                // reclaim): `emit_variant_reg_flatten` read the disc + (on a payload case) the payload
                // scalar via borrowing `sum-disc`/`sum-payload` + unbox — pure-borrow, no dup, no handle
                // moved out — so the variant handle in `var_slot` is DEAD after the flatten. When the arg
                // is a freshly-built OWNED variant (`heap_operand_ownership == Owned`) or a child-dup
                // site, deep-drop it (the payload was COPIED out as a scalar, so the cascade is balanced;
                // else the variant shell leaks per host call). A BORROWED variant is left untouched
                // (leak-over-UAF). Import mirror in `collect_used_ops`'s variant-arg arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(var_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A bare scalar-payload VARIANT argument whose payloads MIX int with float (or f32 with f64):
            // decomposed into `(disc, join)` via `emit_variant_mixed_scalar_arg_reg_flatten` (per-case
            // unbox + reinterpret-coerce into the join slot). Disjoint from the uniform scalar-variant arm
            // above (`variant_scalar_payload_cases` declined the mix); all-scalar → no cursor. Same
            // pure-borrow reclaim as the uniform variant (the flatten borrows the disc + payload).
            _ if crate::backend::wasm::host::variant_mixed_scalar_payload_cases(db, &at)
                .is_some() =>
            {
                let var_slot = arg_base.max(*high);
                scratch_ty.insert(var_slot, ValType::I32);
                *high = (*high).max(var_slot + 1);
                emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(var_slot));
                let work_base = *high;
                emit_variant_mixed_scalar_arg_reg_flatten(
                    db, var_slot, &at, work_base, high, scratch_ty, out,
                )?;
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(var_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `variant{nullary…, bytes-case(s)}` argument: the guest emits the value-heap
            // variant HANDLE into a slot, then decomposes it into the canonical `(disc, i32, i32)`
            // register-flatten via `emit_variant_bytes_arg_reg_flatten` (the arbitrary-disc twin of the
            // `result<list<u8>, enum>` flatten) — a Bytes case copies the payload rope into `mem` at the
            // cursor and yields `(disc, ptr, len)`, a nullary case yields `(disc, 0, 0)`. Checked BEFORE
            // the scalar `_` arm and disjoint from the scalar-variant arm above (that declined a Bytes
            // payload); `option`/`result` shapes took their own arms.
            _ if crate::backend::wasm::host::variant_bytes_payload_cases(db, &at).is_some() => {
                let bytes_discs = crate::backend::wasm::host::variant_bytes_payload_cases(db, &at)
                    .expect("gated by the arm guard");
                let var_slot = arg_base.max(*high);
                scratch_ty.insert(var_slot, ValType::I32);
                *high = (*high).max(var_slot + 1);
                emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(var_slot));
                let cursor = scratch_cursor_slot
                    .expect("a variant{…,bytes} arg reserves the scratch cursor (pre-scan)");
                let work_base = *high;
                emit_variant_bytes_arg_reg_flatten(
                    var_slot,
                    &bytes_discs,
                    cursor,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (v-memory-safety, variant-bytes twin of the result host-arg
                // reclaim): `emit_variant_bytes_arg_reg_flatten` read the disc + (on a Bytes case) copied
                // the payload rope via borrowing `sum-disc`/`sum-payload`/`bytes-get` — pure borrow, no
                // dup, no handle moved out — so the variant handle in `var_slot` is DEAD after the flatten.
                // When the arg is a freshly-built OWNED variant (`heap_operand_ownership == Owned`) or a
                // child-dup site, deep-drop it (the payload was COPIED out, so the cascade is balanced;
                // else the variant shell leaks per host call). A BORROWED variant is left untouched
                // (leak-over-UAF). Import mirror in `collect_used_ops`'s variant-bytes-arg arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(var_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `variant{nullary…, list<scalar>-case(s)}` argument: the guest emits the
            // value-heap variant HANDLE into a slot, then decomposes it into the canonical `(disc, i32,
            // i32)` register-flatten via `emit_variant_list_arg_reg_flatten` (the `list` sibling of the
            // bytes-case variant) — a list case marshals the payload list into `mem` at the cursor and
            // yields `(disc, ptr, count)`, a nullary case yields `(disc, 0, 0)`. Checked BEFORE the scalar
            // `_` arm and disjoint from the scalar/bytes variant arms above.
            _ if crate::backend::wasm::host::variant_list_payload_cases(db, &at).is_some() => {
                let (list_discs, elem) =
                    crate::backend::wasm::host::variant_list_payload_cases(db, &at)
                        .expect("gated by the arm guard");
                let var_slot = arg_base.max(*high);
                scratch_ty.insert(var_slot, ValType::I32);
                *high = (*high).max(var_slot + 1);
                emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(var_slot));
                let cursor = scratch_cursor_slot
                    .expect("a variant{…,list} arg reserves the scratch cursor (pre-scan)");
                let work_base = *high;
                emit_variant_list_arg_reg_flatten(
                    db,
                    var_slot,
                    &list_discs,
                    &elem,
                    cursor,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (variant-list twin of the variant-bytes reclaim):
                // `emit_variant_list_arg_reg_flatten` read the disc + (on a list case) marshalled the
                // payload list via borrowing `sum-disc`/`sum-payload`/`vec-*` — pure borrow, no dup, no
                // handle moved out — so the variant handle in `var_slot` is DEAD after the flatten.
                // Deep-drop it when the arg is a freshly-built OWNED variant or a dup-site; a BORROWED
                // variant is left untouched. Import mirror in `collect_used_ops`'s variant-list-arg arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(var_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `variant{nullary…, one tuple-of-scalars case}` argument: the guest emits the
            // value-heap variant HANDLE into a slot, then decomposes it into the canonical `(disc, e0,
            // e1, …)` positional register-flatten via `emit_variant_tuple_arg_reg_flatten` (the product
            // sibling; all-scalar → NO `mem`/cursor). A nullary case zero-fills the payload slots. Checked
            // BEFORE the scalar `_` arm and disjoint from the scalar/bytes/list variant arms above.
            _ if crate::backend::wasm::host::variant_tuple_payload_case(db, &at).is_some() => {
                let (tuple_disc, _abis) =
                    crate::backend::wasm::host::variant_tuple_payload_case(db, &at)
                        .expect("gated by the arm guard");
                let tuple_ty =
                    crate::backend::wasm::select::variant_payload_ty_at(db, &at, tuple_disc as u32)
                        .expect("the tuple case's payload type resolves (detector gated)");
                let var_slot = arg_base.max(*high);
                scratch_ty.insert(var_slot, ValType::I32);
                *high = (*high).max(var_slot + 1);
                emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(var_slot));
                let work_base = *high;
                emit_variant_tuple_arg_reg_flatten(
                    db, var_slot, tuple_disc, &tuple_ty, work_base, high, scratch_ty, out,
                )?;
                // MARSHALED-ARG RECLAIM (variant-tuple twin): the flatten read the disc + (on the tuple
                // case) unboxed the elements via borrowing `sum-disc`/`sum-payload`/`arr-get`/unbox — pure
                // borrow — so the variant handle is DEAD after. Deep-drop when OWNED or a dup-site.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(var_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `variant{nullary…, one record-of-scalars case}` argument: the record sibling of
            // the tuple-payload variant. Decomposes into `(disc, f0, f1, …)` via
            // `emit_variant_record_arg_reg_flatten` (recurses `emit_record_arg_marshal`, WIT-order fields;
            // all-scalar → NO `mem`/cursor); a nullary case zero-fills. The record case's WIT `record` type
            // (for field ordering) is extracted from the arg's `WitType::Variant` at the record disc.
            _ if crate::backend::wasm::host::variant_record_payload_case(db, &at).is_some() => {
                let (record_disc, record_ty) =
                    crate::backend::wasm::host::variant_record_payload_case(db, &at)
                        .expect("gated by the arm guard");
                let record_wit = match wit_params.as_ref().and_then(|p| p.get(arg_i)) {
                    Some(crate::wit_world::WitType::Variant(cases)) => {
                        cases.get(record_disc as usize).and_then(|(_, p)| p.clone())
                    }
                    _ => None,
                };
                let Some(record_wit) = record_wit else {
                    return Err(Reject::decline(
                        "a variant record-payload arg has no WIT record case type",
                    ));
                };
                let var_slot = arg_base.max(*high);
                scratch_ty.insert(var_slot, ValType::I32);
                *high = (*high).max(var_slot + 1);
                emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(var_slot));
                let work_base = *high;
                emit_variant_record_arg_reg_flatten(
                    db,
                    var_slot,
                    record_disc,
                    &record_ty,
                    &record_wit,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (variant-record twin): pure-borrow flatten → the variant handle is
                // DEAD after. Deep-drop when OWNED or a dup-site.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(var_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level MIXED `variant{…, scalar-case(s), bytes-case(s)}` argument: the guest emits the
            // value-heap variant HANDLE into a slot, then decomposes it into the canonical variant JOIN
            // flatten via `emit_variant_mixed_arg_reg_flatten` (per-case dispatch: scalar → unbox, bytes →
            // rope-copy at the cursor). Checked AFTER the uniform variant arms and BEFORE the scalar `_`.
            _ if crate::backend::wasm::host::variant_mixed_payload_cases(db, &at).is_some() => {
                // Use the WIT-AWARE cases so a RECORD payload case's field slots follow the WIT record's
                // declaration order (the bare detector orders them name-lex). Consistent with `serialize`
                // (which flattens the classifier's WIT-ordered `HostParam` cases) + `host_imports` (the WIT
                // `variant` type). A record case with no resolvable WIT field order → a CODED CDZ0903
                // decline: this fires for a sum whose WIT is NOT a `WitType::Variant` (e.g. a
                // `result<bytes, record>` — a 2-variant sum that reaches this arm, but its WIT is
                // `WitType::Result`, so the Err-record case cannot be WIT-ordered here) or a WIT field
                // absent from the guest record. Coded (not a bare `Reject::decline`) so the boundary
                // classifier band stays coded — a compound-`Err` result arg is a queued crossing (it needs
                // the Ok/Err-disc-aware WIT ordering + byte-exact verification via the forthcoming
                // `host-arg-received` harness), not a silent codeless refusal.
                let wit_variant = wit_params.as_ref().and_then(|p| p.get(arg_i));
                let cases = crate::backend::wasm::host::variant_mixed_payload_cases_wit(
                            db,
                            &at,
                            wit_variant,
                        )
                        .ok_or_else(|| {
                            Reject::coded(
                                crate::diag::Code::HostOpNoBoundaryForm,
                                "a mixed variant host-op argument with a record payload case has no resolvable \
                                 WIT field order (its declared WIT is not a `variant`, e.g. a `result` with a \
                                 record error case, or a WIT field is absent from the guest record)",
                            )
                        })?;
                let var_slot = arg_base.max(*high);
                scratch_ty.insert(var_slot, ValType::I32);
                *high = (*high).max(var_slot + 1);
                emit(db, arg, slots, var_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(var_slot));
                let cursor = scratch_cursor_slot
                    .expect("a mixed variant arg reserves the scratch cursor (pre-scan)");
                let work_base = *high;
                emit_variant_mixed_arg_reg_flatten(
                    db,
                    var_slot,
                    &at,
                    &cases,
                    wit_variant,
                    cursor,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (mixed-variant twin): pure-borrow flatten (sum-disc/payload +
                // unbox/bytes-get) → the variant handle is DEAD after. Deep-drop when OWNED or a dup-site.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(var_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `result<list<u8>, enum>` argument: the guest emits the value-heap Result
            // HANDLE into a slot, then decomposes it into the canonical `(disc, i32, i32)`
            // register-flatten via `emit_result_arg_reg_flatten` (the register twin of the `result<list
            // <u8>, enum>` record-FIELD flatten) — Ok copies the payload rope into `mem` at the cursor
            // and yields `(0, ptr, len)`, Err yields `(disc, err-enum-disc, 0)`. Checked BEFORE the
            // scalar `_` arm (a Sum's `emit` yields a HANDLE, not the flattened slots the built-in
            // `result` param expects); `option`/`variant`/`enum` above already declined a result shape.
            _ if crate::backend::wasm::host::result_bytes_enum(db, &at).is_some() => {
                let res_slot = arg_base.max(*high);
                scratch_ty.insert(res_slot, ValType::I32);
                *high = (*high).max(res_slot + 1);
                emit(db, arg, slots, res_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(res_slot));
                let cursor = scratch_cursor_slot
                    .expect("a result<list<u8>, enum> arg reserves the scratch cursor (pre-scan)");
                let work_base = *high;
                emit_result_arg_reg_flatten(res_slot, cursor, work_base, high, scratch_ty, out)?;
                // MARSHALED-ARG RECLAIM (v-memory-safety, result twin of the option/variant host-arg
                // reclaim): `emit_result_arg_reg_flatten` read the disc + (Ok) copied the Bytes rope /
                // (Err) read the enum disc via borrowing `sum-disc`/`sum-payload`/`bytes-get` — pure
                // borrow, no dup, no handle moved out — so the Result handle in `res_slot` is DEAD after
                // the flatten. When the arg is a freshly-built OWNED result (`heap_operand_ownership ==
                // Owned`) or a child-dup site, deep-drop it (the payload was COPIED out, so the cascade is
                // balanced; else the Result shell leaks per host call). A BORROWED result is left
                // untouched (leak-over-UAF). Import mirror in `collect_used_ops`'s result-arg arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(res_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `result<scalar, enum>` argument: the guest emits the value-heap Result HANDLE
            // into a slot, then decomposes it into the canonical `(disc, join)` register-flatten via
            // `emit_result_scalar_arg_reg_flatten` (the 2-slot scalar-Ok twin of the Bytes-Ok result) —
            // Ok unboxes the scalar into the join slot, Err reads the err enum's disc. No rope, so NO
            // cursor (unlike the Bytes result). Checked BEFORE the scalar `_` arm (a Sum's `emit` yields
            // a HANDLE, not the flattened slots); option/variant/enum/result-bytes above already declined.
            _ if crate::backend::wasm::host::result_scalar_enum(db, &at).is_some() => {
                let res_slot = arg_base.max(*high);
                scratch_ty.insert(res_slot, ValType::I32);
                *high = (*high).max(res_slot + 1);
                emit(db, arg, slots, res_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(res_slot));
                let work_base = *high;
                emit_result_scalar_arg_reg_flatten(
                    db, res_slot, &at, work_base, high, scratch_ty, out,
                )?;
                // MARSHALED-ARG RECLAIM (result-scalar twin of the Bytes-result reclaim): the flatten
                // borrowed `sum-disc`/`sum-payload`/unbox — no dup, no handle moved out — so the Result
                // handle in `res_slot` is DEAD. Deep-drop iff Owned / a dup-site (else the shell leaks).
                // Import mirror in `collect_used_ops`'s result-scalar-arg arm.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(res_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `result<record-of-scalars, enum>` argument: the guest emits the value-heap
            // Result HANDLE into a slot, then decomposes it into `(disc, record-fields…)` via
            // `emit_result_record_arg_reg_flatten` (the record-Ok twin of the scalar-Ok result) — Ok
            // marshals the payload record's fields into the join slots (WIT order), Err puts the err enum's
            // disc in the first slot. No rope, so NO cursor. Checked BEFORE the scalar `_` arm (a Sum's
            // `emit` yields a HANDLE); the other result arms above are mutually exclusive by the Ok shape.
            _ if crate::backend::wasm::host::result_record_enum(db, &at).is_some() => {
                let res_slot = arg_base.max(*high);
                scratch_ty.insert(res_slot, ValType::I32);
                *high = (*high).max(res_slot + 1);
                emit(db, arg, slots, res_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(res_slot));
                let (ok_record, _errs) =
                    crate::backend::wasm::host::result_record_enum(db, &at).unwrap();
                let ok_wit = match wit_params.as_ref().and_then(|p| p.get(arg_i)) {
                    Some(crate::wit_world::WitType::Result { ok: Some(w), .. }) => (**w).clone(),
                    _ => {
                        return Err(Reject::decline(
                            "a result<record,enum> arg has no WIT Ok record type",
                        ));
                    }
                };
                // A Bytes/list field of the Ok record copies into `mem` at the cursor (a record of only
                // scalars needs none). `scratch_cursor_slot` is reserved by the pre-scan when the record
                // has such a field (below); pass it through (None for an all-scalar record).
                let work_base = *high;
                emit_result_record_arg_reg_flatten(
                    db,
                    res_slot,
                    &ok_record,
                    &ok_wit,
                    scratch_cursor_slot,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (result-record twin): the flatten borrowed the handle (sum-disc/
                // sum-payload/arr-get/unbox) — no dup, no handle moved out — so the Result handle in
                // `res_slot` is DEAD. Deep-drop iff Owned / a dup-site. Import mirror in `collect_used_ops`.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(res_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `result<tuple-of-scalars, enum>` argument: the guest emits the value-heap Result
            // HANDLE into a slot, then decomposes it into `(disc, elem0, elem1, …)` via
            // `emit_result_tuple_arg_reg_flatten` (the tuple-Ok twin of the record-Ok result) — Ok
            // marshals the payload tuple's elements into the join slots (positional), Err puts the err
            // enum's disc in the first slot. A bytes/list/compound element copies into `mem` at the
            // scratch cursor (the `has_runtime_compound` pre-scan reserves it iff the Ok tuple has such an
            // element; None for an all-scalar tuple). Checked BEFORE the scalar `_` arm; the other result
            // arms above are mutually exclusive by the Ok shape.
            _ if crate::backend::wasm::host::result_tuple_enum(db, &at).is_some() => {
                let res_slot = arg_base.max(*high);
                scratch_ty.insert(res_slot, ValType::I32);
                *high = (*high).max(res_slot + 1);
                emit(db, arg, slots, res_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(res_slot));
                let (ok_tuple, _errs) =
                    crate::backend::wasm::host::result_tuple_enum(db, &at).unwrap();
                let ok_wit = match wit_params.as_ref().and_then(|p| p.get(arg_i)) {
                    Some(crate::wit_world::WitType::Result { ok: Some(w), .. }) => {
                        Some((**w).clone())
                    }
                    _ => None,
                };
                let work_base = *high;
                emit_result_tuple_arg_reg_flatten(
                    db,
                    res_slot,
                    &ok_tuple,
                    ok_wit.as_ref(),
                    scratch_cursor_slot,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (result-tuple twin): the flatten borrowed the handle — no dup, no
                // handle moved out — so the Result handle in `res_slot` is DEAD. Deep-drop iff Owned / a
                // dup-site. Import mirror in `collect_used_ops`.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(res_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A top-level `result<list<scalar>, enum>` argument: the guest emits the value-heap Result
            // HANDLE into a slot, then decomposes it into `(disc, ptr/errdisc, count/0)` via
            // `emit_result_list_arg_reg_flatten` (the list-Ok twin of the Bytes-Ok result) — Ok marshals
            // the payload list into `mem` at the cursor (→ `(ptr, count)`), Err yields `(err-disc, 0)`.
            // Needs the scratch cursor (the list copy). Checked BEFORE the scalar `_` arm; the other result
            // arms above are mutually exclusive by the Ok shape.
            _ if crate::backend::wasm::host::result_list_enum(db, &at).is_some() => {
                let res_slot = arg_base.max(*high);
                scratch_ty.insert(res_slot, ValType::I32);
                *high = (*high).max(res_slot + 1);
                emit(db, arg, slots, res_slot + 1, high, scratch_ty, layout, out)?; // [handle]
                out.push(Lir::LocalSet(res_slot));
                let (elem, _errs) = crate::backend::wasm::host::result_list_enum(db, &at).unwrap();
                // The list element's declared WIT (the arg WIT is `result<list<T>, enum>`; the Ok arm
                // carries the `list<T>` whose element type orders a record element — `None` for a scalar).
                let elem_wit = match wit_params.as_ref().and_then(|p| p.get(arg_i)) {
                    Some(crate::wit_world::WitType::Result { ok: Some(w), .. }) => {
                        match w.as_ref() {
                            crate::wit_world::WitType::List(ew) => Some((**ew).clone()),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let cursor = scratch_cursor_slot
                    .expect("a result<list,enum> arg reserves the scratch cursor (pre-scan)");
                let work_base = *high;
                emit_result_list_arg_reg_flatten(
                    db,
                    res_slot,
                    &elem,
                    elem_wit.as_ref(),
                    cursor,
                    work_base,
                    high,
                    scratch_ty,
                    out,
                )?;
                // MARSHALED-ARG RECLAIM (result-list twin): the flatten borrowed the handle (sum-disc/
                // sum-payload + the list marshal's vec-len/vec-get) — no dup, no handle moved out — so the
                // Result handle in `res_slot` is DEAD. Deep-drop iff Owned / a dup-site. Import mirror in
                // `collect_used_ops`.
                if matches!(heap_operand_ownership(db, arg), Ok(HandleOwnership::Owned))
                    || out.dup_sites.contains(&arg)
                {
                    out.push(Lir::LocalGet(res_slot));
                    out.push(Lir::CallImport(OP_DROP));
                }
            }
            // A scalar argument emits its value directly.
            _ => emit(db, arg, slots, arg_base, high, scratch_ty, layout, out)?,
        }
        // Raise the floor past ANY scratch this arg consumed, so the NEXT arg allocates fresh slots
        // (never reusing — and thus never re-typing — a slot a prior marshal/checked-op still owns).
        arg_base = (*high).max(arg_base);
    }
    // GENERAL RESULT LIFT: a host op whose result is a SPILLED compound (its flattened core form
    // exceeds one value, so the canonical ABI returns it through a caller-provided pointer) is
    // canon-lowered `(args…, retptr) -> ()`. Allocate the return area sized+aligned by the result
    // type's canonical layout, pass it as the trailing arg, call, then lift the host-written value
    // into a Cadenza value-heap handle by recursing over the WIT result type (`emit_result_lift`).
    // This ONE recursion REPLACES the former per-shape lift blocks (`option<list<u8>>`, bare
    // `list<u8>`, `list<tuple<list<u8>,list<u8>>>`) — the general shape mechanism, not a 4th shortcut.
    // The admit predicate is the SAME `host::result_is_liftable` the host-import collection + the
    // component defined-type emission use, so a new structural shape (`list<list<u8>>` for
    // graph.neighbors) rides this recursion in lockstep with its core-sig/comp-type plumbing.
    let spilled_result = crate::backend::wasm::host::result_is_liftable(db, &result);
    if spilled_result {
        let (size, align) = canonical_layout(db, &result);
        let retptr = (*high).max(base);
        *high = (*high).max(retptr + 1);
        scratch_ty.insert(retptr, ValType::I32);
        // retptr = cabi_realloc(0, 0, align, size); leave it as the trailing call arg (tee-stash).
        out.push(Lir::ConstI32(0));
        out.push(Lir::ConstI32(0));
        out.push(Lir::ConstI32(align as i32));
        out.push(Lir::ConstI32(size as i32));
        out.push(Lir::CallImport("cabi_realloc"));
        out.push(Lir::LocalTee(retptr)); // [args…, retptr]
        out.push(Lir::CallHostImport(index)); // (args…, retptr) -> () ; host stored the result
        // The op's declared WIT result type (the host's canonical layout) drives a record result's
        // field ORDER in the lift — the result-side of the follow-the-WIT rule. Fall back to the
        // guest-`Ty`-derived WIT when the world is absent (structural results are order-agnostic).
        let result_wit = crate::backend::wasm::host::wit_op_result_type(db, &effect, &op)
            .or_else(|| crate::backend::wasm::host::spilled_result_wit_type(db, &result));
        emit_result_lift(
            db,
            &result,
            result_wit.as_ref(),
            retptr,
            0,
            high,
            scratch_ty,
            out,
        )?;
        return Ok(());
    }
    out.push(Lir::CallHostImport(index));
    Ok(())
}
