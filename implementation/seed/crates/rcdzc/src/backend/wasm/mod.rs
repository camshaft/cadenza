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

// ABI boundary-shape analysis for compound export arguments — the fixed-shape / among-scalars / closure
// boundary predicates the `emit_*_resource` emitters below call. Extracted from this file's grab-bag of
// free functions (behavior-preserving); `pub(super)` back to here.
mod arg_boundary;
pub mod coalesce;
pub mod dwarf;
// Host-import type + group construction (functype items, record/result defined-types, import groups,
// extern-import bridge) the assembly cascade below calls. Extracted from this file (behavior-preserving);
// `pub(super)` back to here.
pub mod encode;
pub mod envelope;
pub mod host;
mod host_imports;
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
// General component-model DEFINED-TYPE emission for arbitrary WIT value types — the type-table + hand-laid
// bytes that step W3 of general WIT-bindings emission lays for a typed world's records/variants/results.
// Not yet wired into the assembly cascade (that is a later slice); its bytes are pinned against the
// `wasm-encoder` oracle in-module.
#[allow(dead_code)]
pub mod wit_ctype;

use crate::backend::wasm::arg_boundary::{
    CompoundArgBoundary, GroupCompoundArg, NestedCompoundArgBoundary, closure_boundary_byte,
    closure_boundary_reject, fixed_shape_option_scalar_arg, fixed_shape_result_compound_arg,
    fixed_shape_scalar_tuple_arg, fixed_shape_sum_param_arg, multi_compound_args,
    nested_fixed_shape_tuple_arg, nested_sole_or_among_scalars, scalar_field_rebuild,
    single_compound_among_scalars, tuple_field_abi,
};
use crate::backend::wasm::envelope::BoundaryExport;
use crate::backend::wasm::host_imports::{
    build_host_group, build_host_result_types, collect_host_arg_strings, declare_result_lift_ops,
    extern_op_comp_functype, host_as_extern_for, host_op_comp_functype, host_param_abi,
};
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
///
//= spec/capabilities/debug-information.md#emitting-debug-information-does-not-change-observable-behavior
//# Emitting debug information for a program MUST NOT change the program's observable behavior.
///
//= spec/capabilities/debug-information.md#emitting-debug-information-does-not-change-observable-behavior
//# The portion of an artifact that the runtime executes MUST be byte-identical whether or not debug information is emitted for it, so that debug information occupies a region of the artifact the runtime does not execute rather than altering the code that runs.
/// Whether a result type crosses the host boundary via the RESOURCE ESCAPE (a heap value with no scalar
/// boundary valtype — `encode() -> list<u8>`) rather than the scalar path. This is EXACTLY the set the
/// resource-escape gate in [`emit`] admits for a single nullary export; keeping the predicate here (and
/// using it BOTH at the gate and at the multi-export diagnosis) means the "why did this decline" message
/// names the ACTUAL constraint (arity — a compound must be the sole nullary export) rather than the type,
/// and the two can never drift. A `Ty::Fn` (closure) is NOT here — it has its own resource path with a
/// `call` method, keyed separately.
pub(crate) fn crosses_as_resource_escape(ty: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    matches!(
        ty,
        Ty::Tuple(_)
            | Ty::Record(_)
            | Ty::Sum { .. }
            | Ty::List(_)
            | Ty::Map(_, _)
            | Ty::Set(_)
            | Ty::Bytes
            | Ty::String
            | Ty::Symbol
            // A BigInt is a heap handle with NO scalar boundary valtype (only the fixed-width int aliases
            // 8/16/32/64 cross as scalars), so it crosses via the value-form escape like Bytes/String —
            // the value-encode walk renders it as a KIND_INT leaf (`Shape::BigInt`, descriptor tag 17).
            | Ty::BigInt
            // A Rational has no scalar boundary valtype either, so it crosses via the value-form escape.
            // A CONSTANT rational's `constant_value_form` bakes its `num/den` name leaf (B4-1); a runtime-
            // computed rational crosses via the same value-encode walker (`ShapeNode::Rational`, R3c),
            // rendered as a `{numerator, denominator}` record.
            | Ty::Rational
            | Ty::Nominal { .. }
            | Ty::Qty { .. }
            // A TYPE-VALUE is a first-class value that crosses the boundary (core-semantics.md §Types Are
            // First-Class Values — "returned from a function, and inspected at runtime"). It is fully
            // compile-time-known, so it crosses via the CONSTANT value-form escape: `constant_value_form`
            // bakes `(: <TypeName> Type)` from the reduced type (nullary export only — a type-valued export
            // has no parameter to depend on). Its runtime footprint is nil (the type is erased); the escape
            // carries only the baked rendering.
            | Ty::Type
    )
}

/// Append the lambda-lifted closure bodies to `funcs`, AFTER the `layout.order` defs, in table-slot
/// order — the shared step both the main emit path and the runtime-resource ESCAPE path need so a
/// first-class closure's `call_indirect` has a body to dispatch (at func idx `import_base + order.len() +
/// slot`) and the funcref table/elem (laid by the module assembler from `layout.lifted`) resolves. Each
/// lifted lambda is UNIFORMLY an `(env, params…) -> ret` function: local slot 0 is the closure CELL (the
/// env, read by `Core::Captured` as `arr-get(local 0, 1+index)`), read via a FRESH `$closure-env` key that
/// nothing in the body resolves to (so it claims slot 0 without shadowing the lambda's own params). An
/// UNREACHED lambda (demanded during type-checking but built by no reachable `Core::Closure`) emits an
/// inert stub with the same signature — never called (its table entry is laid but never selected), keeping
/// the function-index space + type section consistent. Mirrors the loop inlined at the main-path sites.
fn append_lifted_bodies(
    db: &mut Db,
    funcs: &mut Vec<SelectedFunc>,
    layout: &Layout,
) -> Result<(), Reject> {
    for (code, lifted) in layout.lifted.clone().into_iter().enumerate() {
        let env_key = db.push_name("$closure-env");
        let mut params = vec![(env_key, crate::ty::Ty::Bytes)];
        params.extend(lifted.params.iter().cloned());
        if layout.lifted_reached.get(code).copied().unwrap_or(true) {
            funcs.push(select_function_of(db, lifted.body, &params, layout, None)?);
        } else {
            funcs.push(select::stub_function(&params, &lifted.ret_ty));
        }
    }
    Ok(())
}

pub fn emit(
    db: &mut Db,
    layout: &Layout,
    spans: Option<&crate::spans::SpanData>,
    external_debug_info: Option<&str>,
) -> Result<Vec<u8>, Reject> {
    // KEBAB-NAME COLLISION GUARD. Each export's component extern name is normalized to kebab-case
    // (`kebab_extern_name`, applied at `comp_export_item`), so two DISTINCT source names could normalize
    // to the SAME extern name (`(export fA)` + `(export f-a)` → both `f-a`). Emitting that would produce a
    // component with a duplicate export name (invalid) or silently drop one — so reject it here, before
    // any emit path, naming the two colliding source names. (An identity-normalized common case never
    // collides with itself; only genuinely-distinct source names that share a normalized form do.)
    if let Some(reject) = crate::backend::common::export_name::kebab_export_collision(layout) {
        return Err(reject);
    }
    // BOUNDARY-NAME VALIDITY GUARD. An export/`@test` name that normalizes to an invalid kebab word (a
    // hyphen-delimited segment starting with a DIGIT — `step-by-2`, `a-2-b`) would emit a component
    // `wasmtime` rejects wholesale at load, with no diagnostic (a silent total-component failure — every
    // test in the file "fails", or the artifact is unloadable). Reject it here, before emit, naming the
    // offending name + the fix — the export-name analogue of the `is_valid_interface_name` guard.
    if let Some(reject) = crate::backend::common::export_name::invalid_kebab_export_name(db, layout)
    {
        return Err(reject);
    }
    // HOST-OP BOUNDARY-REPRESENTABILITY guard — hoisted BEFORE every emit path (escape / closure / main),
    // so an unsupported host op declines HONESTLY regardless of which path the export's result routes to.
    // A host op with a DETERMINED String/compound RESULT (or a determined compound ARGUMENT) has no
    // boundary form this compiler emits yet; without this, such an op collected `result: None` and hit
    // `select`'s internal "not in the host-import set" message (documented "a compiler bug") — and a
    // String-RESULT op used AS the export routed to `emit_runtime_resource` (the value-escape path) and hit
    // it THERE, ahead of the post-collection check further down. Checking here covers all paths uniformly.
    // (`ty_undetermined`-gated inside, so a fold-synthesized `Ty::Any` HostCall is not falsely flagged.)
    // A WORLD-DRIVEN bytes provider (a reducer whose target WIT world names a `list<u8>`-crossing member)
    // takes the host-fused bytes path, which now LIFTS an `option<list<u8>>` host result into a value-heap
    // `Option<Bytes>` (the kv.get select lift, GAP C) — so that result is representable HERE. Every OTHER
    // emit path lacks the lift, so it stays a decline. Gate on being that provider (component-name + a world
    // bytes member).
    // The typed interface-instance path (`record_interface_export` → `assemble_typed_interface_with_host_
    // runtime`) ALSO lifts a compound host result (W4c-b-iii): a reducer guest that exports a typed record
    // interface AND performs a world import (`identity.id`) composes the host import + runtime + typed export.
    // So a compound host result is representable when the world has a typed record-param export member too.
    // (A guest that has such a member but routes elsewhere is caught by the plain host-delegating path's own
    // compound-result decline, so this broadening never mis-emits.)
    let allow_option_bytes = db.component_name.is_some()
        && db.wit_world.clone().is_some_and(|wb| {
            world_bytes_crossing_export(layout, &wb).is_some() || world_has_typed_record_export(&wb)
        });
    for &def in &layout.order {
        let body = def_body(db, def)?;
        if let Some((op, pos, ty)) =
            host::first_unrepresentable_host_op(db, body, allow_option_bytes)
        {
            // "an argument" / "a result" — the article agrees with the position word.
            let article = if pos == "argument" { "an" } else { "a" };
            return Err(Reject::unsupported(format!(
                "the host operation `{op}` has {article} {pos} of type `{ty}`, which has no component \
                 boundary form this compiler emits on a bare `(effect …)`. On a bare effect, host \
                 RESULTS cross as a scalar/unit, and host ARGUMENTS as a scalar/unit/string or a `list<u8>` \
                 (Bytes). A `list<u8>` (Bytes) or `option<list<u8>>` RESULT crosses on the WORLD-DRIVEN \
                 boundary path — a component that declares an imposed `(wit-world …)` for the effect's \
                 interface — NOT on a bare effect (that path lifts the host bytes into a value-heap handle; \
                 the bare host-call emit does not). A record/compound ARGUMENT, or a \
                 `list<list<u8>>`/`list<tuple<…>>` RESULT, is not supported on a bare effect."
            )));
        }
    }
    // ESCAPING-CLOSURE guard (CDZ0406) — hoisted BEFORE every emit path, so a closure that performs an
    // effect and crosses the host boundary declines with the CODED CDZ0406 regardless of HOW it escapes
    // (a bare closure result, or one NESTED in a returned tuple/record/sum/collection). The individual
    // `emit_*_resource` emitters each run this same `layout.lifted` scan, but only for the shapes they
    // handle — a closure nested in a COMPOUND result (`(host (ask) (tuple 1 (fn (x) (+ x (ask.ask)))))`,
    // breaker es1) routes to a compound-escape path that lacked the scan, so it fell through to `select`'s
    // generic "not in the host-import set" decline (code None) — a diagnostic-parity gap vs the rust
    // backend, which rejects CDZ0406 for the same program. Scanning here covers all faces uniformly.
    // The scan targets the LIFTED CLOSURE BODIES (`layout.lifted`) — the code that runs LATER, when the
    // host invokes `call()`, OUTSIDE the delegation frame — so a `Core::HostCall` there is a genuine escape
    // with no home. A make-time HostCall in the EXPORT body proper (the build-time-delegated case
    // `(host (ask) (let ((v (ask.ask))) (fn (x) (+ x v))))`, discharged while the delegation is still in
    // dynamic scope, the closure capturing only the plain result) is NOT in a lifted body and is correctly
    // NOT flagged. (The per-path scans remain as-is — now redundant for the shapes reached here, harmless.)
    // Only scan REACHED lifted slots: `layout.lifted` also holds UNREACHED lambdas (demanded during
    // type-checking but built by no reachable `Core::Closure`), which `append_lifted_bodies` emits as inert
    // never-called STUBS gated on `layout.lifted_reached`. A HostCall in such a dead/stub body is provably
    // unreachable — flagging it would spuriously reject CDZ0406 a program whose effectful closure can never
    // run. Mirror `append_lifted_bodies`' `layout.lifted_reached` gate so only a genuinely-reachable escaping
    // closure is caught; stop at the first (a coded reject needs just one).
    {
        let mut escaping = Vec::new();
        for (code, l) in layout.lifted.iter().enumerate() {
            if !layout.lifted_reached.get(code).copied().unwrap_or(true) {
                continue;
            }
            host::collect_host_imports(db, l.body, &mut escaping);
            if !escaping.is_empty() {
                break;
            }
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
    // The admitted set is `crosses_as_resource_escape` — every heap value with no scalar boundary valtype:
    // Tuple/Record/Sum (compounds), List/Map/Set (collections — CONSTANT bytes baked, a runtime collection
    // whose `constant_value_form` is None falls through to the decline, its looping walker a later
    // increment), Bytes/String/Symbol (heap byte/name values — a RUNTIME one routes to the looping-walker
    // path below), a NOMINAL newtype (crosses as its erased underlying value, tagged under the nominal
    // name), and a QUANTITY (its full value form bakes the unit the scalar path would erase). Only the
    // single nullary-export case takes the resource shape; any other (multi-export, parameterized) falls
    // through and declines below with the arity/parameter diagnosis (`crosses_as_resource_escape` there too,
    // so the message names the real constraint, not the type).
    // A PROVIDER (X5c, `db.component_name` set) publishes its exports to a PEER over the shared runtime,
    // so a compound result crosses as its `u32` handle through the provider interface — NOT as the HOST
    // resource-escape (which serializes to `list<u8>` and declines a parameterized heap return). Skip the
    // escape for a provider; a compound export takes the provider path below.
    if db.component_name.is_none()
        && let [e] = &layout.exports[..]
        && crosses_as_resource_escape(&e.result)
    {
        let body = def_body(db, e.def)?;
        // A CONSTANT compound (a foldable body) bakes its value bytes into the resource core with a
        // NULLARY `make` — only a nullary export can be constant (a parameterized body's value depends on
        // its argument, so `constant_value_form` returns `None` and it falls through to the runtime
        // walkers below, whose `make` forwards the params). Guard the bake on nullary so a parameterized
        // export never takes the constant shape.
        if e.params.is_empty()
            && let Some(value_bytes) = crate::lower::constant_value_form(db, body)
        {
            let main_core = serialize::resource_core_module(&value_bytes);
            let dtor_core = serialize::resource_dtor_module();
            return Ok(envelope::assemble_resource(&main_core, &dtor_core));
        }
        // A LIST result whose value is NOT constant-foldable (a runtime-built list) has no baked-bytes
        // form, and there is no runtime value-form template for a list yet (its length is dynamic, so the
        // `encode()` walker would need to LOOP — a later increment). It is not a sum, and
        // `runtime_value_form_template` returns `None` for a list, so it falls through to the decline
        // below — an honest "runtime list return not yet supported", not a miscompile.
        // Route the RUNTIME escape on the ERASED result type. A newtype-over-a-heap-type result is a
        // `Ty::Nominal { inner }` whose runtime value IS its `inner` (the tag adds nothing), so a
        // newtype-over-SUM (`(type Cached (Mk (Option Int64)))` returned at run time) must take the SUM
        // escape by its inner sum, not fall through to the scalar decline below. `strip_nominal` peels the
        // tag so a nominal-over-{sum,bytes,compound} routes exactly as the bare underlying type does. (The
        // CONSTANT-value path above is type-agnostic, so this matters only for a RUNTIME newtype value —
        // which used to DECLINE "a sum crosses the boundary only as a single nullary export's result".)
        let result = e.result.strip_nominal().clone();
        // A NOMINAL result whose erased underlying type is a RECURSIVE sum (a recursive newtype, `(type
        // Lst (Mk (Option (Tuple Int64 Lst))))`) escapes via the recursive-sum WALKER — but routed on the
        // UN-STRIPPED nominal, so the shape descriptor's top-level `Named` carries the nominal's OWN name
        // (`Lst`), not the inner sum's (`Option`). `sum_shape_descriptor` builds a `Named`-rooted shape for
        // a nominal directly (the erased-tag frame), closing the recursion on the newtype's decl. Tried
        // before the stripped-sum routing so the name is preserved; a non-recursive nominal (a flat
        // `sum_form_template` exists) still takes the stripped path below.
        // The un-stripped-nominal recursive-sum route is ONLY for a newtype whose erased inner is a HEAP
        // value (a recursive sum, a compound) — its `encode()` walks the heap spine. A newtype over a
        // SCALAR (`(type W (Mk Int64))`) erases to a bare core int with NO heap spine; `sum_shape_descriptor`
        // still builds a descriptor for its nominal frame, so WITHOUT this guard a scalar newtype was routed
        // through `emit_recursive_sum_resource` (which expects a heap handle) while its value is a raw i64 →
        // an invalid module ("expected i32, found i64") for a scalar-newtype value escaping a PARAM'd def
        // (adv-63b; the nullary case bakes a constant and never reaches here). Gate on the stripped inner
        // being a resource-escape (heap) type — a scalar-erased newtype falls through to the scalar
        // `runtime_value_form_template` branch below (which boxes it).
        if matches!(&e.result, crate::ty::Ty::Nominal { .. })
            && crosses_as_resource_escape(&result)
            && crate::lower::sum_form_template(db, &result).is_none()
            && let Some(desc) = crate::lower::sum_shape_descriptor(db, &e.result)
        {
            return emit_recursive_sum_resource(db, layout, e.def, &desc, spans);
        }
        // A SUM result crosses through the resource shape but its `encode()` SWITCHES on the runtime
        // discriminant (`sum-disc`) and renders the matching variant — a per-variant template, not a
        // single flat one. Route through the sum escape when the sum has a value-form (`None` — a
        // variant with a non-renderable payload — falls through to decline below).
        if let crate::ty::Ty::Sum { .. } = &result {
            if let Some(sum_tpl) = crate::lower::sum_form_template(db, &result) {
                return emit_runtime_sum_resource(db, layout, e.def, &sum_tpl, spans);
            }
            // A RECURSIVE sum (a self-referential payload — a linked list, a tree) has no fixed
            // per-variant template (`sum_form_template` returned `None`), so its `encode()` walks the heap
            // spine to a runtime-determined depth. Build the SHAPE DESCRIPTOR and route through the
            // runtime `value-encode` op: the encode body bakes the descriptor as a heap Bytes, calls
            // `value-encode(rep, desc)` to get the value-form document, and copies it out
            // (`DESIGN-recursive-sum-escape-walker.md`, approach C).
            if let Some(desc) = crate::lower::sum_shape_descriptor(db, &result) {
                return emit_recursive_sum_resource(db, layout, e.def, &desc, spans);
            }
        } else if matches!(result, crate::ty::Ty::Bytes | crate::ty::Ty::String) {
            // A RUNTIME `Bytes` result (a `concat`/recursion-built sequence — not a compile-time constant)
            // crosses through the resource shape, but its value form is VARIABLE-length: `encode()` LOOPS,
            // writing the static prefix, the runtime `bytes-len` as a LEB, a `bytes-get` copy loop, then
            // the static suffix (`DESIGN-runtime-bytes-escape-walker.md`). The FIRST looping walker.
            //
            // A RUNTIME `String` (a `String.concat`/recursion-built UTF-8 rope) is the SAME byte-leaf heap
            // rep as Bytes (`String.concat` is `bytes-concat`), so it escapes through the SAME walker — only
            // the value-form framing differs (`(: "…" String)` vs `(: b"…" Bytes)`, via `runtime_string_form`
            // vs `runtime_bytes_form`). This gives a runtime String `encode` + the `len`/`is-empty`/`to-bytes`
            // methods for free (VM: String-as-resource-with-methods), replacing the prior "String has no
            // component boundary representation" decline.
            let form = if matches!(result, crate::ty::Ty::String) {
                crate::lower::runtime_string_form(db)
            } else {
                crate::lower::runtime_bytes_form(db)
            };
            if let Some(form) = form {
                return emit_runtime_bytes_resource(db, layout, e.def, &form, spans);
            }
        } else if matches!(
            result,
            crate::ty::Ty::List(_)
                | crate::ty::Ty::Map(_, _)
                | crate::ty::Ty::Set(_)
                // A RUNTIME-computed BigInt/Rational result crosses via the SAME `value-encode` walker —
                // its magnitude is variable-length (no fixed hole-template), and the runtime already
                // renders `Shape::BigInt`/`Shape::Rational`. Only a NULLARY export reaches here (the
                // parameterized-heap-return guard above is general); a constant BigInt still takes the
                // baked-bytes constant path (`constant_value_form`) — this serves the runtime-COMPUTED one
                // (`(* (BigInt.of a) (BigInt.of b))` at a nullary export).
                | crate::ty::Ty::BigInt
                | crate::ty::Ty::Rational
        ) && let Some(desc) = crate::lower::sum_shape_descriptor(db, &result)
        {
            // A RUNTIME `List`/`Map`/`Set` (a push/insert/recursion-built collection — not constant-
            // foldable) has a VARIABLE size, so it escapes via the runtime `value-encode` op (the same
            // walker a recursive sum uses): the encode body bakes the compiler's shape descriptor as a heap
            // Bytes, calls `value-encode(rep, desc)` to render the value form (`(: (list …) (List <e>))` /
            // `(: (map (k v) …) (Map <k> <v>))` / `(: ((. Set of) (list …)) (Set <e>))`, entries in
            // canonical key order), and copies it out. `sum_shape_descriptor`'s List/Map/Set arm builds a
            // parametric `Framed(<type-node>, …)` frame so the element/key/value types are observable — the
            // type node is RECURSIVE, so a nested element crosses too (`(List (List Int64))`, `(Map K (Set
            // V))`), and the inner value shape already recurses to render the nested collection values.
            return emit_recursive_sum_resource(db, layout, e.def, &desc, spans);
        } else if let Some(tpl) =
            crate::lower::runtime_value_form_template(&e.result, &db.name_ctx())
        {
            // A RUNTIME compound (not constant-foldable — a recursive return, a call whose result is
            // built on the heap) crosses through the SAME resource shape, but its `encode()` WALKS the
            // live handle rather than baking constant bytes (R2). Build the value-form TEMPLATE for the
            // result type; if it has one, route through `assemble_runtime_resource`.
            // A SCALAR-ERASED result (a runtime `Qty` — it erases to its bare inner scalar, not a heap
            // handle) needs `make` to BOX that scalar before `resource-new`; signal the box op + any extend.
            // Only a Qty over an Int inner takes the runtime-scalar template (the `Ty::Qty` arm scopes to Int
            // + reference unit). `box-int` boxes it; an Int inner in an I32 SLOT (ground width ≤ 32) is an i32
            // core value that must be i32→i64 widened first (signed/unsigned per its signedness) — the same
            // extend `emit_box_i32_to_i64_extend` applies to a boxed narrow leaf. Gate on the SLOT (`≤ 32` =
            // `int_valtype` I32), NOT `< 64`: a MID-WIDTH inner (33..63, e.g. `(Int 40)` via a `.wrap`) is
            // ALREADY an i64 slot, so extending it would emit `i64.extend_i32_*` on an i64 → invalid wasm
            // ("expected i32, found i64"). A full-width Int64/UInt64 (and any ≥33-bit width) needs no extend.
            // A compound result stays `None` (already a handle).
            let scalar_box = match result.strip_nominal() {
                crate::ty::Ty::Qty { inner, .. } => match inner.strip_nominal() {
                    crate::ty::Ty::Int(it) if it.ground_width() <= 32 => {
                        Some(("box-int", Some(it.ground_signed())))
                    }
                    _ => Some(("box-int", None)),
                },
                // A scalar-erased NEWTYPE (`(type W (Mk Int64))`) returned from a PARAMETERIZED def: the
                // routing top stripped its tag, so `result` is a bare `Ty::Int`. Its runtime value is a raw
                // core int — `make` must BOX it (`resource-new` takes a heap handle), exactly like the Qty
                // case. Without this, `make` handed the raw scalar where the boxed handle was expected. Box
                // with the i32→i64 extend a narrow leaf needs (gate on the SLOT ≤ 32; a mid-width 33..63 is
                // already an i64 slot). This is the scalar half of the adv-63b fix — the other half gates the
                // recursive-sum branch above off a scalar newtype so it FALLS THROUGH to here.
                crate::ty::Ty::Int(it) if it.ground_width() <= 32 => {
                    Some(("box-int", Some(it.ground_signed())))
                }
                crate::ty::Ty::Int(_) => Some(("box-int", None)),
                _ => None,
            };
            // PRE-ENCODE (Axis 2): bake any compile-time-constant leaf of a TUPLE/RECORD return directly into
            // the value-form template, dropping its runtime hole — so a partially/fully-constant compound
            // return no longer re-encodes its constant leaves on every event (a fully-constant one becomes a
            // hole-free memcpy). Byte-identical when nothing bakes; scoped to a compound result so the
            // scalar-erased (`scalar_box`) template — a single boxed scalar the `make` body still forwards — is
            // untouched. The return body's core supplies the constant values (`bake_constant_leaves` keeps any
            // non-literal / runtime leaf as a hole).
            let tpl = if matches!(
                result.strip_nominal(),
                crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_)
            ) {
                let body = def_body(db, e.def)?;
                crate::lower::bake_constant_leaves(db, body, &tpl)
            } else {
                tpl
            };
            return emit_runtime_resource(db, layout, e.def, &tpl, scalar_box, spans);
        } else if matches!(result, crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_))
            && let Some(desc) = crate::lower::sum_shape_descriptor(db, &result)
        {
            // A TUPLE/RECORD whose `runtime_value_form_template` was `None` because it contains a
            // VARIABLE-LENGTH element (a runtime-built list/map/set, or a sum) — the fixed-hole template
            // can't represent a dynamic-depth element. It escapes via the SAME runtime `value-encode`
            // walker a bare collection uses: `sum_shape_descriptor`'s Tuple/Record arm builds a parametric
            // `Framed(<type-node>, …)` frame and `shape_of` recurses into the collection element (looping
            // to runtime depth), so `(tuple (build 3) 30)` renders `(: (tuple (list 1 2 3) 30) (Tuple (List
            // Int64) Int64))`. A FIXED-shape tuple/record (all scalar/byte/fixed-compound elements) already
            // took the cheaper static-template arm above; this fallback serves ONLY the variable-shape case
            // — the "show DATA + RESULT together" example pattern (a tuple of a runtime-built list + a
            // computed scalar) v-guide-infra flagged. (Before this it fell through to the honest
            // "value-form walker that loops to a runtime-determined depth is not yet emitted" decline.)
            return emit_recursive_sum_resource(db, layout, e.def, &desc, spans);
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

    // DIRECTION 2, the ROUND-TRIP (C-HOST-4): an export whose PARAMETER is a closure `(-> …)` — the host
    // hands a closure resource back in, and the body applies it. Routed to `emit_roundtrip_resource` when
    // the program ALSO has a PRODUCER export (a closure-RESULT) so the closure the consumer receives was
    // minted in-guest (its lifted lambda is in-program → the consumer's `call_indirect` resolves by
    // signature; the consumer body treats its closure param as a cell, and the serializer wrapper
    // `resource.rep`s the boundary handle → cell). A consumer-ONLY program (a host-fabricated closure)
    // stays out of scope — `emit_roundtrip_resource` declines it (no producer).
    if layout.exports.iter().any(|e| {
        e.params
            .iter()
            .any(|(_, t)| matches!(t, crate::ty::Ty::Fn(_, _)))
    }) {
        // Collect every closure SIGNATURE the program touches — a producer's result + each consumer's
        // closure param. If they are all the SAME, the single-resource round-trip handles it; if there is
        // MORE THAN ONE distinct signature, route to the N-resource-type distinct-sig round-trip.
        let mut sigs: Vec<&crate::ty::Ty> = Vec::new();
        for e in &layout.exports {
            if matches!(e.result, crate::ty::Ty::Fn(_, _)) && !sigs.contains(&&e.result) {
                sigs.push(&e.result);
            }
            for (_, t) in &e.params {
                if matches!(t, crate::ty::Ty::Fn(_, _)) && !sigs.contains(&t) {
                    sigs.push(t);
                }
            }
        }
        if sigs.len() > 1 {
            return emit_distinct_sig_roundtrip_resource(db, layout, spans);
        }
        return emit_roundtrip_resource(db, layout, spans);
    }

    // MULTI-EXPORT closures: more than one export, and EVERY export's result is a closure of the SAME
    // signature. They cross together as one resource type with N `make-<name>` functions sharing one
    // `call` (`emit_multi_closure_resource`; the `multi_export_closures_share_one_call` oracle proved the
    // shape). A mix of DISTINCT closure signatures (needing N resource types) or a closure alongside a
    // non-closure export is a later slice — declined below with a message naming what is unsupported.
    if layout.exports.len() > 1
        && layout
            .exports
            .iter()
            .all(|e| matches!(e.result, crate::ty::Ty::Fn(_, _)))
    {
        // All closure exports must share ONE signature (the shared `call`'s functype). Compare each
        // export's result type to the first; a mismatch → decline (distinct-signature multi-export is the
        // N-resource-type slice, not yet built).
        let first = &layout.exports[0].result;
        if layout.exports.iter().all(|e| &e.result == first) {
            let defs: Vec<usize> = layout.exports.iter().map(|e| e.def).collect();
            let result = first.clone();
            return emit_multi_closure_resource(db, layout, &defs, &result, spans);
        }
        // DISTINCT signatures — each becomes its own resource type. `emit_distinct_sig_resource` groups the
        // exports by signature and publishes G resource types (the `distinct_signature_…` oracle proved it).
        return emit_distinct_sig_resource(db, layout, spans);
    }
    // A closure export ALONGSIDE a non-closure (plain) export — a MIXED multi-export. The closure(s) cross
    // via the resource envelope (`make`/`call` under `cadenza:closure/exports`); each plain export is
    // aliased off the SAME program instance and published as an ORDINARY top-level component func (the
    // `oracle_mixed_component` byte anchor proved the coexistence). Handled when the closure exports all
    // share ONE signature (the shared-`call` shape); distinct closure signatures alongside a plain export
    // remain a further widening (declined inside `emit_mixed_closure_resource`).
    if layout.exports.len() > 1
        && layout
            .exports
            .iter()
            .any(|e| matches!(e.result, crate::ty::Ty::Fn(_, _)))
    {
        return emit_mixed_closure_resource(db, layout, spans);
    }

    // The STATIC BYTES table (`DESIGN-static-data.md` §2d): the distinct fully-constant `Bytes` payloads
    // the program builds once into module globals (a `start` init) and reads with `global.get` + a dup at
    // each use. Collected BEFORE the import set so the forced ops below (the init's `bytes-alloc`/
    // `bytes-set` + the read-`dup`) join it, and attached to the layout below so selection's `Core::BytesOf`
    // arm can route a constant literal to its global. Empty for a program with no constant bytes literal.
    let static_bytes = collect_static_bytes(db, &layout.order);
    // §2d increment 6: the markable constant Tuple/Record roots hoisted to build-once immortal globals.
    // Collected here (before the import set) so its init ops are forced below; the init Lir itself is built
    // after `with_static_bytes` (it needs the byte-global count for the compound global indices).
    let static_compounds = collect_static_compounds(db, &layout.order);
    // The per-program runtime IMPORT SET must be fixed BEFORE selection, because it determines both
    // `layout.import_base` (the shift a defined func's index takes) and the index a `CallImport`
    // resolves to. Walk every reachable body's core for the value-heap ops it will emit
    // (`collect_used_ops`, which mirrors `select`'s op choices exactly), collect them into a
    // deterministic sorted set, and resolve each to its generated `RtOp`. Empty for a program that uses
    // no runtime op — no import section, no shift → byte-identical to a runtime-free build.
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_module_used_ops(db, layout, &mut used)?;
    // STATIC BYTES need their ops in the import set: the `start` init builds each once with `bytes-alloc`
    // + `bytes-set`, then `mark-immortal`s it (rc=IMMORTAL: census-excluded + dup/drop no-op). A hoisted
    // use is just a bare `global.get` — no `dup` (the immortal's drop is a no-op, so a use needs no retain).
    // `collect_used_ops` already reports `bytes-alloc`/`bytes-set` for any `Core::BytesOf`, but the init's
    // `mark-immortal` (and a program whose only bytes are all hoisted) might otherwise import neither; force
    // the three the init emits when the table is non-empty. No-op (byte-identical) with no constant bytes.
    if !static_bytes.is_empty() {
        used.insert("bytes-alloc");
        used.insert("bytes-set");
        used.insert("mark-immortal");
    }
    // STATIC COMPOUNDS (increment 6): the `start` init builds each tuple/record/list/map/set/sum ONCE and
    // `mark-immortal`s it. Those init `mark-immortal`(-deep) / `vec-of-arr` / `map-*` / `set-*` / `sum-new` /
    // `value-canonicalize` / `bytes-compact` ops are NOT in any body when the only use of a constant is a
    // hoisted `global.get`, so the import set must force the init's ops. PRECISELY: import EXACTLY the ops the
    // init emits per each compound's SHAPE, derived by a dry-run of the real init emit path
    // (`collect_static_compound_ops` → `emit_immortal_static`). This replaces the prior UNCONDITIONAL full
    // batch (which force-imported arr/box/bytes/vec/map/set/canonicalize/compact whenever ANY static compound
    // existed) — that over-approximation left map/set/vec/bytes/canonicalize imports DEAD in a program whose
    // constants are e.g. only sums or scalar tuples (the imports-dominant unused-import gap, v-wasm-opt). No
    // mirror-divergence: the dry-run is the SAME emit path, so it imports neither less (missing-import →
    // invalid module) nor more than the init actually calls.
    for op in select::collect_static_compound_ops(db, &static_compounds, layout) {
        used.insert(op);
    }
    // A typed interface-export member with a RECORD param emits a boundary WRAPPER that BUILDS the record
    // from the flattened fields (`arr-alloc`/`arr-set` + per-field `box-*`). Those ops are the wrapper's,
    // not the guest body's, so add them to the used set here (before the import set is derived) — otherwise
    // the wrapper body cannot resolve them. No-op for any program that is not a record-param interface
    // export (the guest's own op set is unchanged).
    if let (Some(iface), Some(world_bytes)) = (db.component_name.clone(), db.wit_world.clone())
        && let Some((wrappers, _)) = record_interface_export(db, layout, &world_bytes, &iface)
    {
        used.insert("arr-alloc");
        used.insert("arr-set");
        for w in &wrappers {
            for p in w.params.iter().flatten() {
                for f in p {
                    f.collect_box_ops(&mut |op| {
                        used.insert(op);
                    });
                }
            }
            // A spilled compound RESULT reads its value off the returned handle via the recursive canonical
            // writer — collect every runtime op that plan calls (`arr-get`/`vec-len`/`vec-get`/`bytes-*` +
            // each scalar unbox) so they are imported.
            if let serialize::ResultLower::SpillRecord { write, .. } = &w.result {
                canon_write_ops(write, &mut |op| {
                    used.insert(op);
                });
            }
            // A `list<u8>`/Bytes result member (CopyBytes) copies the runtime bytes out (`bytes-len` +
            // `bytes-get` loop) and drops the handle — register those so the wrapper body resolves them.
            if matches!(w.result, serialize::ResultLower::CopyBytes) {
                used.insert("bytes-len");
                used.insert("bytes-get");
                used.insert("drop");
            }
            // A flat single-scalar-field record result (FlatScalarField) reads the one field off the def's
            // record handle (`arr-get` + the scalar unbox `read`) and returns the scalar — register those.
            if let serialize::ResultLower::FlatScalarField { read, .. } = &w.result {
                used.insert("arr-get");
                used.insert(*read);
            }
            // A TOP-LEVEL memory-bearing leaf PARAM: a `Bytes`/`String` copies the incoming `(ptr, len)` out
            // of linear memory via a `bytes-alloc` + `bytes-set` loop; a `list<scalar>` builds a value-heap vec
            // (`vec-empty` + per-element read/box/`vec-push`). The wrapper (its owner) drops the borrowed handle
            // after the call. Register those so the wrapper body resolves them.
            for m in w.mem_leaf_params.iter().flatten() {
                match m {
                    (serialize::MemLeafKind::Str | serialize::MemLeafKind::Bytes, drop_after) => {
                        used.insert("bytes-alloc");
                        used.insert("bytes-set");
                        if *drop_after {
                            used.insert("drop");
                        }
                    }
                    (serialize::MemLeafKind::List(elem), drop_after) => {
                        used.insert("vec-empty");
                        used.insert("vec-push");
                        used.insert(elem.box_op);
                        if *drop_after {
                            used.insert("drop");
                        }
                    }
                }
            }
            // A TOP-LEVEL `option<scalar>` PARAM (sum_params) branches on the boundary disc and builds the guest
            // sum cell via `sum-new` plus each arm's payload box ops, and the wrapper (its owner) drops the
            // borrowed shell after the call — register `sum-new` + the arm ops + `drop`.
            for (rebuild, drop_after) in w.sum_params.iter().flatten() {
                used.insert("sum-new");
                rebuild.arm_true.collect_ops(&mut |op| {
                    used.insert(op);
                });
                rebuild.arm_false.collect_ops(&mut |op| {
                    used.insert(op);
                });
                if *drop_after {
                    used.insert("drop");
                }
            }
        }
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

    // The per-program HOST-import set (E2h-2) — every host-delegated operation a reachable body performs
    // (a `Core::HostCall`), in first-encountered order. Like the runtime set, it must be fixed BEFORE
    // selection (it fixes the host-op call index a `Core::HostCall` resolves to) and it shifts the
    // defined-func index space. This increment supports HOST-ONLY programs: a program mixing host + value-
    // heap runtime imports declines below (the two index spaces compose in a later increment).
    // The per-program HOST-import set (a `Core::HostCall`), first-encountered order. An unrepresentable
    // host-op boundary type was already declined honestly at the top of `emit` (the hoisted guard), so
    // every op reaching here has an emittable scalar/unit/string signature.
    //
    // The component's import set is the UNION of the escaping rows of every exported entrypoint: the loop
    // runs `collect_host_imports` over each def in `layout.order` (the exports and what they reach) and
    // accumulates into one `host_imports` — so one instantiated component carries a single import surface
    // serving every entrypoint, and each entrypoint's authority is exactly the ops reachable from its own
    // body (co-locating a pure entrypoint with an effectful one grants the pure one nothing).
    //= spec/capabilities/capabilities-and-effects.md#a-component-is-bound-against-the-union-of-its-entrypoints-rows
    //# The set of host operations a component imports MUST be the union of the escaping rows its entrypoints acknowledge, so that a component instantiated once carries a single import surface serving every entrypoint it exports, as the component model requires.
    //= spec/capabilities/capabilities-and-effects.md#a-component-is-bound-against-the-union-of-its-entrypoints-rows
    //# The host grant required to instantiate a component MUST be that union, so that provisioning is per-component even though acknowledgment is per-entrypoint, and an entrypoint that reaches fewer effects than its neighbors is still hosted in a component provisioned for all of them.
    //= spec/capabilities/capabilities-and-effects.md#each-entrypoint-acknowledges-its-own-escaping-row
    //# Each entrypoint MUST acknowledge, at itself, the effects it delegates to the host, so that the authority a given entrypoint is permitted to reach is a property of that entrypoint and not of the module or component that contains it, and an entrypoint that delegates nothing is pure regardless of what its neighbors delegate.
    //= spec/capabilities/capabilities-and-effects.md#each-entrypoint-acknowledges-its-own-escaping-row
    //# The authority an entrypoint reaches MUST be determined by the operations reachable from its own body under its own delegations, so that co-locating a pure entrypoint with an effectful one in the same component does not grant the pure entrypoint any authority.
    //= spec/capabilities/capabilities-and-effects.md#the-program-manifest-is-the-union-of-its-entrypoints-delegations
    //# A program's capability manifest MUST be the union of the host delegations its entrypoints declare, so that the manifest is a projection of where authority actually enters the program and not of every effect any module declares.
    //= spec/capabilities/capabilities-and-effects.md#the-program-manifest-is-the-union-of-its-entrypoints-delegations
    //# Dependency resolution MUST NOT introduce a capability that no entrypoint in the program delegated.
    let mut host_imports: Vec<host::HostImport> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        host::collect_host_imports(db, body, &mut host_imports);
    }
    // The CROSS-COMPONENT extern-import set — a PEER-BOUND effect's escaping ops (U2). It is populated
    // ENTIRELY by the effect-binding conversion just below (a peer op is an escaping effect bound to a
    // peer contract, `db.effect_bindings`); there is no separate `(extern …)` surface any more (removed in
    // U4 — cross-component interop is unified with effects).
    let mut extern_imports: Vec<host::ExternImport> = Vec::new();
    // EFFECTS-UNIFICATION (U2): an escaping effect BOUND to a peer contract (`(bind Math "cadenza:math/api")`
    // → `db.effect_bindings`) is a PEER call, not a host call. Move each such host import into the extern
    // set, retargeted to its bound interface — so `Math.add` reaching the boundary emits through the peer
    // envelope exactly as an `(extern …)` op did. A host import whose effect is NOT bound stays a host call.
    // (An in-program `(handle Math …)` discharged the effect before it escaped, so a bound effect that a
    // handler intercepts contributes no import here — the test-override for free.)
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
                false // remove from the host set
            } else {
                true
            }
        });
    }
    // OPTION C (consumer emit): a per-file `@test` CONSUMER layout (`compute_tests_consumer`) records the
    // cross-edge shared-closure defs it imports in `layout.cross_edge_import` (`def → its position in
    // `extern_order``) with the `extern_order` entries themselves `(closure-iface, source_boundary_name(def))`
    // in provider-export order. Those cross-edges are PEER ops exactly like a bound effect's escaping ops — so
    // materialize an `ExternImport` for each, in `extern_order` position order, with the boundary ABI drawn
    // from the def's OWN signature (params via `export_params`, result via `type_of`) so the consumer's import
    // functype MATCHES the provider's export functype (ABI-agreement, the functype twin of the index-agreement
    // `cross_edge_import` already fixes). This joins the SAME extern-emit paths a peer-bound consumer takes
    // (`core_module_with_extern` / `core_module_with_extern_runtime` + the peer envelope), so no new assembly
    // shape is needed. EMPTY for every non-consumer layout (`cross_edge_import` is empty), so this loop adds
    // nothing and the emit stays byte-identical to before.
    // The count of extern imports ALREADY laid down (a peer-bound escaping effect, above) — the cross-edge
    // block appends AFTER these, so each cross-edge's FINAL `extern_order` position is `cross_edge_delta + its
    // layout-time 0-based position`. `compute_tests_consumer` computed the map 0-based; the shift below
    // reconciles it to the final order (0 when no coexisting peer effect → no-op).
    let cross_edge_delta = extern_imports.len();
    if !layout.cross_edge_import.is_empty() {
        // Order the cross-edges by their `extern_order` position so the built `ExternImport` set indexes
        // exactly as `cross_edge_import` (and the provider's export order) says — the import at position `p`
        // (after the `cross_edge_delta` shift below) IS the op `extern_order[p]`, which a
        // `Lir::CallExternImport(p)` resolves to.
        let mut by_pos: Vec<(usize, usize)> = layout
            .cross_edge_import
            .iter()
            .map(|(&d, &p)| (p, d))
            .collect();
        by_pos.sort_unstable();
        for (pos, def) in by_pos {
            let (iface, op) = layout.extern_order[pos].clone();
            let body = def_body(db, def)?;
            let result_ty = crate::infer::type_of(db, body);
            let params = crate::layout::export_params(db, def, &op)?;
            extern_imports.push(host::ExternImport {
                interface: iface,
                op,
                // A compound param/result crosses as its opaque `u32` runtime handle (X5); a scalar by value;
                // `Unit` is elided — exactly the provider's boundary ABI for the same def.
                params: params
                    .iter()
                    .filter_map(|(_, ty)| host::extern_abi_val_type(ty))
                    .collect(),
                result: host::extern_abi_val_type(&result_ty),
            });
        }
    }
    // An extern import composed with a HOST effect is not yet emitted (a consumer that both binds a peer
    // AND delegates a host effect — a further fusion). An extern + the value-heap RUNTIME (a consumer that
    // receives a compound `value` handle from a peer and inspects it) IS emitted (X5, `assemble_extern_runtime`).
    if !extern_imports.is_empty() && !host_imports.is_empty() {
        return Err(Reject::unsupported(
            "a cross-component extern import composed with a host effect is not supported \
             (the extern + host import fusion is unavailable)",
        ));
    }
    // A program mixing a host effect AND the value-heap runtime composes BOTH import spaces. A scalar/unit
    // host op takes `envelope::assemble_host_runtime`; a host op with a STRING parameter takes the
    // shared-memory fusion `envelope::assemble_host_runtime_mem` (the memory + two-interface shape). Both
    // are wired in the host block below.

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
    // The extern-import order (X4b) — `(interface, op)` pairs a peer-bound `Core::HostCall` resolves against
    // (select.rs maps such a call to `Lir::CallExternImport` of its position here).
    let extern_order: Vec<(String, String)> = extern_imports
        .iter()
        .map(|e| (e.interface.clone(), e.op.clone()))
        .collect();
    let layout = layout
        .with_import_base((imports.len() + host_imports.len() + extern_imports.len()) as u32)
        .with_host_order(host_order)
        .with_host_strings(host_strings)
        .with_static_bytes(static_bytes)
        // A String-param host op needs the shared `mem` even with no CONST string arg (the runtime-string
        // `_mem` copy loop writes into it) — so the core module imports `mem` on `set_needs_memory` too.
        .with_host_needs_memory(host::set_needs_memory(&host_imports))
        .with_extern_order(extern_order)
        // Reconcile the cross-edge import positions (computed 0-based at layout time) to their FINAL
        // `extern_order` positions: a peer-bound escaping effect occupies `0..cross_edge_delta` ahead of the
        // cross-edge block, so each cross-edge shifts up by that count. `0` (the common consumer, no coexisting
        // peer effect) → no-op, byte-identical.
        .with_cross_edge_import_shift(cross_edge_delta);
    // §2d increment 6: precompute the compound `start`-init now that the layout carries `static_bytes` (its
    // count fixes the compound global indices = `static_bytes.len() + k`), then attach both the table (for
    // selection's Tuple/Record routing) and the init (for `core_module_impl` to append to the START fn).
    let static_compound_init = select::build_static_compound_init(
        db,
        &static_compounds,
        layout.static_bytes.len(),
        &layout,
    )?;
    let layout = layout.with_static_compounds(static_compounds, static_compound_init);
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
    let mut core = if !extern_imports.is_empty() && !imports.is_empty() {
        // X5: a consumer binding a peer AND using the value-heap runtime (it inspects a compound handle
        // the peer returned) — the core imports peer ops from `"peer"` AND runtime ops from `"heap"`.
        serialize::core_module_with_extern_runtime(&funcs, &extern_imports, &imports, layout)
            .map_err(Reject::decline)?
    } else if !extern_imports.is_empty() {
        // X4b-3: an extern-ONLY program — the core imports its peer ops from `"peer"`.
        serialize::core_module_with_extern(&funcs, &extern_imports, layout)
            .map_err(Reject::decline)?
    } else if host_imports.is_empty() {
        serialize::core_module(&funcs, &imports, layout).map_err(Reject::decline)?
    } else {
        serialize::core_module_with_host(&funcs, &imports, &host_imports, layout)
            .map_err(Reject::decline)?
    };

    // DEBUG (Mode E, D2/D3): append the `.debug_*` DWARF custom sections to the embedded core module, so
    // a debugger can STEP through Cadenza source and inspect scalar locals. One subprogram DIE per emitted
    // function, its code-offset range from `code_ranges` (D1b); its line program carries a row per
    // distinct source POSITION the body visits (per-construct, from the `stmt_lines` markers — a
    // single-construct body falls back to one function-entry row), and its scalar params/`let`-locals /
    // match-binder scopes become `DW_TAG_formal_parameter`/`variable`/`lexical_block` children (D3), all
    // keyed off the `spans` side-table (D1a). Inert + strippable — appended after the executed sections, so
    // `debug = None` is byte-identical to today and `wasm-tools strip` recovers it. (`name` rode in
    // `core_module` historically; `append_debug_sections` now emits BOTH `name` and `.debug_*` from one place.)
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
    let is_provider = db.component_name.is_some();

    // PLAIN-EXPORT ENTRY-PARAM path (entry-param declines slice 1): a bare exported def (no imposed WIT
    // world, no host effect) whose parameter is a memory-bearing `String`/`Bytes` crosses as `string`/
    // `list<u8>` with a guest LIFT WRAPPER — the host→guest MIRROR of the host-op String/Bytes arg marshal.
    // Off this path (a compound param beyond String/Bytes, a compound result, multiple exports, a host
    // effect) `try_bare_entry_param_component` returns `None` and the export falls through to the existing
    // boundary loop, which declines a memory-bearing param honestly (no regression).
    if !is_provider
        && db.wit_world.is_none()
        && host_imports.is_empty()
        && extern_imports.is_empty()
        && layout.exports.iter().any(|e| {
            e.params.iter().any(|(_, t)| {
                // A memory-bearing leaf (String/Bytes/list) OR a two-variant sum (option/result) param — the
                // cheap pre-filter. A non-option `Sum`/`Nominal` still declines INSIDE (ty_natural_wit → None),
                // so widening the filter to sums is safe (it just gives the entry path a chance to classify).
                matches!(
                    t,
                    crate::ty::Ty::String
                        | crate::ty::Ty::Bytes
                        | crate::ty::Ty::List(_)
                        | crate::ty::Ty::Sum { .. }
                        | crate::ty::Ty::Nominal { .. }
                )
            })
        })
        && let Some(result) = try_bare_entry_param_component(db, layout, &funcs, &imports)
    {
        return result;
    }

    let mut boundary: Vec<BoundaryExport> = Vec::new();
    for e in &layout.exports {
        // A PROVIDER export whose result is a runtime-owned COMPOUND crosses to a PEER as its opaque `u32`
        // HANDLE over the shared runtime (X5c) — NOT the host resource-escape (skipped above for a
        // provider) and NOT a decline. Its parameters likewise: a scalar by its scalar rep, a compound by
        // its handle. `extern_abi_val_type` unifies the two; `Unit` params are elided.
        if is_provider && let Some(v) = host::extern_abi_val_type(&e.result) {
            let mut params = Vec::new();
            for (_, ty) in &e.params {
                if matches!(ty, crate::ty::Ty::Unit) {
                    continue;
                }
                let av = host::extern_abi_val_type(ty).ok_or_else(|| {
                    Reject::decline(format!(
                        "a provider parameter `{}` has no cross-component boundary representation",
                        ty.render_name(&db.name_ctx())
                    ))
                })?;
                params.push(av.comp_byte());
            }
            boundary.push(BoundaryExport {
                name: e.name.clone(),
                params,
                result: crate::backend::wasm::envelope::BoundaryResult::Primitive(v.comp_byte()),
            });
            continue;
        }
        // The export's RESULT crosses as a `BoundaryResult`: unit → None, a scalar → its primitive
        // byte. A HEAP value (a compound OR a String/list/bytes/map/set/symbol/quantity/newtype — every
        // `crosses_as_resource_escape` type) does not cross on THIS multi-export path — the single nullary
        // export case took the resource-escape shape above; one reaching here declines. Diagnose the ACTUAL
        // trigger, using the context known here — NOT the generic `export_result` message, which blames the
        // TYPE ("type `String` has no component boundary representation") when the real constraint is ARITY:
        // a String DOES cross fine as the sole export, so a multi-export/parameterized instance must say so.
        // (Keyed on `crosses_as_resource_escape` — the SAME predicate the gate uses — so the two never
        // drift and every escape-capable type gets the arity diagnosis, not just Tuple/Record/Sum. AND
        // gated on `export_result_valtype` DECLINING it: a nominal-over-SCALAR or a quantity-over-scalar
        // is a `crosses_as_resource_escape` type but ERASES to a scalar boundary valtype, so it crosses
        // fine on this multi-export path — `export_result_valtype` returns `Ok(Some)` for it, and it must
        // NOT be diagnosed as an arity decline. Only a type with NO scalar valtype reaches the arity case.)
        // A DIVERGING export — its body provably traps (`body_diverges`: a bare `(trap …)`, a zero-arm
        // match on a `Never` scrutinee, a call to such a function, OR a trap reached THROUGH an
        // effect-statement sequence / a `let` — the `(host (log) (do (log.emit "m") (trap …)))` shape a
        // unit-test failure path takes) — has a `Never` result type (a fresh var / `Any`) with no boundary
        // representation, but NO value ever crosses: the guest traps. Cross it as a UNIT (no-result)
        // export — the core function is already emitted 0-result (`select_function` maps a diverging `ret`
        // to `Ty::Unit` via the SAME `body_diverges`), and the host observes the trap. Checked BEFORE the
        // escape/valtype declines so a diverging `Any`/`Var` result is not misdiagnosed as an
        // undetermined-type fault.
        if serialize::export_result_valtype(&e.result, &db.name_ctx()).is_err()
            && crate::backend::common::diverge::body_diverges(db, e.body)
        {
            boundary.push(BoundaryExport {
                name: e.name.clone(),
                params: {
                    let mut params = Vec::new();
                    for (_, ty) in &e.params {
                        let vt = serialize::export_result_valtype(ty, &db.name_ctx())
                            .map_err(Reject::decline)?
                            .ok_or_else(|| {
                                Reject::decline(format!(
                                    "a diverging export's parameter `{}` has no boundary representation",
                                    ty.render_name(&db.name_ctx())
                                ))
                            })?;
                        params.push(vt);
                    }
                    params
                },
                result: crate::backend::wasm::envelope::BoundaryResult::None,
            });
            continue;
        }
        if crosses_as_resource_escape(&e.result)
            && serialize::export_result_valtype(&e.result, &db.name_ctx()).is_err()
        {
            // AMBIGUOUS TYPE first — a result whose payload/element type is an UNRESOLVED variable (a bare
            // `(None)` : `Option ?0`, an empty `(list)` : `List ?0`) has no defined serialization
            // REGARDLESS of export shape. A single NULLARY export with an unresolved payload reaches
            // here (the escape guard above tried and its value-form template returned `None`), so it must
            // NOT be diagnosed as an export-shape problem — the shape is fine; the TYPE is undetermined.
            // Report a type error naming the annotation fix (CDZ0203, the type-determination fault code),
            // NOT the parameterized/multi-export message. `e.params.is_empty()` distinguishes it from a
            // parameterized export (whose free var, if any, would still be a shape issue at this stage).
            //
            // This is the escape-with-an-unconstrained-type-variable rejection: a value that escapes to the
            // host whose type has a variable no use constrains (a bare `None`, an empty list) is rejected
            // here rather than crossing with an invented type, so a serialized value's type header is always
            // fully determined — the fix is an annotation that determines the variable, not an export-shape
            // change. A CONSUMED such value would have constrained the variable and never reach this guard.
            //= spec/capabilities/type-system.md#inference-is-principal-type-inference-by-unification
            //# A value that escapes to the host whose type contains a type variable no use constrains MUST be rejected at compile time with the type-determination fault code, rather than crossing the boundary with an invented type, so that a serialized value's type header is always fully determined. A bare `None` returned as the program result (type `Option ?`, the payload free), an `Ok` whose `Err` parameter is never constructed, or an empty list indexed to `None` (element free) is rejected for its unresolved type — the fix is an annotation that determines the variable — not for its export shape. A CONSUMED such value (matched, or passed to a typed parameter) constrains the variable and type-checks without annotation; the ambiguity bites only at an unannotated escape.
            // `has_undetermined_escape_component` (not bare `has_free_var`) so an empty collection whose
            // element grounded to `Ty::Any` (`(list)` → `(List Any)`) gets THIS coded CDZ0203 "annotate it"
            // — the same undetermined-escape fault as a free-`Var` `(None)`, just a different grounding —
            // rather than falling through to the misleading uncoded "value-form walker loops to a runtime
            // depth" decline below (which describes a genuine recursive-collection limitation, not this
            // undetermined-type one). Matches the `cdz check` twin in `compile.rs`.
            if e.result.has_undetermined_escape_component() && e.params.is_empty() && !multi_export
            {
                return Err(Reject::coded(
                    crate::diag::Code::TypeMismatch,
                    format!(
                        "the result type `{}` is not fully determined — annotate it \
                         (e.g. `(: <expr> (Option Int64))`) so its value has a defined form",
                        e.result.render_name(&db.name_ctx())
                    ),
                ));
            }
            // Each of the four whys below is a DISTINCT decline family that happens to share the
            // `returning a <T> from `<name>`: <reason>` shape. They are constructed as SEPARATE `Reject`s (a
            // shared `prefix` + a per-branch reason) rather than one shared `Reject::unsupported(format!(…{why}))`,
            // so a catalogued family can later carry its OWN `DeclineId` without mis-tagging its siblings — the
            // recursive-sum / runtime-collection walker branch is the `WasmValueFormWalkerRecursive` decline
            // (v-deferral-declines tags it now that this shared block is split; the other three — multi-export
            // arity, compound-param, parameterized-heap-return — are distinct reasons that would mis-tag if they
            // shared one `Reject`). The message text is BYTE-IDENTICAL to the prior shared `format!` (so any
            // reason-matched decline corpus case is unaffected) — this is purely a construction split.
            let prefix = format!(
                "returning a {} from `{}`",
                e.result.render_name(&db.name_ctx()),
                e.name
            );
            if multi_export {
                // THE ARITY constraint — the real reason a compound/heap export declines here. Names it as
                // such (a compound crosses as the SOLE export), NOT "the type has no boundary
                // representation" (false — it crosses fine alone via the resource escape).
                return Err(Reject::unsupported(format!(
                    "{prefix}: a heap value (a compound, string, or collection) crosses the host boundary only as the program's SINGLE export; this program has multiple exports (make it the only export, or return a scalar)"
                )));
            } else if !e.params.is_empty()
                && e.params.iter().any(|(_, ty)| {
                    !matches!(ty, crate::ty::Ty::Unit)
                        && !matches!(
                            serialize::export_result_valtype(ty, &db.name_ctx()),
                            Ok(Some(_))
                        )
                })
            {
                // A single PARAMETERIZED export whose heap result reached here — the resource escape now
                // FORWARDS scalar params (`make(a…) -> own<t>`), so a scalar-param heap return crosses. This
                // arm fires only when a param ACTUALLY has NO scalar boundary type (a compound/closure param),
                // which `make` cannot yet forward — that widening is a later increment. (Guarded on a param
                // genuinely lacking a scalar valtype so a scalar-param heap return whose RESULT is the real
                // constraint is NOT misdiagnosed as a param fault — see the result-constraint arm below.)
                return Err(Reject::unsupported(format!(
                    "{prefix}: a heap value escapes to the host as a resource with SCALAR parameters only; this export has a parameter with no scalar boundary type (a compound-parameter heap return is not supported)"
                )));
            } else if !e.params.is_empty() {
                // A single PARAMETERIZED export whose params are ALL scalar (an Int64, say) but whose heap
                // RESULT reached here: the param is fine — the RESULT is the constraint. Its value-form is
                // emitted only for a NULLARY (constant) export (the constant-bake path); a parameterized one
                // needs a runtime value-encode render, which this heap type does not yet have (e.g. a Symbol:
                // the runtime has no `Shape::Sym` and renders it as a String, not its canonical `(Symbol.of …)`
                // form the constant bake produces — so admitting it would cross a NON-canonical value). Name
                // the RESULT truthfully rather than blaming the scalar param.
                return Err(Reject::unsupported(format!(
                    "{prefix}: a parameterized export cannot return this heap type — its value form is emitted only for a nullary (constant) export, and a runtime value-encode render for it is not available (the scalar parameters are fine)"
                )));
            } else {
                // A single NULLARY export whose heap result reached here — the resource-escape path above
                // TRIED and its value-form template was `None`: the result has no runtime value form yet.
                // This is the RECURSIVE-sum / dynamic-shape / runtime-collection case (a self-referential
                // sum, or a runtime-built list/map/set built at runtime has an UNBOUNDED static shape, so
                // the `encode()` walker would need to LOOP to a runtime-determined depth — the analogue of
                // the runtime-`Bytes` looping walker, a later increment). The honest reason is the missing
                // walker; consuming such a value to a scalar already works, only rendering it as the
                // boundary result is deferred. This is the `WasmValueFormWalkerRecursive` decline family —
                // now that the shared-`Reject` block is split, this branch is an independent site
                // v-deferral-declines can tag with `DeclineId::WasmValueFormWalkerRecursive` (their catalog +
                // drift-check ownership) without touching its three siblings.
                return Err(Reject::declined(
                    crate::diag::DeclineId::WasmValueFormWalkerRecursive,
                    format!(
                        "{prefix}: rendering this value as the host result needs a value-form walker that loops to a runtime-determined depth (a recursive-sum / runtime-collection result); folding it to a scalar is supported"
                    ),
                ));
            }
        }
        let result =
            serialize::export_result(&e.result, &db.name_ctx()).map_err(Reject::decline)?;
        // Each parameter's COMPONENT-boundary valtype (distinct from the core valtype — a signed 64
        // integer is `s64` at the boundary, `i64` in the core). A parameter is a scalar (a `list<u8>`
        // INPUT is not yet a surface type), so its faithful primitive byte is required.
        let mut params = Vec::new();
        for (_, ty) in &e.params {
            // A scalar param crosses as its component primitive. A compound/heap param (record/tuple/sum/
            // list/string/…) has NO scalar boundary valtype and declines here — but it is a PARAMETER, not a
            // result, so do NOT surface `export_result_valtype`'s result-phrased message ("returning a … on
            // the multi-export boundary", doubly wrong for a param). Name the PARAM constraint truthfully.
            // (A memory-bearing String/Bytes/list or an option param on a SINGLE export is lifted by the
            // entry-param wrapper above; a record/tuple/other compound entry param is a later slice.)
            let vt = match serialize::export_result_valtype(ty, &db.name_ctx()) {
                Ok(Some(vt)) => vt,
                _ => {
                    return Err(Reject::unsupported(format!(
                        "parameter `{}` of `{}` has no scalar boundary representation — a non-scalar entry \
                         parameter is not supported on this export path",
                        ty.render_name(&db.name_ctx()),
                        e.name
                    )));
                }
            };
            params.push(vt);
        }
        boundary.push(BoundaryExport {
            name: e.name.clone(),
            params,
            result,
        });
    }

    // MAX-FLAT-PARAMS GUARD: a function whose params flatten to MORE than the canonical-ABI limit (16 core
    // values) must pass them MEMORY-INDIRECT — the caller stores the params to a linear-memory area and
    // passes a single pointer, which the lift reads back. This emit writes the FLAT form regardless, so past
    // 16 it produces an INVALID component (the worst outcome: a compile-success-only check reads a malformed
    // artifact as green). Until the indirect convention is emitted, DECLINE honestly at >16 rather than
    // emitting garbage. Each boundary param here is one scalar/handle = one flattened core value, so
    // `params.len()` IS the flattened count. (Rust targets have no flat limit and compile these fine.)
    for be in &boundary {
        if be.params.len() > crate::backend::wasm::wit_ctype::MAX_FLAT_PARAMS {
            return Err(Reject::unsupported(format!(
                "export `{}` has {} boundary parameters; the canonical ABI passes more than {} flat \
                 parameters memory-indirect, which this backend does not support — a >{}-parameter export \
                 is declined rather than emitting an invalid component",
                be.name,
                be.params.len(),
                crate::backend::wasm::wit_ctype::MAX_FLAT_PARAMS,
                crate::backend::wasm::wit_ctype::MAX_FLAT_PARAMS,
            )));
        }
    }

    // §3c COMPILER-PLATFORM SEPARATION — a provider export member the TARGET WIT WORLD declares as a
    // `list<u8>`-in/out boundary crosses as CANONICAL VALUE-FORM bytes. This takes PRECEDENCE over the
    // host-import / plain-provider paths below: `emit_bytes_provider_member` handles BOTH a pure reducer AND
    // a HOST-FUSED one (its body calls a host import like `kv`) — the declared WORLD, not the presence of a
    // host import, decides the bytes shape. Absent a world declaring such a member, this falls through to
    // the normal host/plain provider shapes (byte-identical for every program that targets no such world).
    if let Some(iface) = db.component_name.clone()
        && let Some(world_bytes) = db.wit_world.clone()
        && let Some(def) = world_bytes_crossing_export(layout, &world_bytes)
    {
        return emit_bytes_provider_member(db, layout, def, &iface, spans);
    }

    // §3c GENERAL WIT-BINDINGS — a TARGET WIT WORLD declaring an export INTERFACE whose funcs are all
    // SCALAR (no `record`/`variant`/`list<u8>` boundary yet) emits the guest as a component INSTANCE that
    // exports the interface (`assemble_typed_interface`). This is the no-wrapper subset of general
    // WIT-bindings emission: a scalar def's compiled core func lifts DIRECTLY (its `(i32)->i32` core sig is
    // the flattened boundary sig), so the interface funcs alias the existing core exports with no marshalling
    // body. A member carrying a record/variant/`list<u8>` still declines here (returns `None`) until the
    // lift/lower wrapper-body slice. Additive: every world that is not a scalar interface-export is `None`,
    // so it falls through to the shapes below exactly as before.
    if let Some(iface) = db.component_name.clone()
        && let Some(world_bytes) = db.wit_world.clone()
        && let Some(typed) = scalar_interface_export(layout, &world_bytes, &iface)
    {
        return Ok(envelope::assemble_typed_interface(&core, &typed));
    }

    // §3c GENERAL WIT-BINDINGS (W4c-b) — a typed interface-export world with a RECORD-param member: emit a
    // boundary WRAPPER per member (build the record from the flattened fields, call the def) via
    // `core_module_with_wrappers`, and export the interface instance. The wrapper's rebuild ops were added
    // to the import set above. Still the no-spill subset (scalar-field record params, scalar/unit results);
    // a `list<u8>`/record result declines here until the memory + result-lower wrapper slice.
    // A compound host RESULT makes the wrapper core IMPORT `"mem"`.`"cabi_realloc"` (the shared allocator, so
    // the host-op lower has a Realloc option at lower-time), which shifts the wrapper core's func indices +1.
    // The wrappers' `def_abs` (computed inside `record_interface_export` from `layout.abs`) AND the emitted
    // core (`core_module_with_wrappers`) must BOTH see that shifted `import_base`, so bump it BEFORE building
    // the wrappers. Byte-identical when there is no compound host result.
    let typed_needs_realloc = host_imports.iter().any(|h| h.spilled_result.is_some());
    let typed_realloc_layout;
    let typed_layout = if typed_needs_realloc {
        typed_realloc_layout = layout.with_import_base(layout.import_base + 1);
        &typed_realloc_layout
    } else {
        layout
    };
    if let Some(iface) = db.component_name.clone()
        && let Some(world_bytes) = db.wit_world.clone()
        && let Some((wrappers, typed)) =
            record_interface_export(db, typed_layout, &world_bytes, &iface)
    {
        let import_name = runtime_import_name();
        // A typed guest that ALSO performs a world import (`identity.id`, `state.get`, …) composes the host
        // effect interface alongside the runtime + the typed export (W4c-b-iii, the generic world-import call
        // surface). No host import → the runtime-only shape (byte-identical to before).
        if host_imports.is_empty() {
            let wrapped_core = serialize::core_module_with_wrappers(
                &funcs,
                &imports,
                &[],
                &wrappers,
                typed_layout,
            )
            .map_err(Reject::decline)?;
            return Ok(envelope::assemble_typed_interface_with_runtime(
                &wrapped_core,
                &typed,
                &imports,
                &import_name,
            ));
        }
        // Group the host imports by INTERFACE (effect), preserving first-seen order — a reducer performing ops
        // from N interfaces (graph + deliver, the default-handler guest) emits N imported component
        // instance-types. The core side stays FLAT: all ops bind under one `"host"` module by name (group
        // boundaries are invisible to it), so `core_module_with_wrappers` takes the flat `host_imports`.
        let mut group_order: Vec<String> = Vec::new();
        let mut grouped: std::collections::HashMap<String, Vec<host::HostImport>> =
            std::collections::HashMap::new();
        for hi in &host_imports {
            if !grouped.contains_key(&hi.effect) {
                group_order.push(hi.effect.clone());
            }
            grouped
                .entry(hi.effect.clone())
                .or_default()
                .push(hi.clone());
        }
        // Cross-interface op-name COLLISION: the merged `"host"` core instance exports each op BY NAME, so two
        // interfaces sharing an op name would collide. `collect_host_imports` self-dedups by (effect, op), so a
        // duplicate name here is a genuine cross-interface clash — decline (the interface-qualified host binding
        // is a later increment). Single-interface guests never trip this.
        {
            let mut seen = std::collections::HashSet::new();
            if host_imports.iter().any(|hi| !seen.insert(hi.op.clone())) {
                return Err(Reject::unsupported(
                    "two host ops share a name across the reducer's interfaces; the interface-qualified host \
                     binding is not supported",
                ));
            }
        }
        let mut groups = Vec::with_capacity(group_order.len());
        for effect in &group_order {
            groups.push(build_host_group(
                db,
                &world_bytes,
                effect,
                &grouped[effect],
            )?);
        }
        // A host op that needs LINEAR MEMORY — a `list<u8>`/`string` PARAM, or a compound RESULT whose spilled
        // return the guest lifts — lowers with a Memory canon option, so the SHARED `"mem"` module shape. The
        // memory + realloc are COMPONENT-WIDE (one shared memory across all interfaces), so compute over ALL ops.
        let host_needs_memory = host::set_needs_memory(&host_imports)
            || host_imports.iter().any(|h| h.spilled_result.is_some());
        let needs_realloc = typed_needs_realloc;
        // `typed_layout` already carries the +1 import shift when `typed_needs_realloc`, so the def/self-call
        // indices line up with the emitted core.
        let wrapped_core = serialize::core_module_with_wrappers(
            &funcs,
            &imports,
            &host_imports,
            &wrappers,
            typed_layout,
        )
        .map_err(Reject::decline)?;
        // A memory-needing host set takes the SHARED-`"mem"` shape; a memoryless one the defined-memory shape.
        // Both compose the N host interfaces + runtime + typed export.
        return Ok(if host_needs_memory {
            envelope::assemble_typed_interface_with_host_runtime_mem(
                &wrapped_core,
                &typed,
                &groups,
                &imports,
                &import_name,
                needs_realloc,
            )
        } else {
            envelope::assemble_typed_interface_with_host_runtime(
                &wrapped_core,
                &typed,
                &groups,
                &imports,
                &import_name,
            )
        });
    }

    // DECLINE-DON'T-MISCOMPILE: the world declares a TYPED record-param export interface and a component
    // name is set, yet `record_interface_export` did NOT fire above — so some member the exported interface
    // declares has no matching guest def (a PARTIAL guest: the program defines only a subset of the
    // interface's members), or a member's shape is unsupported. Without this, the program silently falls
    // through to the raw heap-handle export (`u32 -> u32`), a component the boundary cannot bind. A component
    // MUST export every member of the interface it exports, so decline clearly. Purely world/WIT-shape-driven
    // — this is generic over ANY declared export interface, not any particular interface or member set.
    if db.component_name.is_some()
        && let Some(world_bytes) = db.wit_world.clone()
        && world_has_typed_record_export(&world_bytes)
    {
        return Err(Reject::decline(
            "the program does not fully implement the world's typed export interface: a component that \
             exports an interface must define every member that interface declares, each with a matching \
             definition of the right shape; this program defines only some of them, so it cannot cross the \
             typed interface-instance boundary",
        ));
    }

    // A HOST-delegating program takes the host-import envelope shape (E2h-2): the delegated effect is a
    // component INTERFACE, its operations imported funcs the boundary resolves. This increment delegates a
    // SINGLE effect (every host import shares one effect name); a program delegating two distinct effects
    // declines (the multi-interface shape composes in a later increment).
    if !host_imports.is_empty() {
        let iface = host_imports[0].effect.clone();
        if host_imports.iter().any(|h| h.effect != iface) {
            return Err(Reject::unsupported(
                "delegating more than one host effect is not supported (one interface per envelope)",
            ));
        }
        // This pure host-delegating path (`assemble_host_runtime`/`assemble_host`) composes only a
        // scalar/unit host RESULT — a COMPOUND host result (`option<list<u8>>`/`list<tuple<…>>`/bare
        // `list<u8>`) is lifted only on the bytes-provider path OR the typed interface-instance host path
        // (both handled earlier in the dispatch). If such a result reaches HERE, decline rather than
        // mis-emit (the host instance-type would omit the `(list u8)` type its functype references).
        if host_imports.iter().any(|h| h.spilled_result.is_some()) {
            return Err(Reject::decline(
                "a compound host RESULT (option<list<u8>>/list<tuple>/list<u8>) is emitted only on the \
                 bytes-provider or typed interface-instance path, not the plain host-delegating envelope",
            ));
        }
        let host_fns: Vec<envelope::HostFn> = host_imports
            .iter()
            .map(|h| envelope::HostFn {
                op: h.op.clone(),
                comp_functype: host_op_comp_functype(h, 0, 0, &[], None),
                has_list_param: h.params.iter().any(|p| matches!(p, host::HostParam::Bytes)),
                core_functype: Vec::new(), // unused by the envelope (the core module builds its own)
            })
            .collect();
        // A program that ALSO uses the value-heap runtime (a host op result fed into a runtime collection
        // op — `imports` non-empty) composes BOTH imported interfaces: it imports the effect (as `"host"`)
        // AND the runtime (as its versioned name, `"heap"`), aliases + lowers both op sets, and instantiates
        // the program with both bound. A STRING-param host op additionally threads the shared-memory core
        // module (`assemble_host_runtime_mem`); a scalar/unit host set takes the memoryless
        // `assemble_host_runtime`. The core module already composed both import spaces
        // (`core_module_with_host` above).
        if !imports.is_empty() {
            let import_name = runtime_import_name();
            return Ok(if host::set_needs_memory(&host_imports) {
                envelope::assemble_host_runtime_mem(
                    &core,
                    &boundary,
                    &iface,
                    &host_fns,
                    &imports,
                    &import_name,
                )
            } else {
                envelope::assemble_host_runtime(
                    &core,
                    &boundary,
                    &iface,
                    &host_fns,
                    &imports,
                    &import_name,
                )
            });
        }
        // A host op with a STRING parameter needs the shared-memory shape (the `(ptr,len)` a `string`
        // lowers to is read from a memory both the program and the op's canon-lower bind); a scalar-only
        // host set takes the memoryless shape (byte-identical to E2h-2).
        return Ok(if host::set_needs_memory(&host_imports) {
            envelope::assemble_host_mem(&core, &boundary, &iface, &host_fns)
        } else {
            envelope::assemble_host(&core, &boundary, &iface, &host_fns)
        });
    }

    // A CROSS-COMPONENT consumer (X4b) takes the extern-import envelope shape: each peer interface is a
    // component INTERFACE, its operations imported funcs the composition resolves (bound under `"peer"`).
    // U9: a consumer may bind MORE THAN ONE distinct peer interface — each becomes its own imported
    // component instance, and each op aliases out of its interface's instance. The one `"peer"` core
    // instance exports every lowered op FLAT by name, so op names must be globally UNIQUE across the bound
    // interfaces; a cross-interface collision declines (the merged instance would export the name twice).
    if !extern_imports.is_empty() {
        // A MIDDLE-OF-CHAIN component is BOTH a consumer (binds a peer) AND a provider (compiled with a
        // `--component-name`): it imports its peer(s), computes, and PUBLISHES its own boundary as a named
        // interface instance for a downstream consumer (U11). `publish_iface` = `db.component_name`; when
        // set, the extern envelope bundles the consumer's lifted boundary funcs into an instance exported
        // under that name instead of exporting them top-level. `None` (a pure consumer) is byte-identical
        // to the X3/X5 top-level-export shape. This makes an A→B→C chain work: B binds A's interface and
        // publishes its own for C.
        let publish_iface = db.component_name.clone();
        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for e in &extern_imports {
            if let Some(&prior) = seen.get(e.op.as_str())
                && prior != e.interface.as_str()
            {
                return Err(Reject::decline(format!(
                    "peer operation `{}` is offered by two bound interfaces (`{}` and `{}`); an \
                     operation name must be unique across the peer interfaces a component binds — \
                     rename one so each peer op has a distinct name",
                    e.op, prior, e.interface
                )));
            }
            seen.insert(e.op.as_str(), e.interface.as_str());
        }
        // Each op's interface, PARALLEL to `extern_fns` (i.e. `extern_order`) — the envelope groups the ops
        // by interface into per-interface imported instances.
        let op_ifaces: Vec<&str> = extern_imports
            .iter()
            .map(|e| e.interface.as_str())
            .collect();
        let extern_fns: Vec<envelope::HostFn> = extern_imports
            .iter()
            .map(|e| envelope::HostFn {
                op: e.op.clone(),
                comp_functype: extern_op_comp_functype(e),
                core_functype: Vec::new(),
                has_list_param: false, // unused by the envelope (the core module builds its own)
            })
            .collect();
        // A consumer that ALSO uses the value-heap runtime (it inspects a compound `value` handle the
        // peer returned) composes BOTH imports — `assemble_extern_runtime` imports the peer(s) AND the
        // runtime (as `"heap"`), matching the core's dual import (X5). Otherwise the peer-only envelope (X3).
        if !imports.is_empty() {
            let import_name = runtime_import_name();
            return Ok(envelope::assemble_extern_runtime(
                &core,
                &boundary,
                &op_ifaces,
                &extern_fns,
                &imports,
                &import_name,
                publish_iface.as_deref(),
            ));
        }
        return Ok(envelope::assemble_extern(
            &core,
            &boundary,
            &op_ifaces,
            &extern_fns,
            publish_iface.as_deref(),
        ));
    }

    // IMPOSED-WORLD CONTRACT GUARD (breaker 2026-08-28): an explicit `wit_world` declares a CONCRETE typed
    // contract. If we've fallen through every typed export path (bytes-provider, scalar/record interface) to
    // the generic `u32`-handle PROVIDER path AND an export result is a COMPOUND (crosses as a handle), then
    // the declared typed result (enum/record/variant/tuple/…) could NOT be emitted — publishing a `u32`
    // handle would EXPORT A DIFFERENT TYPE than the world declares (a reordered-enum world silently became
    // `f: func(…) -> u32`). DECLINE loudly rather than silently mislabel the export. A component-name-ONLY
    // peer provider (X5c) has NO imposed world — `wit_world` is None — so its intended compound-as-handle
    // crossing is unaffected; and a member the typed paths DID emit returned early above, so only a genuinely
    // unemittable declared-typed compound reaches here.
    if db.wit_world.is_some()
        && let Some(bad) = layout.exports.iter().find(|e| {
            crate::backend::wasm::host::abi_val_type(&e.result).is_none()
                && crate::backend::wasm::host::extern_abi_val_type(&e.result).is_some()
        })
    {
        let ty_name = bad.result.render_name(&db.name_ctx());
        let ename = bad.name.clone();
        return Err(Reject::unsupported(format!(
            "the export `{ename}` returns `{ty_name}`, which the imposed world declares as a typed \
             component type, but this compiler cannot emit that typed export — rather than silently \
             cross it as an opaque `u32` handle (exporting a different type than the world declares), it \
             declines. (A record/enum/variant/tuple result under a declared world is supported; other \
             compound results are not supported.)"
        )));
    }

    // A PROVIDER (X4b/X5c) publishes its boundary exports as a named INTERFACE INSTANCE (the name the
    // `component-name` request supplied, stored on the Db) so a peer's `(effect …)` `(bind "iface")` binds
    // to it (the effects-unified surface, U2). A
    // scalar export crosses by value; a runtime COMPOUND export crosses as its `u32` handle (the boundary
    // loop above already set that). A provider whose exports build runtime values imports the runtime, so
    // it takes the provider+runtime envelope; a bare (no-runtime) provider the plain provider envelope.
    // (A `list<u8>`/Bytes result is the host resource-escape shape, never a peer handle — excluded.)
    if let Some(iface) = db.component_name.clone()
        && boundary
            .iter()
            .all(|e| e.result != envelope::BoundaryResult::Bytes)
    {
        // (The world-driven bytes gate ran EARLIER, before the host/plain provider branches — see above.)
        if imports.is_empty() {
            return Ok(envelope::assemble_provider(&core, &boundary, &iface));
        }
        let import_name = runtime_import_name();
        return Ok(envelope::assemble_provider_runtime(
            &core,
            &boundary,
            &imports,
            &import_name,
            &iface,
        ));
    }

    // The versioned runtime import name (`cadenza:runtime/heap@0.0.0+<hash>`) — the name the runtime
    // component is imported under, carrying the content-address suffix `cdz-run` resolves it by. Unused
    // when `imports` is empty (the bare envelope). Built here (not in `envelope`) so the envelope stays
    // ABI-agnostic; the ABI identity lives in the generated `runtime_abi` table.
    let import_name = runtime_import_name();
    Ok(envelope::assemble(&core, &boundary, &imports, &import_name))
}

/// Collect the DISTINCT fully-constant `Bytes` literals used anywhere in the reachable program (`order` =
/// the emission def indices), interned BY CONTENT in stable first-seen order — the build-once static-bytes
/// table (`DESIGN-static-data.md` §2d, increment 2). Each distinct byte sequence is materialized ONCE into a
/// module global (the build-once emit slice that follows); every use then reads that global (`global.get` +
/// dup) instead of re-`bytes-alloc`+`bytes-set`-ing the buffer per evaluation. Two literals with identical
/// content collapse to ONE entry (constant-CSE over the canonical byte value). A `Bytes.of` with any runtime
/// element contributes nothing — it is not constant, so `constant_bytes_value` returns `None` and it keeps
/// building per call.
///
/// Walks each def body over [`core_child_ids`](crate::backend::wasm::select::core_child_ids) — the complete
/// child enumerator the B2 sharing analysis uses — with an explicit work stack + a visited set, so a shared
/// (DAG) node is examined once and a deep or cyclic core graph cannot overflow the stack. The visited set is
/// shared across defs (a node id is unique in the arena, so a node reachable from two defs is walked once).
/// Returns the distinct payloads (index = the global slot each will occupy); the EMPTY vec for a program with
/// no constant bytes literal — then the build-once emit adds no GLOBAL/START section and the module is
/// byte-identical to before.
pub fn collect_static_bytes(db: &mut Db, order: &[usize]) -> Vec<Vec<u8>> {
    let mut distinct: Vec<Vec<u8>> = Vec::new();
    let mut seen_content: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let mut visited: std::collections::HashSet<crate::ast::StructId> =
        std::collections::HashSet::new();
    for &def in order {
        let Ok(body) = def_body(db, def) else {
            continue;
        };
        let mut stack = vec![body];
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            // A constant `Bytes` (`BytesOf`-of-consts / `ConstBytes`) OR a constant `String` (`ConstStr`)
            // is a flat-byte-payload hoist target: both build the identical UTF-8 leaf via
            // `bytes-alloc`+`bytes-set`, so both intern BY CONTENT into the one table (a String and a Bytes
            // with equal bytes share one immortal global). The two extractors are disjoint (a node is one or
            // the other), so `or_else` picks whichever applies.
            if let Some(payload) = crate::lower::constant_bytes_value(db, id)
                .or_else(|| crate::lower::constant_string_value(db, id))
                && seen_content.insert(payload.clone())
            {
                distinct.push(payload);
            }
            // Descend to children. A constant leaf's own children (a `BytesOf`'s `ConstInt`s) contribute
            // nothing, but descending them is harmless — so the walk stays uniform. Push in reverse so
            // children pop in `core_child_ids` order (a stable pre-order DFS).
            let children = crate::backend::wasm::select::core_child_ids(db, id);
            for &c in children.iter().rev() {
                stack.push(c);
            }
        }
    }
    distinct
}

/// Collect the constant `Tuple`/`Record` (and SMALL constant `List`, ≤32) ROOTS in the reachable program
/// that are per-node-immortal-markable (`lower::is_markable_constant_compound` /
/// `is_markable_constant_list`) — the §2d build-once static table. Each root is built ONCE as an
/// immortal tree in the module `start` init and read with `global.get` at each use, instead of the
/// per-evaluation `arr-alloc` + boxed `arr-set` (+ `vec-of-arr` for a list) the backend emits today. On
/// finding a markable root, collect it and do NOT descend — its whole subtree (nested markable
/// tuples/records/small-lists, boxed scalars, bytes/string leaves) is built inline as part of THIS root's
/// tree, not as separate globals. A small `List` joins the SAME table as tuples/records (all are "static
/// roots built once"): the routing (`try_emit_static_compound`) is keyed by node id, type-agnostic, so a
/// list needs no separate table — only the `start`-init builder (`emit_immortal_static`) distinguishes a
/// list (arr + `vec-of-arr`) from a tuple/record (arr). A `List > 32` is NOT markable (its `vec-of-arr`
/// builds RRB-trie internals with no compile-time handle to mark) so it never reaches here. Keyed by node
/// id (`StructId`), so two USES of one constant node share a global; two structurally-identical DISTINCT
/// literals get one each (structural interning is a later refinement — moving the build to `start` is the
/// win either way). Empty for a program with no markable constant root → no additions, byte-identical.
pub fn collect_static_compounds(db: &mut Db, order: &[usize]) -> Vec<crate::ast::StructId> {
    let mut roots: Vec<crate::ast::StructId> = Vec::new();
    let mut visited: std::collections::HashSet<crate::ast::StructId> =
        std::collections::HashSet::new();
    for &def in order {
        let Ok(body) = def_body(db, def) else {
            continue;
        };
        let mut stack = vec![body];
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            if crate::lower::is_markable_constant_compound(db, id)
                || crate::lower::is_markable_constant_list(db, id)
                || crate::lower::is_markable_constant_map(db, id)
                || crate::lower::is_markable_constant_set(db, id)
                || crate::lower::is_markable_constant_sum_nullary(db, id)
                || crate::lower::is_markable_constant_sum_payloaded(db, id)
            {
                roots.push(id);
                continue; // the whole subtree is built inline under this root — don't collect nested
            }
            let children = crate::backend::wasm::select::core_child_ids(db, id);
            for &c in children.iter().rev() {
                stack.push(c);
            }
        }
    }
    roots
}

/// Append the `.debug_*` DWARF custom sections to an already-serialized core module `core`, when a
/// `spans` side-table is present (a debug build). Shared by the ordinary multi-export path and the
/// runtime resource-escape path: both lay the user function bodies FIRST in the code section, so
/// `code_ranges(funcs)` gives their correct payload-relative offsets and `code_section_payload_base`
/// walks the real bytes for the base — regardless of how many synthesized funcs (a resource walker's
/// `make`/`t-encode`) trail them. Inert + strippable (appended after the executed sections); a `None`
/// `spans` or a core with no code section leaves `core` untouched (byte-identical to a no-debug build).
///
//= spec/capabilities/debug-information.md#stripping-debug-information-recovers-the-undecorated-artifact
//# Removing the debug information from an artifact that carries it MUST yield the byte-identical artifact the same source derives when debug information is excluded, so that debug information is a strippable addition to the runnable form rather than a modification of it.
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
            return Err(Reject::unsupported(
                "a DWARF sidecar for this sum-returning export is not supported (its variant \
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
        // The sidecar only runs for a NULLARY export (guarded at the top of this fn), so `make` forwards
        // no params — pass `&[]`, byte-identical to the nullary emitter's `make() -> own<t>`.
        let main_core = serialize::runtime_resource_core_module_form(
            &funcs,
            &imports,
            export_abs,
            serialize::EscapeForm::Sum(&tpl),
            &[],
            &[],
            &escape_lifted_table(&layout),
        )
        .map_err(Reject::decline)?;
        return Ok(Some(resource_dwarf_from_core(
            db, &layout, &funcs, &imports, &main_core, span_data,
        )?));
    } else if matches!(result, crate::ty::Ty::Bytes | crate::ty::Ty::String) {
        // Mode S mirrors Mode E: a runtime String escapes through the SAME bytes walker (its String value
        // form), so route it here too — else its sidecar would decline while the embedded component emits.
        let form = if matches!(result, crate::ty::Ty::String) {
            crate::lower::runtime_string_form(db)
        } else {
            crate::lower::runtime_bytes_form(db)
        };
        let Some(form) = form else {
            return Err(Reject::unsupported(
                "a DWARF sidecar for this runtime-Bytes/String export is not supported (no value form)",
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
        // MUST match Mode E's `emit_runtime_bytes_resource` method set (`[Len, ToBytes]` — the Bytes
        // resource carries `t-len` + `t-to-bytes`), or the sidecar's code offsets diverge from the
        // embedded component's (VM-1/VM-3).
        let main_core = serialize::runtime_resource_core_module_form_ex(
            &funcs,
            &imports,
            export_abs,
            serialize::EscapeForm::RuntimeBytes(&form),
            &[
                serialize::CoreMethod::Len,
                serialize::CoreMethod::IsEmpty,
                serialize::CoreMethod::ToBytes,
            ],
            &[], // nullary sidecar — `make` forwards no params
            &[], // …and no compound-param rebuild
            &escape_lifted_table(&layout),
        )
        .map_err(Reject::decline)?;
        return Ok(Some(resource_dwarf_from_core(
            db, &layout, &funcs, &imports, &main_core, span_data,
        )?));
    } else if let Some(tpl) = crate::lower::runtime_value_form_template(&result, &db.name_ctx()) {
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
        let main_core = serialize::runtime_resource_core_module(
            &funcs,
            &imports,
            export_abs,
            &tpl,
            &[],
            &[],
            &escape_lifted_table(&layout),
        )
        .map_err(Reject::decline)?;
        return Ok(Some(resource_dwarf_from_core(
            db, &layout, &funcs, &imports, &main_core, span_data,
        )?));
    }

    // A runtime compound with no value-form walker yet (e.g. a runtime list) — the embedded path
    // declines the same shape, so the sidecar does too rather than emit into a core it can't build.
    Err(Reject::unsupported(
        "a DWARF sidecar for this runtime compound-returning export is not supported (no value \
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
    // The default escape imports ONE resource type's 2 canon intrinsics (`resource.new`/`resource.rep`)
    // beyond the runtime ops. The distinct-signature path (G resource types) uses `resource_escape_build_n`.
    resource_escape_build_n(db, layout, 2, extra)
}

/// [`resource_escape_build`] parameterized by the number of RESOURCE-INTRINSIC core funcs the envelope
/// prepends beyond the runtime ops (`resource.new`/`resource.rep` per resource type). This fixes
/// `import_base` — the shift a defined func's index takes — BEFORE selection, so `abs`/`lifted_abs`/the
/// element segment (and any embedded inter-def call index) are correct for a core with `imports.len() +
/// intrinsics` imports. The single/multi-export/round-trip paths pass `intrinsics = 2` (one resource type);
/// distinct-signature passes `2*G`.
fn resource_escape_build_n(
    db: &mut Db,
    layout: &Layout,
    intrinsics: u32,
    extra: impl FnOnce(&mut std::collections::BTreeSet<&'static str>),
) -> Result<(Vec<&'static runtime_abi::RtOp>, Vec<SelectedFunc>, Layout), Reject> {
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    // Scan BOTH the top-level defs AND the lambda-lifted closure bodies — a lifted body is emitted as its
    // own function (see `append_lifted_bodies`), so an op used ONLY inside a closure must be imported too,
    // or its `CallImport` resolves to `u32::MAX` → invalid wasm (the escape-module closure-table bug).
    collect_module_used_ops(db, layout, &mut used)?;
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
    let layout = layout.with_import_base(imports.len() as u32 + intrinsics);
    let mut funcs: Vec<SelectedFunc> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        let params = match layout.export_plan(def) {
            Some(ep) => ep.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        funcs.push(select_function_of(db, body, &params, &layout, Some(def))?);
    }
    // Append the lambda-lifted closure bodies after the `order` defs (same as the main emit path + the
    // escape emitters) — a compound-returning program whose body dispatches a first-class closure via
    // `call_indirect` needs the lifted body present + the funcref table laid (the caller passes
    // `lifted_table` to the module assembler). Without this the escape module is missing both.
    append_lifted_bodies(db, &mut funcs, &layout)?;
    Ok((imports, funcs, layout))
}

/// The lambda-lifted closures' ABSOLUTE core-func indices, in table-slot order, for a resource-escape
/// module built over `layout` (whose `import_base` was fixed by `resource_escape_build_n`). Empty for a
/// closure-free program → the assembler lays no table. Passed as `form_ex2`'s `lifted_table`.
fn escape_lifted_table(layout: &Layout) -> Vec<u32> {
    (0..layout.lifted.len())
        .map(|slot| layout.lifted_abs(slot))
        .collect()
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
/// resource-escape shapes (a nullary compound export) are handled by [`resource_escape_dwarf`] FIRST —
/// it rebuilds the matching resource core (whose different layout Mode E attributes over identically) and
/// derives the sidecar from that, so a compound-returning export gets a sidecar whose offsets match the
/// embedded form. Only a runtime shape with no value-form walker yet (a runtime list) still declines.
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
    // `Some((box_op, extend))` when the export result is a SCALAR-ERASED value (a runtime `Qty` — it erases to
    // its bare inner scalar, not a heap handle): the `make` body must box that scalar (`box-int`) before
    // `resource-new` so the root rep is a real handle the walker can `get-int`. `extend` = `Some(signed)` when
    // the inner is a NARROW int (i32 core value) needing an i32→i64 widen before `box-int`; `None` for a
    // full-width Int64/UInt64. `None` (outer) for a compound result (already a handle). See
    // `EscapeForm::FlatScalar`.
    scalar_box: Option<(&'static str, Option<bool>)>,
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    // The `make`-forwarded params: a compound parameter is rebuilt in-guest from its flattened leaves,
    // so its rebuild ops join the import set (frozen below).
    let make_params = export_make_params(db, layout, export_def)?;

    // BUILD-ONCE STATIC COMPOUNDS (WIT static encoding, 2026-08-27): the escaping bodies (the export +
    // its call graph) may embed markable constant Tuple/Record/List/Map/Set literals — e.g. the inner
    // `(tuple 1 2)` of a compound RETURN. Collect them as build-once roots and precompute the START init
    // that builds each immortal, exactly as the ordinary `emit` path does — the resource-escape assembler
    // never ran this before, so those constants built MORTAL per-`make` (the imc/irb corpus family). The
    // collected roots are threaded onto the selection layout below (so the body's `Core::Tuple`/… arms emit
    // `global.get` via `try_emit_static_compound`) and their count + init are passed to the core builder to
    // emit the GLOBAL/START sections. Empty (no markable constant) → byte-identical to before.
    let static_compounds = collect_static_compounds(db, &layout.order);
    let static_compound_init = if static_compounds.is_empty() {
        Vec::new()
    } else {
        // byte_base 0: the resource module has NO static-bytes globals, so compound globals ARE `0..n`.
        select::build_static_compound_init(db, &static_compounds, 0, layout)?
    };

    // Ops the reachable bodies emit (construction: arr-alloc/arr-set/box-*), PLUS the ops the walker
    // `t-encode` calls (arr-get + get-int/get-bool per template leaf). The walker ops are added here
    // because they appear only in the synthesized encode body, not in any reachable Core.
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_module_used_ops(db, layout, &mut used)?;
    make_params.collect_rebuild_ops(&mut |op| {
        used.insert(op);
    });
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
    // A SCALAR-ERASED result (a runtime Qty) needs its box op (`box-int`) in the import set — the `make`
    // body calls it to box the erased scalar into the root heap cell before `resource-new`. (The i32→i64
    // extend, if any, is a core instruction, not an imported op.)
    if let Some((box_op, _extend)) = scalar_box {
        used.insert(box_op);
    }
    // The resource DTOR calls `drop` to release the escaped compound's rc handle on host-drop (or when
    // `encode` consumes the `own<t>`). `drop` appears only in the synthesized dtor, never in a reachable
    // Core, so add it here — it becomes one of the lowered ops, and the envelope threads it into the
    // separate `heap-dtor` instance the dtor imports.
    used.insert("drop");
    // The static-compound START init builds each immortal with `arr-alloc` + boxed `arr-set` (+ `vec-of-arr`
    // / `map-*` / `set-*` per kind), then `mark-immortal[-deep]`. When EVERY constant use is hoisted the
    // bodies no longer emit those ops, so `collect_module_used_ops` would omit them and the init's
    // `CallImport` would reference an undeclared import — force the full init op set when the table is
    // non-empty (mirrors the ordinary `emit` path). Idempotent; no-op when there are no static compounds.
    if !static_compounds.is_empty() {
        used.insert("arr-alloc");
        used.insert("arr-set");
        used.insert("box-int");
        used.insert("box-bool");
        used.insert("bytes-alloc");
        used.insert("bytes-set");
        used.insert("mark-immortal");
        used.insert("vec-of-arr");
        used.insert("mark-immortal-deep");
        used.insert("map-empty");
        used.insert("map-insert");
        used.insert("set-empty");
        used.insert("set-insert");
        // A hoisted NULLARY mixed-sum terminal builds via `sum-new(disc, IMM_UNIT)` in the init — force it.
        used.insert("sum-new");
        // A hoisted MAP/SET with a LIST key/element canonicalizes it (`value-canonicalize`) for CHAMP-slot
        // exactness; a rope String/Bytes key compacts (`bytes-compact`). Force both — else the init's
        // `CallImport` resolves out-of-range (invalid `function[N]`, the ikc1/itf2 regressor). Mirrors 585.
        used.insert("value-canonicalize");
        used.insert("bytes-compact");
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

    // PEER-IN-RESOURCE-ESCAPE (task #6): a peer-BOUND effect op reached by a reachable body (`main` RETURNS
    // the compound the peer produced) must have its extern import carried into the resource component —
    // exactly as the ordinary `emit` path does. Collect the host imports over the reachable bodies, split
    // the peer-bound ones (retargeted to their interface) into `extern_imports`, and — when present —
    // thread `extern_order` so `layout.extern_index` resolves + shift `import_base` past the `p` peer ops.
    // A single peer interface is supported (the fused envelope's scope); a compound arg to such an op, or a
    // second peer interface, or a HOST effect alongside, still declines below.
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
    // A HOST effect delegated from a resource-escaping entrypoint composes the host import space with the
    // resource escape (`envelope::assemble_host_runtime_resource`, the host-side mirror of the peer fusion
    // below). SCOPE this increment: SCALAR/unit host ops only (a STRING-param host op needs the shared-memory
    // `_mem` variant — a later increment; its `(ptr,len)` ABI is 2 core slots, which the scalar type section
    // here does not model). A host effect ALONGSIDE a peer effect is a further fusion (both import spaces at
    // once) — decline. Only the pure-host, scalar case is emitted here.
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
        // The host op set, laid FIRST in the core module (host imports `0..h`) — a `CallHostImport(i)`
        // resolves to core func `i`. Record `host_order` so `select` emits each `Core::HostCall` with its
        // raw index, shift `import_base` past the `h` host ops + `k` runtime ops + resource-new/rep, then
        // rebuild the core module with `leading_is_host = true` (host ops import from `"host"`) and dispatch
        // to `assemble_host_runtime_resource`. The host ops carry the SAME core functype as an extern op
        // (scalar-by-value), so they are threaded through the `extern_fns` slot of the core-module builder
        // (byte-identical for a scalar op; the module string is switched by `leading_is_host`).
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
            return Err(Reject::unsupported(
                "delegating more than one host effect from a resource-escaping entrypoint is not \
                 supported (one interface per envelope)",
            ));
        }
        let host_layout = layout
            .with_import_base(h + k + 2)
            .with_host_order(host_order)
            // Thread build-once static compounds onto the HOST-fused layout too, so a host-effecting body that
            // ALSO returns a markable constant compound (`main() = host H in (do (H.op) (tuple 1 2))`) emits
            // `global.get` for the constant instead of rebuilding it MORTAL per `make`. Empty → no-op.
            .with_static_compounds(static_compounds.clone(), static_compound_init.clone());
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
        let export_abs = host_layout
            .abs(export_def)
            .ok_or_else(|| Reject::decline("the escaping export is not in the emission order"))?;

        // The host ops as the core module's leading-import slot (ExternImport-shaped; byte-identical to a
        // host functype for a scalar op — see the `leading_is_host` review). A String param declined above.
        let host_as_extern = host_as_extern_for(&host_imports);
        let mut main_core = serialize::runtime_resource_core_module_form_ex2(
            &funcs,
            &imports,
            &host_as_extern,
            true, // leading ops are HOST — import from "host"
            export_abs,
            match scalar_box {
                Some((box_op, extend)) => serialize::EscapeForm::FlatScalar {
                    tpl,
                    box_op,
                    extend,
                },
                None => serialize::EscapeForm::Flat(tpl),
            },
            &[],
            &make_params.leaf_vts,
            &make_params.core_slots(),
            &escape_lifted_table(host_layout),
            static_compounds.len(),
            &static_compound_init,
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
    // The fused envelope now supports MULTIPLE distinct peer interfaces (the component groups the peer ops
    // by interface into g imported instances; the core module imports them all from one `"peer"` module).
    let p = extern_imports.len() as u32;
    let extern_order: Vec<(String, String)> = extern_imports
        .iter()
        .map(|e| (e.interface.clone(), e.op.clone()))
        .collect();

    // Defined funcs' absolute indices are shifted past the `p` peer ops + `k` runtime ops + the two
    // resource intrinsics (`resource-new`, `resource-rep`), so `import_base = p + k + 2`.
    let k = imports.len() as u32;
    let layout = layout
        .with_import_base(p + k + 2)
        .with_extern_order(extern_order)
        // Thread the build-once static compounds onto the selection layout so the body's `Core::Tuple`/… arms
        // emit `global.get idx` (`try_emit_static_compound`) matching the GLOBAL section the builder lays.
        .with_static_compounds(static_compounds.clone(), static_compound_init.clone());
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
    // Append the lambda-lifted closure bodies AFTER the `order` defs (mirror the main emit path,
    // `emit`'s ~553 loop): the escape module dispatches a first-class closure via `call_indirect`, so its
    // lifted body must be emitted here (at func idx `import_base + order.len() + slot`) AND the funcref
    // table/elem must be laid (in `form_ex2`, from `layout.lifted`). WITHOUT this the escape module had
    // neither → `func N: unknown table 0` at the `call_indirect` (v-core-opt's mixed-match-Option tail-loop
    // / filter-map bug).
    append_lifted_bodies(db, &mut funcs, layout)?;

    // The escaping export's absolute core-func index — `make` calls it to build the compound.
    let export_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the escaping export is not in the emission order"))?;

    let mut main_core = serialize::runtime_resource_core_module_form_ex2(
        &funcs,
        &imports,
        &extern_imports,
        false, // leading ops are PEER (extern), not host — import from "peer"
        export_abs,
        match scalar_box {
            Some((box_op, extend)) => serialize::EscapeForm::FlatScalar {
                tpl,
                box_op,
                extend,
            },
            None => serialize::EscapeForm::Flat(tpl),
        },
        &[],
        &make_params.leaf_vts,
        &make_params.core_slots(),
        &escape_lifted_table(layout),
        static_compounds.len(),
        &static_compound_init,
    )
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
    if extern_imports.is_empty() {
        // The ordinary (no-peer) runtime-resource escape — byte-identical to before.
        return Ok(envelope::assemble_runtime_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_params.boundary_slots(),
        ));
    }
    // The FUSED resource-escape × peer-extern envelope (task #6): the component imports the peer
    // interface(s) AND publishes the resource. `op_ifaces` runs parallel to `peer_fns` so the envelope
    // groups ops by their (possibly multiple) interfaces.
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
    // incidental decline (CDZ0401 "no home", or "not in the host-import set").
    //
    // The detector scans the LIFTED CLOSURE BODIES ONLY — the code that crosses the boundary and runs
    // LATER, when the host invokes `call()`, outside the delegation frame. A `Core::HostCall` there is a
    // genuine escape (its effect has no home at the call). A HostCall in the EXPORT BODY PROPER is NOT an
    // escape: the export body is the `make` code, which runs at export-execution time WHILE the `(host …)`
    // delegation is still in dynamic scope — a build-time effect whose result the closure merely captures
    // as a plain value (`(host (ask) (let ((v (ask.ask))) (fn (x) (+ x v))))`) is discharged in scope, and
    // the returned closure is effect-free. So it must NOT be flagged — mirroring the intra-program
    // `(handle … (let ((v (E.get))) (fn (x) (+ x v))))`, which compiles because the fold reduces its
    // effect away. Scanning the whole export body over-rejected exactly this build-time-delegated case
    // (it caught the `make`-time HostCall as if the closure had performed it).
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
    // A MAKE-TIME host call in the EXPORT BODY (not the lifted closure) — the build-time-delegated case
    // `(host (ask) (let ((v (ask.ask))) (fn (x) (+ x v))))`: `ask.ask` is discharged WHILE the delegation
    // is in scope and the closure captures only the plain result `v` (NOT the CDZ0406 escape — that scans
    // the lifted body). Collect these make-time host imports; the actual host-composition emit is routed
    // AFTER the boundary bytes (`arg_bytes`/`result_byte`/`ret_is_*`) are computed below.
    let mut host_imports: Vec<host::HostImport> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        host::collect_host_imports(db, body, &mut host_imports);
    }
    // DIRECT-CALL COMPOUND ARG (fixed-shape scalar tuple/record): a single closure arg that is a tuple/record
    // of aliased-width scalars crosses as a NATIVE component `tuple<…>` the canonical ABI flattens into scalar
    // core params; the core `call` rebuilds the cell from the flat fields (`TupleArgRebuild`). Detected here
    // so the scalar `arg_bytes` decline below doesn't reject it. SCOPE this increment: EXACTLY one such
    // compound arg, a scalar result, no build-time host effect (each a clean later widening). The SOLE-tuple
    // path (`fixed_shape_scalar_tuple_arg`) also accepts a NESTED fixed-shape compound FIELD, flattening its
    // leaves depth-first and rebuilding the sub-cell recursively (brick 3). Still `None` (falls to the
    // decline): a COLLECTION field, or the AMONG-SCALARS / multi-export path with a nested field (`tuple_field_abi`
    // stays scalar-field-only). `tuple_arg` = (full flattened field/scalar bytes, full flattened core vts,
    // prefix scalar bytes, suffix scalar bytes, rebuild). The SOLE-tuple case (one compound arg, no scalars)
    // has empty prefix/suffix and `base_param=1`; the AMONG-SCALARS case (one tuple + ≥1 scalar arg) carries
    // the prefix/suffix + a shifted `base_param`. Both flatten the tuple into native `tuple<…>` fields the
    // core `call` rebuilds.
    let tuple_arg: Option<CompoundArgBoundary> = if host_imports.is_empty() {
        if arg_tys.len() == 1 {
            fixed_shape_scalar_tuple_arg(&arg_tys[0])
                .map(|(fb, fv, rb)| (fb, fv, Vec::new(), Vec::new(), rb))
        } else {
            single_compound_among_scalars(arg_tys.as_slice())
        }
    } else {
        None
    };
    // A fixed-shape compound arg with a NESTED compound FIELD (a tuple/record containing a tuple/record) — the
    // SOLE arg OR among aliased-width scalars. `fixed_shape_scalar_tuple_arg`/`single_compound_among_scalars`
    // above return `None` (they reject a non-scalar field), so this catches it. The canonical ABI flattens the
    // nested tuple RECURSIVELY into leaf scalar core params; the core `call` rebuilds the nested cell via the
    // recursive `TupleArgRebuild`, and the envelope mints the inner `tuple<…>` types (`TupleFieldShape`). The
    // sole case has empty prefix/suffix + `base_param=1`; the among-scalars case carries prefix/suffix + a
    // shifted `base_param`.
    let nested_tuple: Option<NestedCompoundArgBoundary> =
        if host_imports.is_empty() && tuple_arg.is_none() {
            nested_sole_or_among_scalars(arg_tys.as_slice())
        } else {
            None
        };
    // TWO OR MORE fixed-shape tuple/record args (the N-compound-args direct-call path): each tuple crosses as
    // its own native `tuple<…>` (the canonical ABI flattens all into scalar core params), rebuilt in-guest
    // from its `TupleArgRebuild`. Detected only when neither single-tuple classifier fired (they require
    // exactly one compound). `multi_args` = (the ordered `ArgSlot` list, the full flattened core vts, one
    // rebuild per tuple). Scoped this increment: SINGLE-export, SCALAR result, no build-time host effect.
    let multi_args: Option<(
        Vec<crate::backend::wasm::envelope::ArgSlot>,
        Vec<crate::backend::wasm::lir::ValType>,
        Vec<crate::backend::wasm::serialize::TupleArgRebuild>,
    )> = if host_imports.is_empty() && tuple_arg.is_none() && nested_tuple.is_none() {
        multi_compound_args(arg_tys.as_slice())
    } else {
        None
    };
    // A SOLE `(Option scalar)`/`(Result scalar scalar)` closure ARG crosses as a native `option<payload>`/
    // `result<ok,err>` the canonical ABI flattens to `(disc: i32, payload)` core params; the core `call`
    // rebuilds the sum cell via `SumArgRebuild`, the envelope mints the boundary type via the returned
    // `ArgSlot`. Detected only when no tuple classifier fired. Scoped: the SOLE arg, scalar result, no
    // build-time host effect (each a clean later widening).
    let sum_arg: Option<(
        crate::backend::wasm::envelope::ArgSlot,
        Vec<crate::backend::wasm::lir::ValType>,
        crate::backend::wasm::serialize::SumArgRebuild,
    )> = if host_imports.is_empty()
        && tuple_arg.is_none()
        && nested_tuple.is_none()
        && multi_args.is_none()
        && arg_tys.len() == 1
    {
        fixed_shape_option_scalar_arg(db, &arg_tys[0])
            .or_else(|| fixed_shape_result_compound_arg(db, &arg_tys[0]))
    } else {
        None
    };
    // Boundary bytes (component valtypes) for the `call` method's ARGS — aliased scalar widths (a compound
    // closure arg on the direct-call path is handled by `tuple_arg` above, when it is a fixed-shape scalar
    // tuple/record; any other compound arg declines here — host→guest decode is not supported).
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
    // The RESULT: either a scalar (crosses by value) OR a BYTE-ROPE (`Bytes` OR `String`) which crosses as
    // `list<u8>` through linear memory — the compound-result path, reusing the value-escape's list machinery.
    // A `String` is a UTF-8 byte-rope handle representationally IDENTICAL to `Bytes` (same `bytes-*` store),
    // so its `call` copies the UTF-8 bytes out exactly as a `Bytes` result does — the host receives the raw
    // `list<u8>` (the encoded bytes, not a decoded string). Both peel a nominal first. Other compounds
    // (tuple/list/record) need the escape's `encode` walker + value-form framing (a later widening).
    let ret_is_bytes = matches!(
        ret_ty.strip_nominal(),
        crate::ty::Ty::Bytes | crate::ty::Ty::String
    );
    // A COMPOUND result (tuple/record/sum/list/map/set) that is NOT a byte-rope crosses `call` as `list<u8>`
    // carrying the canonical VALUE FORM — the host decodes + pretty-prints the typed `(: value T)` document.
    // Reuses the value-heap escape's `runtime_value_form_template` (the static template + runtime leaf holes)
    // + `encode_walk_body` walker, keyed on the CLOSURE'S RESULT handle instead of a resource rep. `None` if
    // the type has no value-form surface (a function/type-value) → falls through to the scalar decline.
    // Skip the compound path for a byte-rope (its own list<u8> path) or a scalar (crosses by value); only a
    // genuine compound (no scalar boundary byte) consults the value-form template.
    let ret_template = if ret_is_bytes || closure_boundary_byte(&ret_ty).is_some() {
        None
    } else {
        crate::lower::runtime_value_form_template(ret_ty.strip_nominal(), &db.name_ctx())
    };
    let ret_is_compound = ret_template.is_some();
    // A VARIABLE-LENGTH collection result (`List`/`Map`/`Set`) has no static template — it crosses as
    // `list<u8>` too, but the value form is rendered at run time by the runtime `value-encode(rep, desc)` op
    // walking the returned handle against a compiler-baked shape DESCRIPTOR (the recursive-sum escape's
    // approach C). `sum_shape_descriptor`'s List/Map/Set arm builds a parametric `Framed` descriptor.
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
    // On the SINGLE-export path a fixed-shape scalar tuple/record arg — SOLE or AMONG scalar args — composes
    // with EVERY result shape: the shared `emit_closure_call_args` helper threads the prefix scalars, the
    // rebuilt tuple, and the suffix scalars for the scalar AND the three list-result (`bytes`/value-form/
    // value-encode) cores alike, and the envelope emits the interleaved `call` functype. (The MULTI/MIXED/
    // DISTINCT-SIG among-scalars list-result paths remain a follow-on and decline in their own emit fns.)
    let result_byte = if ret_is_bytes || ret_is_compound || ret_is_collection {
        0 // unused by the list-returning paths; `call` returns list<u8>, not a scalar byte
    } else {
        closure_boundary_byte(&ret_ty)
            .ok_or_else(|| closure_boundary_reject("result", &ret_ty, &db.name_ctx()))?
    };
    // BRICK (d): a closure export whose build-time `make` code delegates a host effect (`host_imports`
    // non-empty, collected above) composes the host interface into the closure resource. This increment
    // handles a SCALAR closure result + a single scalar/unit host effect; other shapes decline cleanly.
    if !host_imports.is_empty() {
        if ret_is_bytes || ret_is_compound || ret_is_collection {
            return Err(Reject::unsupported(
                "a closure export that BOTH delegates a build-time host effect AND returns a \
                 byte-rope/compound/collection is not supported (the host-composed closure core supports \
                 a scalar result)",
            ));
        }
        let iface = host_imports[0].effect.clone();
        if host_imports.iter().any(|hi| hi.effect != iface) {
            return Err(Reject::unsupported(
                "a closure export delegating more than one host effect is not supported (one interface \
                 per closure envelope)",
            ));
        }
        if host::set_needs_memory(&host_imports) {
            return Err(Reject::unsupported(
                "a closure export delegating a host op with a string parameter is not supported (the \
                 shared-memory host shape and the closure resource envelope are not composed)",
            ));
        }
        return emit_closure_host_resource(
            db,
            layout,
            export_def,
            &host_imports,
            &iface,
            &arg_bytes,
            result_byte,
            &arg_tys,
            &ret_ty,
        );
    }
    // Core valtypes for the `call` method's args (used to build the core `call` signature). For a FIXED-SHAPE
    // tuple arg the canonical ABI FLATTENS the tuple into its scalar fields, so the core `call` receives the
    // FIELD valtypes (not one i32 handle) — the `call` body rebuilds the cell from them. Otherwise each arg's
    // own machine valtype (a scalar; a `Bytes` result is an i32 heap handle).
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = if let Some((_, all_vts, _, _, _)) =
        &tuple_arg
    {
        all_vts.clone()
    } else if let Some((_, leaf_vts, _, _, _, _)) = &nested_tuple {
        leaf_vts.clone() // the DEPTH-FIRST flattened leaf params of a nested tuple arg
    } else if let Some((_, all_vts, _)) = &multi_args {
        all_vts.clone() // the flattened leaves of EVERY tuple/scalar arg, in order (N-compound-args)
    } else if let Some((_, payload_vts, _)) = &sum_arg {
        // A sum arg flattens to `(disc: i32, <payload leaves…>)` core params — a scalar payload is ONE leaf, a
        // compound (Option-of-tuple) payload its recursively-flattened leaves. The shared `call` signature.
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
            valtype_of(t)
                .ok_or_else(|| Reject::decline("closure export param has no machine valtype"))
        })
        .collect::<Result<_, _>>()?;
    let make_param_bytes: Vec<u8> = export_params
        .iter()
        .map(|(_, t)| {
            closure_boundary_byte(t)
                .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
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
    let mut lifted_ops: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();
    for &body in &lifted_bodies {
        select::collect_used_ops(db, body, &mut lifted_ops);
    }
    let (imports, mut funcs, layout) = resource_escape_build(db, layout, |used| {
        used.insert("arr-get");
        used.insert("get-int");
        used.insert("drop");
        // A `Bytes`-result `call` copies the closure's returned Bytes handle into linear memory.
        if ret_is_bytes {
            used.insert("bytes-len");
            used.insert("bytes-get");
        }
        // A COMPOUND-result `call` walks the returned handle to fill the value-form template — it reads
        // `get-bool` for a boolean leaf (int leaves already covered by `get-int`) and, for a nested compound,
        // `arr-get` (already covered). Import `get-bool` so a Bool leaf's hole fill resolves.
        if ret_is_compound {
            used.insert("get-bool");
        }
        // A VARIABLE-LENGTH collection result `call` renders the value form via `value-encode(rep, desc)`:
        // build the descriptor Bytes (`bytes-alloc`/`bytes-set`), encode, copy the doc out (`bytes-len`/
        // `bytes-get`). Same op set as the recursive-sum escape walker.
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
        // A DIRECT-CALL fixed-shape scalar tuple/record ARG: the `call` body rebuilds the flattened fields
        // into a heap cell (`emit_tuple_rebuild`: `arr-alloc`, then per field its box op + `arr-set`). Those
        // ops appear ONLY in the synthesized rebuild, not in any reachable body — so register EXACTLY what
        // the rebuild emits: `arr-alloc`/`arr-set` plus each field's box op. All-int tuples happened to work
        // because `box-int` is pulled in elsewhere, but a Bool field needs `box-bool`, a Float `box-float`/
        // `box-float32` — absent from the import index without this, so `emit_tuple_rebuild`'s `imp(bop)`
        // panicked ("rebuild op imported"). Keying off `field_box_ops` keeps imports consistent with the
        // codegen for every field-type mix.
        if let Some((_, _, _, _, rebuild)) = &tuple_arg {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for f in &rebuild.fields {
                f.collect_box_ops(&mut |bop| {
                    used.insert(bop);
                });
            }
        }
        // A NESTED tuple arg: the recursive rebuild also emits `arr-alloc`/`arr-set` (per cell level) + each
        // leaf's box op.
        if let Some((_, _, rebuild, _, _, _)) = &nested_tuple {
            used.insert("arr-alloc");
            used.insert("arr-set");
            for f in &rebuild.fields {
                f.collect_box_ops(&mut |bop| {
                    used.insert(bop);
                });
            }
        }
        // ≥2 tuple args (N-compound): each rebuilds its own cell — register every tuple's box ops (a Bool/Float
        // field would otherwise panic in `emit_tuple_rebuild` on `imp(bop)`, exactly as for the single-tuple arg).
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
        // A SOLE `(Option/Result scalar)` arg: the `call` rebuilds the sum cell via `sum-new` (branching on
        // disc), boxing each arm's payload with its box op (a Bool/Float payload needs `box-bool`/`box-float`).
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
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    // A fixed-shape tuple ARG threads through the LIST-result cores + envelope identically whether it is FLAT
    // (`tuple_arg`), NESTED (`nested_tuple`), or N-COMPOUND (`multi_args`): the core rebuilds each cell from a
    // `TupleArgRebuild` (a SLICE now — one per tuple), and the envelope mints a flat `tuple<…>` (from
    // `tuple_bytes`), a recursive nested one (from `tuple_shape`), OR N tuples (from the `ArgSlot` slot list).
    // `list_rebuilds` = every tuple's rebuild in arg order; `list_slots` is `Some` only for the N-compound
    // case (the scalar-tuple/nested cases keep the byte-identical `tuple_bytes`/`tuple_shape` mint).
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
    let list_tuple_bytes: Option<&[u8]> = tuple_arg.as_ref().map(|(fb, _, _, _, _)| fb.as_slice());
    let list_shape: Option<&[crate::backend::wasm::envelope::TupleFieldShape]> = nested_tuple
        .as_ref()
        .map(|(_, _, _, shape, _, _)| shape.as_slice());
    let (list_tpre, list_tsuf): (&[u8], &[u8]) = tuple_arg
        .as_ref()
        .map(|(_, _, pre, suf, _)| (pre.as_slice(), suf.as_slice()))
        .or_else(|| {
            // A nested arg may sit AMONG scalars too — carry its prefix/suffix (empty for a sole nested arg).
            nested_tuple
                .as_ref()
                .map(|(_, _, _, _, pre, suf)| (pre.as_slice(), suf.as_slice()))
        })
        .unwrap_or((&[], &[]));
    // N-COMPOUND-ARGS (≥2 fixed-shape tuple/record args) with a SCALAR result: each tuple crosses as its own
    // native `tuple<…>` (the canonical ABI flattens all into scalar core params); the core `call` rebuilds
    // every arg cell from its `TupleArgRebuild` (threaded as a slice — brick 3), and the envelope mints N
    // `tuple<…>` types via the `ArgSlot` model. A LIST result (byte-rope/compound/collection) over ≥2 tuple
    // args flows through the shared list-result routings below (which now also thread the slot model), so this
    // block fires only for the scalar-result case.
    if let Some((slots, _all_vts, rebuilds)) = &multi_args
        && !ret_is_bytes
        && !ret_is_compound
        && !ret_is_collection
    {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &[serialize::ClosureMake {
                export_name: "make".to_string(),
                export_abs,
                param_vts: make_param_vts.clone(),
            }],
            &[],
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false, // own<t> (single-use) — every rebuilt-arg cell drop is unconditional, so leak-free
            rebuilds,
            &[], // no sum arg (this is a tuple/multi-tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes, // empty — the flattened fields are carried by the slot list
            result_byte,
            false,
            None, // single-tuple flat path unused
            &[],  // single-tuple prefix unused
            &[],  // single-tuple suffix unused
            None, // single-tuple nested shape unused
            Some(slots),
        ));
    }
    // A SOLE `(Option scalar)`/`(Result scalar scalar)` closure ARG with a SCALAR result: the sum crosses as a
    // native `option<payload>`/`result<ok,err>` the canonical ABI flattens to `(disc:i32, payload)`. The core
    // `call` rebuilds the sum cell (branch on disc → `sum-new`); the envelope mints the boundary type via the
    // classifier's `ArgSlot`. A LIST result over a sum arg is a later widening (list cores don't thread sums).
    if let Some((slot, _payload_vt, rebuild)) = &sum_arg
        && !ret_is_bytes
        && !ret_is_compound
        && !ret_is_collection
    {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &[serialize::ClosureMake {
                export_name: "make".to_string(),
                export_abs,
                param_vts: make_param_vts.clone(),
            }],
            &[],
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false, // own<t> (single-use) — the rebuilt sum cell drop is unconditional, so leak-free
            &[],   // no tuple arg
            std::slice::from_ref(rebuild),
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes, // empty — the flattened disc/payload are carried by the slot list
            result_byte,
            false,
            None, // single-tuple flat path unused
            &[],  // prefix unused
            &[],  // suffix unused
            None, // nested shape unused
            Some(std::slice::from_ref(slot)),
        ));
    }
    // A SUM arg with a LIST result (byte-rope / value-form / value-encode) DECLINES cleanly: the three
    // list-result cores + their envelopes thread `list_rebuilds` (tuple rebuilds), not `SumArgRebuild`s, so the
    // boundary `call` functype would carry the flattened `(disc, payload)` params while the core rebuilds no sum
    // cell — a mismatched module. (Guarding here, before the list blocks below, prevents that miscompile; the
    // scalar-result sum path above already returned.) Threading sums through the list cores is a later widening.
    if sum_arg.is_some() && (ret_is_bytes || ret_is_compound || ret_is_collection) {
        return Err(Reject::unsupported(
            "a closure taking a sum (Option/Result) argument AND returning a list/compound/byte-rope needs \
             the sum-arg rebuild path in the list-result cores (which thread tuple-arg rebuilds, not sum-arg)",
        ));
    }
    // A `Bytes`-result closure crosses `call` as `list<u8>` (through linear memory): the bytes-result core
    // serializer + the memory/realloc-lifting envelope. A scalar result takes the by-value path.
    if ret_is_bytes {
        // C-HOST-6: the `call` takes `borrow<t>` (repeatable — the host keeps the handle; the `t-dtor`
        // reclaims the cell). The byte-rope copy path is unaffected; only the cell rep-recovery + release
        // change (rep passed directly, no self-drop). A fixed-shape tuple ARG is threaded via `tuple_arg`:
        // the bytes `call` rebuilds the cell from the flattened fields, the envelope emits the `tuple<…>` type.
        let main_core = serialize::closure_bytes_resource_core_module_borrow(
            &funcs,
            &imports,
            export_abs,
            &arg_vts,
            &make_param_vts,
            lifted_type_idx,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_bytes_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes,
            true,
            list_tuple_bytes,
            list_tpre,
            list_tsuf,
            list_shape,
            list_slots,
        ));
    }
    // A COMPOUND result crosses `call` as `list<u8>` carrying the value form — same `list<u8>` boundary as
    // the bytes path (so the SAME envelope), but the core walks the closure's returned handle to fill the
    // value-form template. The host decodes the bytes to `(: value T)`.
    if let Some(template) = &ret_template {
        // C-HOST-6 borrow<t> `call` — the compound-walk path is unaffected; the cell is kept (repeatable). A
        // fixed-shape tuple ARG is threaded via `tuple_arg`: the value-form `call` rebuilds the arg cell from
        // the flattened fields before dispatch, and the shared list<u8> envelope emits the `tuple<…>` type.
        let main_core = serialize::closure_value_resource_core_module_borrow(
            &funcs,
            &imports,
            export_abs,
            &arg_vts,
            &make_param_vts,
            lifted_type_idx,
            template,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_bytes_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes,
            true,
            list_tuple_bytes,
            list_tpre,
            list_tsuf,
            list_shape,
            list_slots,
        ));
    }
    // A VARIABLE-LENGTH collection result → the value-encode core (dispatch → the collection handle, build
    // the descriptor Bytes, `value-encode(rep, desc)` → the value-form document, copy out). Same `list<u8>`
    // envelope as the bytes/compound paths; cdz-run try-decodes to `(: (list …) (List <e>))` etc.
    if let Some(descriptor) = &ret_descriptor {
        // C-HOST-6 borrow<t> `call` — the value-encode path is unaffected; the cell is kept (repeatable). A
        // fixed-shape tuple ARG is threaded via `tuple_arg`: the value-encode `call` rebuilds the arg cell
        // from the flattened fields before dispatch, and the shared list<u8> envelope emits the `tuple<…>` type.
        let main_core = serialize::closure_value_encode_resource_core_module_borrow(
            &funcs,
            &imports,
            export_abs,
            &arg_vts,
            &make_param_vts,
            lifted_type_idx,
            descriptor,
            &layout,
            true,
            &list_rebuilds,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_bytes_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes,
            true,
            list_tuple_bytes,
            list_tpre,
            list_tsuf,
            list_shape,
            list_slots,
        ));
    }
    // DIRECT-CALL COMPOUND ARG: a fixed-shape scalar tuple/record closure argument crosses as a native
    // component `tuple<…>` the canonical ABI flattens into scalar core params. The core `call` receives the
    // flattened fields (`arg_vts` = the field valtypes, set above) and REBUILDS the tuple cell from them via
    // the `TupleArgRebuild`; the envelope's `call` functype takes a `tuple<field-bytes…>` type. Scalar result,
    // no host effect (verified when `tuple_arg` was detected). Uses `own<t>` (single-use) — the borrow lift's
    // directly-passed rep is orthogonal, and this first cut keeps the simpler own/self-drop posture; the
    // rebuilt-arg drop is unconditional regardless (a per-call temporary), so it stays leak-free.
    if let Some((field_bytes, _all_vts, tpre, tsuf, rebuild)) = &tuple_arg {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &[serialize::ClosureMake {
                export_name: "make".to_string(),
                export_abs,
                param_vts: make_param_vts.clone(),
            }],
            &[],
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false, // own<t> (single-use) — the rebuilt-arg cell drop is unconditional, so still leak-free
            std::slice::from_ref(rebuild),
            &[], // no sum arg (single flat/nested tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes, // empty — the tuple arg is carried by `field_bytes`
            result_byte,
            false,
            Some(field_bytes),
            tpre, // prefix scalar bytes (empty for a sole-tuple arg)
            tsuf, // suffix scalar bytes
            None, // an all-scalar-field tuple — no nested shape
            None, // single flat tuple → the tuple_arg_bytes path, not the N-slot model
        ));
    }
    // DIRECT-CALL NESTED COMPOUND ARG (single-export, SCALAR result): a SOLE fixed-shape compound arg with a
    // NESTED compound field crosses as a nested `tuple<…, tuple<…>>` the canonical ABI flattens RECURSIVELY to
    // leaf scalar core params. The core `call` rebuilds the nested cell via the recursive `TupleArgRebuild`;
    // the envelope mints the inner `tuple<…>` types by index (`TupleFieldShape`). A nested arg with a
    // list<u8>-crossing result (byte-rope / compound / collection) was already handled by the list-result
    // routings above (which thread `nested_tuple`'s shape); only a SCALAR result reaches here.
    if let Some((_leaf_bytes, _leaf_vts, rebuild, shape, npre, nsuf)) = &nested_tuple {
        let main_core = serialize::multi_closure_resource_core_module_with_host_borrow(
            &funcs,
            &imports,
            &[],
            &[serialize::ClosureMake {
                export_name: "make".to_string(),
                export_abs,
                param_vts: make_param_vts.clone(),
            }],
            &[],
            &arg_vts,
            ret_vt,
            lifted_type_idx,
            &layout,
            false, // own<t> (single-use) — the rebuilt-arg cell drop is unconditional, so still leak-free
            std::slice::from_ref(rebuild),
            &[], // no sum arg (single flat/nested tuple path)
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_resource_borrow_tuple(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes, // empty — the flattened leaves are carried by the shape
            result_byte,
            false,
            None, // the flat all-scalar path is unused here; the shape drives the mint
            npre, // prefix scalar bytes (empty for a sole nested tuple, non-empty when among scalars)
            nsuf,
            Some(shape),
            None, // single (nested) tuple → the tuple_shape path, not the N-slot model
        ));
    }
    // A SCALAR single-export closure `call` takes `borrow<t>` — the host KEEPS the handle across calls (a
    // REPEATABLE callback, the natural host-closure shape), and the `t-dtor` reclaims the cell when the host
    // finally drops it (`resource_dtor_module_with_drop`, already used above). This replaces the earlier
    // own/self-drop single-use posture (`resource.rep` on a borrow traps in wasmtime 37 — dodged by using the
    // rep the borrow-lift passes DIRECTLY, no `resource.rep`). Still leak-free: make allocs the cell, the dtor
    // drops it. (The compound/collection `call` results above keep own/self-drop for now — a later widening;
    // borrow there also needs the value form's own memory handling, out of this increment's scope.)
    let main_core = serialize::closure_resource_core_module_borrow(
        &funcs,
        &imports,
        export_abs,
        &arg_vts,
        ret_vt,
        &make_param_vts,
        lifted_type_idx,
        &layout,
        true,
    )
    .map_err(Reject::decline)?;
    Ok(envelope::assemble_closure_resource_borrow(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &make_param_bytes,
        &arg_bytes,
        result_byte,
        true,
    ))
}

/// Emit a SCALAR-result closure-resource component whose build-time `make` code delegates a host effect
/// (the closure-capture feature, brick d): `(host (ask) (let ((v (ask.ask))) (fn (x) (+ x v))))`. Composes
/// the host interface into the closure resource, mirroring the scalar tail of [`emit_closure_resource`] but
/// threading `host_imports`: the layout records `host_order` (so `select` emits each `Core::HostCall` as a
/// `CallHostImport(raw position)`) and shifts `import_base` by `h` (host funcs occupy core `0..h`, ahead of
/// the runtime ops + resource intrinsics — matching `multi_closure_resource_core_module_with_host`'s
/// host-first layout). The core is built by that `_with_host` serializer; the component by
/// `assemble_closure_host_runtime_resource`. Caller has verified: scalar closure result, single effect,
/// scalar/unit host ops (no shared memory).
#[allow(clippy::too_many_arguments)]
fn emit_closure_host_resource(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    host_imports: &[host::HostImport],
    iface: &str,
    arg_bytes: &[u8],
    result_byte: u8,
    arg_tys: &[crate::ty::Ty],
    ret_ty: &crate::ty::Ty,
) -> Result<Vec<u8>, Reject> {
    use crate::backend::wasm::lir::valtype_of;
    let h = host_imports.len();
    // The host-import ORDER — its `(effect, op)` name pairs, so `select` resolves each `Core::HostCall` to
    // its raw position (= its core-func index 0..h, host-first). Recorded on the layout BEFORE selection.
    let host_order: Vec<(String, String)> = host_imports
        .iter()
        .map(|hi| (hi.effect.clone(), hi.op.clone()))
        .collect();
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = arg_tys
        .iter()
        .map(|t| valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype")))
        .collect::<Result<_, _>>()?;
    let ret_vt = valtype_of(ret_ty)
        .ok_or_else(|| Reject::decline("closure result has no machine valtype"))?;
    let export_params: Vec<(crate::ast::StructId, crate::ty::Ty)> = layout
        .exports
        .iter()
        .find(|e| e.def == export_def)
        .map(|e| e.params.clone())
        .unwrap_or_default();
    let make_param_vts: Vec<crate::backend::wasm::lir::ValType> = export_params
        .iter()
        .map(|(_, t)| {
            valtype_of(t)
                .ok_or_else(|| Reject::decline("closure export param has no machine valtype"))
        })
        .collect::<Result<_, _>>()?;
    let make_param_bytes: Vec<u8> = export_params
        .iter()
        .map(|(_, t)| {
            closure_boundary_byte(t)
                .ok_or_else(|| closure_boundary_reject("parameter", t, &db.name_ctx()))
        })
        .collect::<Result<_, _>>()?;
    // Lifted closure bodies' ops (a capturing body reads its env), as `emit_closure_resource` does.
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
    // Build the runtime-import set + select, with `host_order` set + `import_base` shifted by `h` (host
    // funcs 0..h, then k runtime ops, then 2 resource intrinsics = the closure escape's `intrinsics`).
    let layout_ho = layout.with_host_order(host_order);
    let (imports, mut funcs, built) =
        resource_escape_build_host(db, &layout_ho, h as u32, |used| {
            used.insert("arr-get");
            used.insert("get-int");
            used.insert("drop");
            used.extend(lifted_ops.iter().copied());
        })?;
    let layout = built;
    let export_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the closure export is not in the emission order"))?;
    if layout.lifted.is_empty() {
        return Err(Reject::decline(
            "a closure export produced no lifted lambda (the closure did not survive as a runtime value)",
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
    let lifted_type_idx = layout.lifted_type_index(0, layout.import_base);
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
    let main_core = serialize::multi_closure_resource_core_module_with_host(
        &funcs,
        &imports,
        host_imports,
        &[serialize::ClosureMake {
            export_name: "make".to_string(),
            export_abs,
            param_vts: make_param_vts.clone(),
        }],
        &[],
        &arg_vts,
        ret_vt,
        lifted_type_idx,
        &layout,
    )
    .map_err(Reject::decline)?;
    Ok(envelope::assemble_closure_host_runtime_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        iface,
        &host_fns,
        &make_param_bytes,
        arg_bytes,
        result_byte,
    ))
}

/// [`resource_escape_build_n`] with a HOST-import prefix: shifts `import_base` by `host_count` beyond the
/// runtime ops + resource intrinsics (host funcs occupy core `0..host_count`, ahead of the runtime import
/// section — the host-first layout `multi_closure_resource_core_module_with_host` + `CallHostImport` rely
/// on). Otherwise identical to `resource_escape_build_n` (collect runtime ops from the order bodies, select
/// with the shifted base). The layout passed in MUST already carry `host_order` (so selection resolves the
/// `Core::HostCall`s). `intrinsics` is the resource-intrinsic count (2 for one resource type).
fn resource_escape_build_host(
    db: &mut Db,
    layout: &Layout,
    host_count: u32,
    extra: impl FnOnce(&mut std::collections::BTreeSet<&'static str>),
) -> Result<(Vec<&'static runtime_abi::RtOp>, Vec<SelectedFunc>, Layout), Reject> {
    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    // Scan BOTH the top-level defs AND the lambda-lifted closure bodies — a lifted body is emitted as its
    // own function (see `append_lifted_bodies`), so an op used ONLY inside a closure must be imported too,
    // or its `CallImport` resolves to `u32::MAX` → invalid wasm (the escape-module closure-table bug).
    collect_module_used_ops(db, layout, &mut used)?;
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
    // import_base = host funcs + runtime ops + 2 resource intrinsics.
    let layout = layout.with_import_base(host_count + imports.len() as u32 + 2);
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

/// Emit the MULTI-EXPORT closure-resource component: several exports whose results are all closures of the
/// SAME signature `(-> A… R)` cross together as one resource type with N `make-<name>` functions sharing
/// ONE `call` (`DESIGN-closure-host-resource-rcdzc.md`, multi-export). Each export's body builds its own
/// closure cell (occupying its own funcref-table slot); its `make` calls that body + `resource.new`s the
/// handle. The shared `call` recovers the code slot from the rep at call time, so it dispatches whichever
/// closure a handle names (proven by the `multi_export_closures_share_one_call` oracle). Distinct
/// signatures (N resource types) are a later slice — declined by the caller.
fn emit_multi_closure_resource(
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
fn emit_mixed_closure_resource(
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

/// Emit the DISTINCT-SIGNATURE multi-export component: several closure exports of DIFFERENT signatures
/// cross as G resource types (one per distinct signature), each with its own `make-<name>`(s) + `call-<g>`.
/// Generalizes `emit_multi_closure_resource` (which requires ONE shared signature). Exports are GROUPED by
/// their solved closure signature; each group becomes a `SigGroup` (serializer) + `SigGroupAbi` (envelope).
/// A group's representative `call_indirect` functype is the FIRST lifted lambda whose valtype shape matches
/// that signature (all closures of one signature share the shape, so slot choice within a group is
/// immaterial). The `distinct_signature_…` oracle + the distinct-sig serializer seam proved the pieces.
fn emit_distinct_sig_resource(
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
fn emit_roundtrip_resource(
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
fn emit_distinct_sig_roundtrip_resource(
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
fn emit_runtime_bytes_resource(
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
        let iface = host_imports[0].effect.clone();
        if host_imports.iter().any(|hi| hi.effect != iface) {
            return Err(Reject::unsupported(
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
            return Err(Reject::unsupported(
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
fn emit_recursive_sum_resource(
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
            return Err(Reject::unsupported(
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

/// §3c — decide which provider export member (if any) the TARGET WIT WORLD declares as a bytes boundary,
/// so [`crate::backend::wasm`] routes it to [`emit_bytes_provider_member`]. Decodes the world (`db.wit_world`,
/// raw cadenza-ast bytes), then for each EXPORT member matched to a layout export BY NAME (v-ah: name match),
/// applies [`crate::wit_world::bridge_decision`] to the single declared param + result against the guest's
/// value-model types: BOTH must be `ValueForm` (the world declares `list<u8>` over a value-encodable guest
/// compound). Returns the def of the first such member. Purely declared-signature-driven — no member name
/// or contract shape is hard-coded, so the compiler stays generic over any WIT (the fold's `apply` is merely
/// the first member that satisfies this, not a special case).
/// Whether the target world declares an EXPORT interface with a `record`-param member — the domain of the
/// typed interface-instance emit (`record_interface_export`). Used to (a) admit a compound host result in
/// the guard, and (b) turn a `record_interface_export` miss into a clean DECLINE rather than a silent
/// heap-handle fallback (a program whose world declares a typed export interface it does not fully
/// implement). Purely world/WIT-shape-driven — no interface or member name is hard-coded, so the compiler
/// stays generic over any WIT export.
fn world_has_typed_record_export(world_bytes: &[u8]) -> bool {
    let Some(arenas) = crate::codec::decode(world_bytes) else {
        return false;
    };
    let Some(world) = crate::wit_world::parse_target_world(&arenas, arenas.root) else {
        return false;
    };
    world.exports.iter().any(|i| {
        i.members.iter().any(|m| {
            m.func
                .params
                .iter()
                .any(|(_, t)| matches!(t, crate::wit_world::WitType::Record(_)))
        })
    })
}

fn world_bytes_crossing_export(layout: &Layout, world_bytes: &[u8]) -> Option<usize> {
    use crate::wit_world::{BridgeAction, bridge_decision, parse_target_world};
    let arenas = crate::codec::decode(world_bytes)?;
    let world = parse_target_world(&arenas, arenas.root)?;
    for iface in &world.exports {
        for member in &iface.members {
            // Bind by kebab-normalized name: the WIT member (`on-message`, kebab) matches a guest export
            // (`onMessage`) under the same rule fields/variants/exports cross under (a Cadenza def cannot be
            // named `on-message`). Both sides normalized (a WIT member is already kebab — idempotent).
            let mk = crate::backend::common::export_name::kebab_extern_name(&member.name);
            let Some(e) = layout
                .exports
                .iter()
                .find(|e| crate::backend::common::export_name::kebab_extern_name(&e.name) == mk)
            else {
                continue;
            };
            // The current bytes-boundary slice is a single-compound-param member (widened later); require the
            // declared param + result to both value-form-bridge against the guest's types.
            if member.func.params.len() != 1 || e.params.len() != 1 {
                continue;
            }
            let param_vf = bridge_decision(&member.func.params[0].1, &e.params[0].1)
                == BridgeAction::ValueForm;
            let result_vf =
                bridge_decision(&member.func.result, &e.result) == BridgeAction::ValueForm;
            if param_vf && result_vf {
                return Some(e.def);
            }
        }
    }
    None
}

/// If the target world (`world_bytes`, binary-AST) declares an export INTERFACE whose members all bind (by
/// name) to guest exports and are all SCALAR funcs — every param/result a scalar WIT type the guest's
/// solved `Ty` lowers to DIRECTLY (no value-form bridge, no memory) — build the [`envelope::TypedInterface`]
/// to emit it as a component instance. `None` when there is no such world, a member does not bind, or any
/// member carries a `record`/`variant`/`list<u8>`/`string` (which needs the lift/lower wrapper — a later
/// slice, still declined here). `iface` is the component's fully-qualified export name (`db.component_name`);
/// the instance is exported under it. Scalar funcs need no wrapper: the compiled def's core func already has
/// the flattened boundary signature, so the interface aliases it directly.
fn scalar_interface_export(
    layout: &Layout,
    world_bytes: &[u8],
    iface: &str,
) -> Option<envelope::TypedInterface> {
    use crate::wit_world::{BridgeAction, WitType, bridge_decision, parse_target_world};
    fn is_scalar(t: &WitType) -> bool {
        matches!(
            t,
            WitType::Bool
                | WitType::U8
                | WitType::U16
                | WitType::U32
                | WitType::U64
                | WitType::S8
                | WitType::S16
                | WitType::S32
                | WitType::S64
                | WitType::F32
                | WitType::F64
                | WitType::Char
        )
    }
    let arenas = crate::codec::decode(world_bytes)?;
    let world = parse_target_world(&arenas, arenas.root)?;
    // A reducer guest declares one export interface; bind every member to a guest export and require it be a
    // fully-scalar, directly-liftable signature.
    let export_iface = world.exports.first()?;
    let mut funcs = Vec::with_capacity(export_iface.members.len());
    for member in &export_iface.members {
        // Bind the WIT member to a guest export by kebab-normalized name (a Cadenza def `onMessage` binds to
        // the WIT `on-message` member — same rule as fields/variants/exports).
        let mk = crate::backend::common::export_name::kebab_extern_name(&member.name);
        let e = layout
            .exports
            .iter()
            .find(|e| crate::backend::common::export_name::kebab_extern_name(&e.name) == mk)?;
        if member.func.params.len() != e.params.len() {
            return None;
        }
        for ((_, declared), (_, guest)) in member.func.params.iter().zip(e.params.iter()) {
            if !is_scalar(declared) || bridge_decision(declared, guest) != BridgeAction::Direct {
                return None;
            }
        }
        let result = match &member.func.result {
            WitType::Unit => None,
            declared => {
                if !is_scalar(declared)
                    || bridge_decision(declared, &e.result) != BridgeAction::Direct
                {
                    return None;
                }
                Some(declared.clone())
            }
        };
        funcs.push(envelope::TypedFunc {
            name: member.name.clone(),
            params: member.func.params.clone(),
            result,
        });
    }
    if funcs.is_empty() {
        return None;
    }
    Some(envelope::TypedInterface {
        name: iface.to_string(),
        types: Vec::new(),
        funcs,
    })
}

/// How to read ONE scalar VALUE off its value-heap handle and store it canonically: `(read-op, narrow-i64→
/// i32?, store-opcode)`. `get-int` yields i64, so a ≤32-bit slot narrows + stores an i32-width store; a
/// 64-bit slot stores i64. `None` if `f` is not an aliased-width scalar.
fn scalar_result_read_store(f: &crate::ty::Ty) -> Option<(&'static str, bool, u8)> {
    use crate::backend::wasm::wasm_abi::op;
    use crate::ty::Ty;
    Some(match f.strip_nominal() {
        Ty::Int(it) => {
            let w = it.ground_width();
            if w <= 8 {
                ("get-int", true, op::I32_STORE8)
            } else if w <= 16 {
                ("get-int", true, op::I32_STORE16)
            } else if w <= 32 {
                ("get-int", true, op::I32_STORE)
            } else {
                ("get-int", false, op::I64_STORE)
            }
        }
        Ty::Bool => ("get-bool", false, op::I32_STORE8),
        Ty::Float(ft) if ft.ground_width() == 64 => ("get-float", false, op::F64_STORE),
        Ty::Float(ft) if ft.ground_width() == 32 => ("get-float32", false, op::F32_STORE),
        _ => return None,
    })
}

/// Like [`scalar_result_read_store`] but driven by the WIT scalar type (not the guest `Ty`) — used when the
/// guest value type is UNRESOLVED (a `None`-only option's payload var) but the boundary still fixes the
/// representation: a `Some` value of WIT type `T` is a value-heap-boxed `T`, so the same unbox/store applies.
/// `None` for a non-scalar WIT type.
fn scalar_read_store_of_wit(wty: &crate::wit_world::WitType) -> Option<(&'static str, bool, u8)> {
    use crate::backend::wasm::wasm_abi::op;
    use crate::wit_world::WitType;
    Some(match wty {
        WitType::Bool => ("get-bool", false, op::I32_STORE8),
        WitType::U8 | WitType::S8 => ("get-int", true, op::I32_STORE8),
        WitType::U16 | WitType::S16 => ("get-int", true, op::I32_STORE16),
        WitType::U32 | WitType::S32 | WitType::Char => ("get-int", true, op::I32_STORE),
        WitType::U64 | WitType::S64 => ("get-int", false, op::I64_STORE),
        WitType::F32 => ("get-float32", false, op::F32_STORE),
        WitType::F64 => ("get-float", false, op::F64_STORE),
        _ => return None,
    })
}

/// A [`serialize::CanonWrite`] built from the WIT type ALONE (no guest `Ty`), for when the guest value type
/// is UNRESOLVED — the DEAD element/payload writer of an empty polymorphic collection (`step{requests: []}`
/// — a reducer that emits no effects) or a `None` option. The value-heap representation follows the type, so
/// a `Some`/non-empty value of this WIT type would be laid this way; but for the empty/none case the code
/// never executes, so a record's field SLOTS use identity (the guest's name-lex slots are unknowable without
/// its `Ty`, but no element is ever written). Only VALIDITY matters here, not the (dead) slot mapping.
fn canon_write_from_wit(
    wty: &crate::wit_world::WitType,
) -> Option<crate::backend::wasm::serialize::CanonWrite> {
    use crate::backend::wasm::serialize::{CanonField, CanonWrite, VariantArm};
    use crate::backend::wasm::wit_ctype;
    use crate::wit_world::WitType;
    Some(match wty {
        WitType::List(e) if **e == WitType::U8 => CanonWrite::Bytes,
        WitType::List(e) => CanonWrite::List {
            elem_size: wit_ctype::canonical_size(e),
            elem_align: wit_ctype::canonical_align(e),
            elem: Box::new(canon_write_from_wit(e)?),
        },
        WitType::Record(fields) => {
            let wtys: Vec<WitType> = fields.iter().map(|(_, t)| t.clone()).collect();
            let offsets = wit_ctype::record_field_offsets(&wtys);
            let mut fs = Vec::new();
            for (i, ((_, ft), off)) in fields.iter().zip(offsets.iter()).enumerate() {
                fs.push(CanonField {
                    index: i as u32, // identity — dead code (this whole writer is only reached for empty/none)
                    offset: *off,
                    write: canon_write_from_wit(ft)?,
                });
            }
            CanonWrite::Record { fields: fs }
        }
        WitType::Option(inner) => {
            let cps = [None, Some(inner.as_ref())];
            let (disc_size, payload_offset) = wit_ctype::variant_disc_layout(&cps);
            CanonWrite::Variant {
                disc_store: disc_store_of(disc_size),
                payload_offset,
                arms: vec![
                    VariantArm {
                        boundary_disc: 0,
                        payload: None,
                    },
                    VariantArm {
                        boundary_disc: 1,
                        payload: Some(Box::new(canon_write_from_wit(inner)?)),
                    },
                ],
            }
        }
        WitType::Variant(cases) => {
            let cps: Vec<Option<&WitType>> = cases.iter().map(|(_, p)| p.as_ref()).collect();
            let (disc_size, payload_offset) = wit_ctype::variant_disc_layout(&cps);
            let mut arms = Vec::new();
            for (i, (_, p)) in cases.iter().enumerate() {
                arms.push(VariantArm {
                    boundary_disc: i as u32,
                    payload: match p {
                        None => None,
                        Some(pw) => Some(Box::new(canon_write_from_wit(pw)?)),
                    },
                });
            }
            CanonWrite::Variant {
                disc_store: disc_store_of(disc_size),
                payload_offset,
                arms,
            }
        }
        scalar => {
            let (read, wrap_i64, store) = scalar_read_store_of_wit(scalar)?;
            CanonWrite::Scalar {
                read,
                wrap_i64,
                store,
            }
        }
    })
}

/// The i32 store opcode for a variant discriminant of `disc_size` bytes.
fn disc_store_of(disc_size: u32) -> u8 {
    use crate::backend::wasm::wasm_abi::op;
    match disc_size {
        1 => op::I32_STORE8,
        2 => op::I32_STORE16,
        _ => op::I32_STORE,
    }
}

/// Build the recursive [`serialize::CanonWrite`] that lowers a value-heap value of type `gty` (WIT `wty`) to
/// its canonical-ABI memory form — the reducer result-lower. A scalar unboxes + stores; a `Bytes`/`list<u8>`
/// copies its bytes out; a record recurses per field at canonical offsets (permuted by name); a `list<T>`
/// writes an element array; option/named-variant write disc + payload. When `gty` is UNRESOLVED (an empty
/// collection / `None`), falls back to [`canon_write_from_wit`] (the dead element/payload writer).
fn canon_write_of(
    db: &mut Db,
    gty: &crate::ty::Ty,
    wty: &crate::wit_world::WitType,
) -> Option<serialize::CanonWrite> {
    use crate::backend::wasm::serialize::{CanonField, CanonWrite, VariantArm};
    use crate::backend::wasm::wasm_abi::op;
    use crate::backend::wasm::wit_ctype;
    use crate::ty::Ty;
    use crate::wit_world::WitType;
    // An UNRESOLVED guest type — the dead element/payload writer of an empty collection / `None`. The value
    // never materializes, so drive the (valid but dead) write off the WIT type alone.
    if matches!(gty.strip_nominal(), Ty::Var(_) | Ty::Any) {
        return canon_write_from_wit(wty);
    }
    match gty.strip_nominal() {
        Ty::Bytes => Some(CanonWrite::Bytes),
        Ty::Record(map) => {
            use crate::backend::common::export_name::kebab_extern_name;
            let WitType::Record(wfs) = wty else {
                return None;
            };
            if wfs.len() != map.len() {
                return None;
            }
            // PERMUTE BY NAME: the value-heap cell's slots are in the guest's name-lex (`BTreeMap`) order,
            // but the WIT record lays its fields (and their canonical offsets) in DECLARATION order — which
            // need not be name-lex (the real `step{requests, outcome}` / `request{contract, payload, token,
            // deadline-nanos}` are declaration-ordered). So map each WIT field to its guest slot BY NAME
            // (kebab-normalized, the same rule as variants/exports) — read `arr-get(handle, guest_slot)` and
            // write at the WIT field's canonical offset. A field with no name match declines.
            let guest_names: Vec<String> = map
                .keys()
                .map(|s| kebab_extern_name(s.name.as_ref()))
                .collect();
            let gtys: Vec<Ty> = map.values().cloned().collect();
            let wtys: Vec<WitType> = wfs.iter().map(|(_, t)| t.clone()).collect();
            let offsets = wit_ctype::record_field_offsets(&wtys);
            let mut fields = Vec::new();
            for ((wname, fw), off) in wfs.iter().zip(offsets.iter()) {
                let slot = guest_names.iter().position(|g| g == wname)?;
                let fg = gtys[slot].clone();
                let fw = fw.clone();
                fields.push(CanonField {
                    index: slot as u32,
                    offset: *off,
                    write: canon_write_of(db, &fg, &fw)?,
                });
            }
            Some(CanonWrite::Record { fields })
        }
        // A TUPLE result / payload — the POSITIONAL twin of the Record arm: element `i` of the guest tuple
        // cell lives at slot `i` (no name permutation), written at the WIT tuple's canonical offset. A tuple
        // and a record with the same element types share the component canonical layout, so
        // `record_field_offsets` yields the tuple's offsets too; `CanonWrite::Record` (index-keyed arr-get)
        // is reused with positional indices — no new writer. This admits a bare tuple export result AND a
        // variant/record payload that is a tuple (the variant arm recurses here). Element writes recurse, so
        // a nested tuple/record/list/bytes/scalar element composes.
        Ty::Tuple(elems) => {
            let WitType::Tuple(wits) = wty else {
                return None;
            };
            if wits.len() != elems.len() {
                return None;
            }
            let offsets = wit_ctype::record_field_offsets(wits);
            let mut fields = Vec::new();
            for (i, ((eg, ew), off)) in elems
                .iter()
                .zip(wits.iter())
                .zip(offsets.iter())
                .enumerate()
            {
                fields.push(CanonField {
                    index: i as u32,
                    offset: *off,
                    write: canon_write_of(db, eg, ew)?,
                });
            }
            Some(CanonWrite::Record { fields })
        }
        Ty::List(elem_g) => {
            let WitType::List(elem_w) = wty else {
                return None;
            };
            let elem_g = (**elem_g).clone();
            let elem_w = (**elem_w).clone();
            Some(CanonWrite::List {
                elem_size: wit_ctype::canonical_size(&elem_w),
                elem_align: wit_ctype::canonical_align(&elem_w),
                elem: Box::new(canon_write_of(db, &elem_g, &elem_w)?),
            })
        }
        // NAMED VARIANT result: a guest sum crossing as WIT `variant<case, case(T), …>`. NAME-MATCHED
        // (v-platform ruling): each guest ctor is normalized to kebab-case (`kebab_extern_name`, the same
        // rule record fields + exports cross under) and must equal a WIT case name — the guest's decl order
        // is irrelevant, so reordering the guest's variants can't silently remap a case to the wrong payload
        // (§1 nominal identity). `boundary_disc` = the matched WIT case index. Belt-and-suspenders: each arm's
        // payload-presence must still agree (guest-nullary iff WIT-case-nullary). A payload arm resolves its
        // type via the ctor + `payload_ty_at_instantiation` (concrete payloads, e.g. a `closed` record, too).
        // A payloadless `enum` (all-nullary sum) result FIELD — the guest value is a BARE i32 discriminant
        // (`db.is_enum_disc`, NOT a heap `sum`, unlike a payload-carrying variant handled below), so store the
        // disc DIRECTLY (no unbox). Accepts a WIT `enum` OR an all-nullary `variant` (both the disc-only
        // shape). Gated on the guest decl-order MATCHING the WIT case-order by name, so the guest's raw disc
        // IS the WIT case index (no runtime remap; a reordering declines this increment — a later slice). The
        // guest-export result-side twin of a host-import enum arg/result (bare-i32 enum-disc).
        Ty::Sum { decl, .. } if db.is_enum_disc(*decl) => {
            use crate::backend::common::export_name::kebab_extern_name;
            let wit_cases: Vec<String> = match wty {
                WitType::Enum(cs) => cs.clone(),
                WitType::Variant(cs) if cs.iter().all(|(_, p)| p.is_none()) => {
                    cs.iter().map(|(n, _)| n.clone()).collect()
                }
                _ => return None,
            };
            let guest_cases: Vec<String> = {
                let dr = db.type_decl_by_occ(*decl)?;
                dr.variants
                    .iter()
                    .map(|v| kebab_extern_name(&v.name))
                    .collect()
            };
            if guest_cases != wit_cases {
                return None; // a case REORDER would need a runtime disc remap — later increment
            }
            let (disc_size, _) = wit_ctype::variant_disc_layout(&vec![None; wit_cases.len()]);
            let store = match disc_size {
                1 => op::I32_STORE8,
                2 => op::I32_STORE16,
                _ => op::I32_STORE,
            };
            Some(CanonWrite::EnumDisc { store })
        }
        Ty::Sum { decl, .. } if matches!(wty, WitType::Variant(_)) => {
            use crate::backend::common::export_name::kebab_extern_name;
            let WitType::Variant(cases) = wty else {
                unreachable!()
            };
            let sum_ty = gty.strip_nominal().clone();
            // Per guest variant (in decl-disc order): (kebab-normalized ctor name, nullary?, ctor occ).
            let guest: Vec<(String, bool, Option<crate::ast::StructId>)> = {
                let dr = db.type_decl_by_occ(*decl)?;
                if dr.variants.len() != cases.len() {
                    return None;
                }
                dr.variants
                    .iter()
                    .map(|v| (kebab_extern_name(&v.name), v.payloads.is_empty(), v.ctor))
                    .collect()
            };
            // Build arms in GUEST decl-disc order (`sum-disc` returns that); each maps to its WIT case BY NAME.
            let mut arms = Vec::with_capacity(guest.len());
            for (gname, gnullary, ctor) in &guest {
                let wit_idx = cases.iter().position(|(cn, _)| cn == gname)?; // NAME-MATCH (soundness)
                let (_, wit_payload) = &cases[wit_idx];
                if *gnullary != wit_payload.is_none() {
                    return None; // payload-shape belt-and-suspenders
                }
                let payload = match wit_payload {
                    None => None,
                    Some(pw) => {
                        let ctor = (*ctor)?;
                        let payload_gty =
                            crate::infer::payload_ty_at_instantiation(db, ctor, &sum_ty)?;
                        let pw = pw.clone();
                        Some(Box::new(canon_write_of(db, &payload_gty, &pw)?))
                    }
                };
                arms.push(VariantArm {
                    boundary_disc: wit_idx as u32,
                    payload,
                });
            }
            let case_payloads: Vec<Option<&WitType>> =
                cases.iter().map(|(_, p)| p.as_ref()).collect();
            let (disc_size, payload_offset) = wit_ctype::variant_disc_layout(&case_payloads);
            let disc_store = match disc_size {
                1 => op::I32_STORE8,
                2 => op::I32_STORE16,
                _ => op::I32_STORE,
            };
            Some(CanonWrite::Variant {
                disc_store,
                payload_offset,
                arms,
            })
        }
        // OPTION result: a two-variant guest sum (one nullary + one single-payload) crossing as WIT
        // `option<inner>` (boundary None=0, Some=1). Resolve which decl-disc is the payload arm + its payload
        // type (mirrors the param-side `fixed_shape_option_scalar_arg`), then write the payload recursively.
        // A `result<ok,err>` RESULT: a 2-variant Sum whose BOTH arms carry a payload (`Ok(ok)`, `Err(err)`),
        // crossing as WIT `result<ok,err>`. Map each guest variant to its WIT arm BY NAME (`ok`→boundary disc
        // 0, `err`→disc 1), writing that arm's payload (recursively) at the canonical result layout (a 1-byte
        // disc + the payload at `align_up(1, max(align(ok), align(err)))`). The both-payload sibling of the
        // option arm below (which handles the nullary+payload shape). A `result<_, E>`/`result<T, _>` with a
        // NULLARY arm, or a non-two-variant sum, declines to the option/general arms (a later slice).
        Ty::Sum { decl, args, .. } if matches!(wty, WitType::Result { .. }) => {
            use crate::backend::common::export_name::kebab_extern_name;
            let WitType::Result { ok, err } = wty else {
                unreachable!("guarded by the arm pattern");
            };
            // Both WIT arms must carry a payload for this slice (`result<T, E>`); a bare `result<_, _>`/`_, E`
            // (a nullary ok or err) is a later slice.
            let (ok_wit, err_wit) = match (ok, err) {
                (Some(o), Some(e)) => ((**o).clone(), (**e).clone()),
                _ => return None,
            };
            let (params, variants): (Vec<String>, Vec<(String, Vec<crate::ast::StructId>)>) = {
                let dr = db.type_decl_by_occ(*decl)?;
                if dr.variants.len() != 2 {
                    return None;
                }
                (
                    dr.params.clone(),
                    dr.variants
                        .iter()
                        .map(|v| (kebab_extern_name(&v.name), v.payloads.clone()))
                        .collect(),
                )
            };
            // Resolve each guest variant's boundary disc (BY NAME) + its SINGLE generic payload type
            // (instantiated against `args`, like the option arm) EAGERLY — separating the immutable `db.ast`
            // reads from the `&mut db` `canon_write_of` recursion below (no overlapping borrow). Indexed by
            // GUEST decl disc (the value `sum-disc` returns).
            let mut resolved: Vec<(u32, Ty, WitType)> = Vec::with_capacity(2);
            for (name, payloads) in &variants {
                if payloads.len() != 1 {
                    return None; // a nullary result arm — a later slice
                }
                let (boundary_disc, wit) = match name.as_str() {
                    "ok" => (0u32, ok_wit.clone()),
                    "err" => (1u32, err_wit.clone()),
                    _ => return None, // not an ok/err-named 2-variant sum
                };
                let occ = payloads[0];
                let pname = db
                    .ast
                    .head_name(occ)
                    .or_else(|| db.ast.as_name(occ))?
                    .to_string();
                let pi = params.iter().position(|p| *p == pname)?;
                let payload_gty = args.get(pi)?.clone();
                resolved.push((boundary_disc, payload_gty, wit));
            }
            let mut arms: Vec<VariantArm> = Vec::with_capacity(2);
            for (boundary_disc, payload_gty, wit) in &resolved {
                let write = canon_write_of(db, payload_gty, wit)?;
                arms.push(VariantArm {
                    boundary_disc: *boundary_disc,
                    payload: Some(Box::new(write)),
                });
            }
            let payload_align =
                wit_ctype::canonical_align(&ok_wit).max(wit_ctype::canonical_align(&err_wit));
            let payload_offset = 1u32.div_ceil(payload_align) * payload_align;
            Some(CanonWrite::Variant {
                disc_store: op::I32_STORE8,
                payload_offset,
                arms,
            })
        }
        Ty::Sum { decl, args, .. } => {
            let WitType::Option(inner) = wty else {
                return None; // result<> results are a later slice
            };
            let (params, variant_payloads): (Vec<String>, Vec<Vec<crate::ast::StructId>>) = {
                let dr = db.type_decl_by_occ(*decl)?;
                if dr.variants.len() != 2 {
                    return None;
                }
                (
                    dr.params.clone(),
                    dr.variants.iter().map(|v| v.payloads.clone()).collect(),
                )
            };
            let counts: Vec<usize> = variant_payloads.iter().map(|p| p.len()).collect();
            let (payload_i, nullary_i) = match counts.as_slice() {
                [0, 1] => (1usize, 0usize),
                [1, 0] => (0usize, 1usize),
                _ => return None, // not a nullary + single-payload option shape
            };
            // The payload variant's single generic payload type, instantiated against `args`.
            let occ = variant_payloads[payload_i][0];
            let pname = db
                .ast
                .head_name(occ)
                .or_else(|| db.ast.as_name(occ))?
                .to_string();
            let pi = params.iter().position(|p| *p == pname)?;
            let payload_gty = args.get(pi)?.clone();
            let inner = (**inner).clone();
            // For a `None`-ONLY option (the guest never constructs `Some`, e.g. reducer-echo's
            // `deadline-nanos`), the guest's option payload type is an UNRESOLVED var, so `canon_write_of`
            // can't dispatch it — fall back to building the payload write from the WIT inner SCALAR type (the
            // Some arm is dead at runtime, but the write must be valid: a `Some` value would be a scalar of
            // that WIT type). Non-scalar unresolved payloads still decline.
            let payload_write = match canon_write_of(db, &payload_gty, &inner) {
                Some(w) => w,
                None => {
                    let (read, wrap_i64, store) = scalar_read_store_of_wit(&inner)?;
                    serialize::CanonWrite::Scalar {
                        read,
                        wrap_i64,
                        store,
                    }
                }
            };
            // Canonical variant layout: disc is 1 byte (2 cases); payload after the disc, aligned to it.
            let payload_align = wit_ctype::canonical_align(&inner);
            let payload_offset = 1u32.div_ceil(payload_align) * payload_align;
            let mut arms = Vec::with_capacity(2);
            for _ in 0..2 {
                arms.push(VariantArm {
                    boundary_disc: 0,
                    payload: None,
                });
            }
            arms[nullary_i] = VariantArm {
                boundary_disc: 0,
                payload: None,
            };
            arms[payload_i] = VariantArm {
                boundary_disc: 1,
                payload: Some(Box::new(payload_write)),
            };
            Some(CanonWrite::Variant {
                disc_store: op::I32_STORE8,
                payload_offset,
                arms,
            })
        }
        _ => {
            let (read, wrap_i64, store) = scalar_result_read_store(gty)?;
            Some(CanonWrite::Scalar {
                read,
                wrap_i64,
                store,
            })
        }
    }
}

/// Register (via `out`) every runtime op the recursive canonical writer `cw` emits, so the wrapper's core
/// imports them: `arr-get` + a scalar's unbox for a record; `vec-len`/`vec-get` for a list; `bytes-len`/
/// `bytes-get` for a `Bytes` leaf.
fn canon_write_ops(
    cw: &crate::backend::wasm::serialize::CanonWrite,
    out: &mut impl FnMut(&'static str),
) {
    use crate::backend::wasm::serialize::CanonWrite;
    match cw {
        CanonWrite::Scalar { read, .. } => out(read),
        // A bare-i32 enum disc is stored directly (no unbox / heap op) — registers nothing.
        CanonWrite::EnumDisc { .. } => {}
        CanonWrite::Bytes => {
            out("bytes-len");
            out("bytes-get");
        }
        CanonWrite::Record { fields } => {
            out("arr-get");
            for f in fields {
                canon_write_ops(&f.write, out);
            }
        }
        CanonWrite::List { elem, .. } => {
            out("vec-len");
            out("vec-get");
            canon_write_ops(elem, out);
        }
        CanonWrite::Variant { arms, .. } => {
            out("sum-disc");
            if arms.iter().any(|a| a.payload.is_some()) {
                out("sum-payload");
            }
            for a in arms {
                if let Some(p) = &a.payload {
                    canon_write_ops(p, out);
                }
            }
        }
    }
}

/// Build the [`serialize::FieldRebuild`] for ONE record field (recursively), appending its flattened core
/// valtypes to `param_vts` in field order. A scalar boxes one flattened leaf; a `list<u8>`/`Bytes` leaf
/// crosses as `(ptr, len)` and copies out of memory (`BytesLeaf`, two i32); a NESTED record builds a `Nested`
/// rebuild over its own fields (the message's `sender` shape). Declines any other compound field.
fn param_field_rebuild(
    db: &mut Db,
    gty: &crate::ty::Ty,
    wty: &crate::wit_world::WitType,
    param_vts: &mut Vec<u8>,
) -> Option<crate::backend::wasm::serialize::FieldRebuild> {
    use crate::backend::wasm::lir::{ValType, valtype_of};
    use crate::backend::wasm::serialize::FieldRebuild;
    use crate::ty::Ty;
    use crate::wit_world::WitType;
    match gty.strip_nominal() {
        Ty::Bytes => {
            param_vts.push(ValType::I32.byte());
            param_vts.push(ValType::I32.byte());
            Some(FieldRebuild::BytesLeaf)
        }
        Ty::Record(map) => {
            let WitType::Record(wfs) = wty else {
                return None;
            };
            let (fields, slots) = record_fields_rebuild(db, map, wfs, param_vts)?;
            Some(FieldRebuild::Nested(fields, slots))
        }
        // A variant/result PARAM field (the response's `answer: result<…>`): the canon lift hands the
        // flattened `(disc, payload…)`; the wrapper reads them and rebuilds the guest sum cell. First reuse the
        // closure-arg `fixed_shape_option_scalar_arg` — a two-variant Option/Result whose payload arms are
        // scalar or a fixed-shape tuple/record; then `fixed_shape_sum_param_arg` for the `list<u8>`/enum payload
        // arms (`result<list<u8>, error>` — the reducer response's `answer`).
        Ty::Sum { .. } => {
            // The canon lift flattens a variant to `(disc: i32, payload-join…)`. `fixed_shape_option_scalar_arg`
            // returns only the PAYLOAD valtypes (the closure convention passes the disc separately), so prepend
            // the disc's i32 — matching `flattened_param_count` (1 disc + the payload leaves) + `emit_sum_field`
            // (disc at the cursor, payload at cursor+1).
            if let Some((_slot, vts, rebuild)) = fixed_shape_option_scalar_arg(db, gty) {
                param_vts.push(ValType::I32.byte());
                param_vts.extend(vts.iter().map(|vt| vt.byte()));
                return Some(FieldRebuild::Sum(Box::new(rebuild)));
            }
            // `list<u8>` (Bytes) / all-nullary enum payload arms — appends the disc + join vts itself.
            fixed_shape_sum_param_arg(db, gty, param_vts)
        }
        _ => {
            let fr = scalar_field_rebuild(gty)?;
            param_vts.push(valtype_of(gty)?.byte());
            Some(fr)
        }
    }
}

/// Build a record's per-field rebuilds + name-lex SLOTS, PERMUTED BY NAME — the entry for both a top-level
/// record param and a nested-record field, at any WIT field order. Iterates the WIT fields IN WIT ORDER (so
/// `param_vts` + the wrapper's flattened-leaf cursor are the actual canon-lift param order), matching each
/// WIT field to its guest field BY NAME (kebab-normalized, the same rule as variant cases + exports).
/// Returns the rebuild (WIT order) + `slots` (each WIT field's name-lex cell position) — the wrapper
/// `arr-set`s each field at its slot. A WIT field with no name match declines. So a declaration-ordered
/// record like `message{contract, sender, payload, token}` or its nested `sender{reducer, host}` (name-lex
/// `host, reducer`) both cross correctly.
fn record_fields_rebuild(
    db: &mut Db,
    map: &std::collections::BTreeMap<crate::resolved::Symbol, crate::ty::Ty>,
    wfs: &[(String, crate::wit_world::WitType)],
    param_vts: &mut Vec<u8>,
) -> Option<(Vec<crate::backend::wasm::serialize::FieldRebuild>, Vec<u32>)> {
    use crate::backend::common::export_name::kebab_extern_name;
    if wfs.len() != map.len() {
        return None;
    }
    let guest_kebab: Vec<String> = map
        .keys()
        .map(|s| kebab_extern_name(s.name.as_ref()))
        .collect();
    let gtys: Vec<crate::ty::Ty> = map.values().cloned().collect();
    let mut rebuild = Vec::new();
    let mut slots = Vec::new();
    for (wname, fw) in wfs {
        let slot = guest_kebab.iter().position(|g| g == wname)?; // NAME-MATCH
        let fg = gtys[slot].clone();
        rebuild.push(param_field_rebuild(db, &fg, fw, param_vts)?); // appends WIT-order vts
        slots.push(slot as u32);
    }
    Some((rebuild, slots))
}

/// The per-element read+box descriptor for a `list<scalar>` entry param — the load op / natural-align /
/// canonical stride / narrow-int extend / box op for one element of a list of Int8/16/32/64, UInt*,
/// Float32/64, or Bool. `None` for a nested or compound element (list<list>, list<record>) — a later slice.
fn list_scalar_elem(elem: &crate::ty::Ty) -> Option<crate::backend::wasm::serialize::ListElem> {
    use crate::backend::wasm::serialize::ListElem;
    use crate::backend::wasm::wasm_abi::op;
    use crate::ty::Ty;
    Some(match elem.strip_nominal() {
        Ty::Int(it) => {
            let signed = it.ground_signed();
            match it.ground_width() {
                64 => ListElem {
                    load_op: op::I64_LOAD,
                    load_align: 3,
                    stride: 8,
                    extend: None,
                    box_op: "box-int",
                },
                32 => ListElem {
                    load_op: op::I32_LOAD,
                    load_align: 2,
                    stride: 4,
                    extend: Some(signed),
                    box_op: "box-int",
                },
                16 => ListElem {
                    load_op: if signed {
                        op::I32_LOAD16_S
                    } else {
                        op::I32_LOAD16_U
                    },
                    load_align: 1,
                    stride: 2,
                    extend: Some(signed),
                    box_op: "box-int",
                },
                8 => ListElem {
                    load_op: if signed {
                        op::I32_LOAD8_S
                    } else {
                        op::I32_LOAD8_U
                    },
                    load_align: 0,
                    stride: 1,
                    extend: Some(signed),
                    box_op: "box-int",
                },
                _ => return None,
            }
        }
        Ty::Bool => ListElem {
            load_op: op::I32_LOAD8_U,
            load_align: 0,
            stride: 1,
            extend: None,
            box_op: "box-bool",
        },
        Ty::Float(ft) => match ft.ground_width() {
            64 => ListElem {
                load_op: op::F64_LOAD,
                load_align: 3,
                stride: 8,
                extend: None,
                box_op: "box-float",
            },
            32 => ListElem {
                load_op: op::F32_LOAD,
                load_align: 2,
                stride: 4,
                extend: None,
                box_op: "box-float32",
            },
            _ => return None,
        },
        _ => return None, // a nested list / compound element — a later slice
    })
}

/// The PLAIN-EXPORT ENTRY-PARAM emit (entry-param declines slice 1): a SINGLE bare exported def whose param
/// is a memory-bearing `String`/`Bytes` (crossing as `string`/`list<u8>`) gets a guest LIFT WRAPPER — the
/// wrapper copies the incoming `(ptr, len)` bytes out of linear memory into a value-heap `Bytes` (a `String`
/// then via `str-from-bytes`) and calls the def; the component types the param as `string`/`list<u8>` with a
/// Memory/Realloc canon-lift ([`envelope::assemble_bare_typed_with_runtime`]). This is the host→guest MIRROR
/// of the host-op String/Bytes ARG marshal already emitted on the import side.
///
/// Returns `None` (fall through to the boundary loop's honest decline) for any shape outside this slice:
/// more than one export, a compound param other than String/Bytes, a unit param, or a non-scalar/unit
/// result. Widened to List/Option/BigInt/Rational/Symbol (and multi-export) in later slices.
fn try_bare_entry_param_component(
    db: &mut Db,
    layout: &Layout,
    funcs: &[SelectedFunc],
    imports: &[&runtime_abi::RtOp],
) -> Option<Result<Vec<u8>, Reject>> {
    use crate::backend::wasm::lir::{ValType, valtype_of};
    use crate::ty::Ty;
    // Slice 1: exactly one bare export.
    if layout.exports.len() != 1 {
        return None;
    }
    let body = layout.exports[0].body;
    let name = layout.exports[0].name.clone();
    let def = layout.exports[0].def;
    let result_ty = layout.exports[0].result.clone();
    let params: Vec<(crate::ast::StructId, Ty)> = layout.exports[0].params.clone();
    let mut param_vts: Vec<u8> = Vec::new();
    let mut mem_leaf_params: Vec<Option<(serialize::MemLeafKind, bool)>> = Vec::new();
    let mut sum_params: Vec<Option<(serialize::SumArgRebuild, bool)>> = Vec::new();
    let mut wit_params: Vec<(String, crate::wit_world::WitType)> = Vec::new();
    for (i, (binder, gty)) in params.iter().enumerate() {
        // A two-variant sum (`option<T>` / `result<ok,err>`) entry param crosses as a native component sum,
        // flattened to `(disc, payload…)`. Build it DIRECTLY as the def arg via the closure-arg classifier's
        // `SumArgRebuild` (branch on the boundary disc → `sum-new`); the def owns the built cell. `ty_natural_wit`
        // declines a `Ty::Sum`, so the WIT type comes from the Db-aware `spilled_result_wit_type` (`option<T>`).
        if let Some((_slot, vts, rebuild)) = fixed_shape_option_scalar_arg(db, gty) {
            let wit = crate::backend::wasm::host::spilled_result_wit_type(db, gty)?;
            // SCOPE = genuine `option<T>` ONLY. `fixed_shape_option_scalar_arg` also classifies a
            // `result<ok,err>`, but `spilled_result_wit_type` resolves a Result to a WIT `variant` (two payload
            // cases), NOT the built-in `result` — a type whose flattening/disc convention DISAGREES with the
            // option-shaped `SumArgRebuild` lift built here, so admitting it emitted an INVALID component
            // (a `result<scalar,scalar>` param). Until a dedicated Result entry-param sub-slice ties the WIT
            // type and the lift together, DECLINE a Result param: fall through to the honest sum decline
            // (`ty_natural_wit`→None below would also decline, but returning early keeps the reason clear).
            // erp1 pins the Result rung; eo1-3/eop1 (Option) stay green.
            if !matches!(wit, crate::wit_world::WitType::Option(_)) {
                return None;
            }
            // The canonical flattening of `option<T>` is `(disc: i32, payload…)` — a leading disc then the
            // payload leaf/leaves (`vts`). `emit_sum_field` reads the disc at the running leaf cursor.
            param_vts.push(ValType::I32.byte());
            for vt in &vts {
                param_vts.push(vt.byte());
            }
            // The def BORROWS the built sum cell (matches it, copying out any payload); the wrapper — its owner
            // — always reclaims the SHELL after the call (the caller-owns-shell convention the closure-arg path
            // uses: it too builds the sum and drops it post-call). The extracted payload escapes by its own
            // copy/incref, independent of the shell. This slice's result is scalar/unit, so the whole sum never
            // escapes; `drop_after` is unconditionally true here (a consuming/escaping widening is a later slice).
            let _ = body;
            let drop_after = true;
            mem_leaf_params.push(None);
            sum_params.push(Some((rebuild, drop_after)));
            wit_params.push((format!("p{i}"), wit));
            continue;
        }
        let wit = crate::wit_world::ty_natural_wit(gty)?;
        // A memory-bearing leaf param (String/Bytes/list<Int64>) all flatten to (ptr, len) and lift via
        // mem_leaf_params. The def OWNS the arg (callee-owns-args), but a param it only BORROWS (byte-len /
        // List.len / compare) is reclaimed by the OWNER — here the wrapper — so `drop_after` = the param does
        // not escape (a consuming/escaping param declines below, a later slice).
        let mem_kind = match gty {
            Ty::String => Some(serialize::MemLeafKind::Str),
            Ty::Bytes => Some(serialize::MemLeafKind::Bytes),
            // A list<scalar> param (Int8/16/32/64, UInt*, Float32/64, Bool) — build the per-element
            // read+box descriptor; a nested/compound element (list<list>, list<record>) declines (later slice).
            Ty::List(elem) => Some(serialize::MemLeafKind::List(list_scalar_elem(elem)?)),
            _ => None,
        };
        match (mem_kind, gty) {
            (Some(kind), _) => {
                let drop_after =
                    !crate::backend::wasm::select::param_escapes_body(db, body, *binder);
                param_vts.push(ValType::I32.byte());
                param_vts.push(ValType::I32.byte());
                mem_leaf_params.push(Some((kind, drop_after)));
                sum_params.push(None);
            }
            (None, Ty::Int(_) | Ty::Bool | Ty::Float(_)) => {
                param_vts.push(valtype_of(gty)?.byte());
                mem_leaf_params.push(None);
                sum_params.push(None);
            }
            (None, _) => return None, // a compound/unit param — a later slice
        }
        wit_params.push((format!("p{i}"), wit));
    }
    // Require at least one memory-bearing leaf OR sum param (a scalar-only export is the existing bare path,
    // untouched — it falls through to the boundary loop).
    if !mem_leaf_params.iter().any(Option::is_some) && !sum_params.iter().any(Option::is_some) {
        return None;
    }
    // MAX-FLAT-PARAMS GUARD (mirror of the boundary-loop guard): a String/Bytes/list param flattens to two
    // core values (ptr, len), so past 8 such params (or fewer, mixed with scalars) the flattened arity
    // exceeds the canonical-ABI limit (16) and needs the memory-indirect convention this path does not emit.
    // Decline rather than produce an invalid component.
    if param_vts.len() > crate::backend::wasm::wit_ctype::MAX_FLAT_PARAMS {
        return None;
    }
    // SLICE 1 = BORROWED memory-bearing params only. A param the def only borrows is reclaimed by the wrapper
    // (its owner) after the call — a guaranteed 0-leak lift. A param that ESCAPES (is consumed: passed to a
    // consuming op like `Symbol.of`, threaded into a recursive self-call, or moved into the result) needs a
    // consuming-param lift whose reclaim the wrapper cannot guarantee here (the consumer, or a looped-param
    // reclaim, must own it) — decline it to the existing todo, a later slice. Prevents a boundary leak (the
    // #3808 default-enforced live-objects check reds a lifted-but-unreclaimed escaping param).
    if mem_leaf_params
        .iter()
        .any(|m| matches!(m, Some((_, false))))
    {
        return None;
    }
    let any_drop = mem_leaf_params.iter().any(|m| matches!(m, Some((_, true))))
        || sum_params.iter().any(|m| matches!(m, Some((_, true))));
    // RESULT: a scalar the def returns raw (Passthrough) or unit; a compound result is a later slice.
    let (result_vts, wit_result) = match &result_ty {
        Ty::Unit => (Vec::new(), None),
        r @ (Ty::Int(_) | Ty::Bool | Ty::Float(_)) => (
            vec![valtype_of(r)?.byte()],
            Some(crate::wit_world::ty_natural_wit(r)?),
        ),
        _ => return None,
    };
    // Augment the runtime-import set with the lift ops (dedup, APPENDED so the def's op indices — a prefix —
    // are preserved). The added count bumps `import_base` so the def's + wrapper's core func indices line up.
    // `drop` is added only when some borrowed param needs the wrapper reclaim (else byte-identical).
    let mut entry_imports: Vec<&runtime_abi::RtOp> = imports.to_vec();
    let mut lift_ops: Vec<&str> = Vec::new();
    if mem_leaf_params.iter().any(|m| {
        matches!(
            m,
            Some((
                serialize::MemLeafKind::Str | serialize::MemLeafKind::Bytes,
                _
            ))
        )
    }) {
        lift_ops.extend(["bytes-alloc", "bytes-set"]);
    }
    for m in &mem_leaf_params {
        if let Some((serialize::MemLeafKind::List(elem), _)) = m {
            // A list<scalar> builds a vec + boxes each element with its own box op (box-int/float/float32/bool).
            lift_ops.extend(["vec-empty", "vec-push"]);
            lift_ops.push(elem.box_op);
        }
    }
    // A sum (option/result) param builds its cell via `sum-new` plus each arm's payload ops (box-int, etc.).
    for (rebuild, _) in sum_params.iter().flatten() {
        lift_ops.push("sum-new");
        rebuild.arm_true.collect_ops(&mut |op| lift_ops.push(op));
        rebuild.arm_false.collect_ops(&mut |op| lift_ops.push(op));
    }
    if any_drop {
        lift_ops.push("drop");
    }
    for op_name in lift_ops {
        if !entry_imports.iter().any(|o| o.name == op_name) {
            let op = runtime_abi::RUNTIME_OPS
                .iter()
                .find(|o| o.name == op_name)?;
            entry_imports.push(op);
        }
    }
    let added = (entry_imports.len() - imports.len()) as u32;
    let entry_layout = layout.with_import_base(layout.import_base + added);
    let def_abs = entry_layout.abs(def)?;
    let wrapper = serialize::WrapperDesc {
        name: name.clone(),
        param_vts,
        result_vts,
        params: vec![None; params.len()],
        param_slots: vec![None; params.len()],
        sum_params,
        // The plain-export route lifts no payloadless-ENUM param (that is the typed-interface-MEMBER route);
        // all-`None` keeps the wrapper body's disc-remap check inert (byte-neutral passthrough).
        enum_disc_params: vec![None; params.len()],
        mem_leaf_params,
        def_abs,
        result: serialize::ResultLower::Passthrough,
    };
    let typed_func = envelope::TypedFunc {
        name,
        params: wit_params,
        result: wit_result,
    };
    // The def bodies were SELECTED with the ORIGINAL `import_base`; APPENDING `added` lift-op imports shifts
    // every DEFINED func index up by `added`, so each baked def-to-def call (`Lir::Call`/`ReturnCall` whose
    // target is a DEF — index >= the original `import_base`) must be re-shifted by `added`. Otherwise a
    // reachable def call (e.g. a RECURSIVE fn's self-call, which cannot be inlined away) resolves to an
    // APPENDED import op — emitting INVALID wasm ("requires [i64] but callee returns [i32]": the fuzzer's
    // bucket-1 miscompile from a heap-typed entry param whose body calls a recursive Int64 fn). Import-op /
    // host / extern calls resolve by NAME/POSITION (`CallImport`/`CallHostImport`/`CallExternImport`, not
    // `Lir::Call`) and existing indices are preserved by appending, so they are untouched. Byte-identical
    // when `added == 0` (a scalar entry adds no lift ops).
    use crate::backend::wasm::lir::Lir;
    let base = layout.import_base;
    let shifted_funcs: Vec<SelectedFunc>;
    let funcs_ref: &[SelectedFunc] = if added > 0 {
        shifted_funcs = funcs
            .iter()
            .map(|f| {
                let mut f = f.clone();
                for insn in &mut f.code {
                    match insn {
                        Lir::Call(i) | Lir::ReturnCall(i) if *i >= base => *i += added,
                        _ => {}
                    }
                }
                f
            })
            .collect();
        &shifted_funcs
    } else {
        funcs
    };
    let wrapped_core = match serialize::core_module_with_wrappers(
        funcs_ref,
        &entry_imports,
        &[],
        std::slice::from_ref(&wrapper),
        &entry_layout,
    ) {
        Ok(c) => c,
        Err(e) => return Some(Err(Reject::decline(e))),
    };
    let import_name = runtime_import_name();
    Some(Ok(envelope::assemble_bare_typed_with_runtime(
        &wrapped_core,
        std::slice::from_ref(&typed_func),
        &entry_imports,
        &import_name,
    )))
}

/// Like [`scalar_interface_export`] but for a member with a RECORD param: build the boundary WRAPPER descs
/// (the canon lift hands the record's flattened fields; the compiled def wants a value-heap handle, so a
/// wrapper builds the handle then calls the def) plus the [`envelope::TypedInterface`] to emit. Fires only
/// when ≥1 member has a record param (a pure-scalar interface is handled by [`scalar_interface_export`]).
/// MVP: record params of SCALAR fields + scalar/unit results — a `list<u8>`/nested-record/variant/record
/// RESULT still declines (needs the memory + result-lower wrapper, a later slice). `iface` = the FQ export
/// name (`db.component_name`). Returns the wrappers (for `core_module_with_wrappers`) + the interface.
fn record_interface_export(
    db: &mut Db,
    layout: &Layout,
    world_bytes: &[u8],
    iface: &str,
) -> Option<(Vec<serialize::WrapperDesc>, envelope::TypedInterface)> {
    use crate::backend::wasm::lir::valtype_of;
    use crate::backend::wasm::serialize::FieldRebuild;
    use crate::ty::Ty;
    use crate::wit_world::{WitType, parse_target_world};
    // A boundary result type the MVP wrapper can pass straight through (a scalar the def returns raw). A
    // record/variant/list/string result spills to memory and needs the result-lower wrapper (later).
    fn scalar_result_vts(r: &Ty) -> Option<Vec<u8>> {
        match r {
            Ty::Unit => Some(Vec::new()),
            Ty::Record(_)
            | Ty::Tuple(_)
            | Ty::Sum { .. }
            | Ty::List(_)
            | Ty::Map(_, _)
            | Ty::Set(_)
            | Ty::String
            | Ty::Bytes => None,
            other => Some(vec![valtype_of(other)?.byte()]),
        }
    }
    // Build the result-lower + core result valtypes for a member's result. A scalar/unit passes straight
    // through (the def returns it raw); an all-scalar RECORD that SPILLS (flattens to >1 canonical value) is
    // written to a memory return area at canonical offsets, the core returning the area pointer. A flat
    // (1-value) record, a `list<u8>`/nested/variant field, or a WIT-vs-name-lex field-order mismatch → `None`
    // (a later slice, or a soundness decline).
    fn record_result_lower(
        db: &mut Db,
        gr: &Ty,
        wr: &WitType,
    ) -> Option<(serialize::ResultLower, Vec<u8>)> {
        use crate::backend::wasm::serialize::ResultLower;
        use crate::backend::wasm::wit_ctype;
        // A scalar/unit result the def returns raw.
        if let Some(vts) = scalar_result_vts(gr) {
            return Some((ResultLower::Passthrough, vts));
        }
        // A `list<u8>`/Bytes RESULT member: the def returns a value-heap Bytes handle, which the wrapper
        // copies to a `cabi_realloc`'d buffer and writes as the canonical `(ptr,len)` `list<u8>` return
        // (`ResultLower::CopyBytes`). The multi-member typed-interface twin of the single-export bytes
        // provider (the operator's `encode_quoted() -> list<u8>` member). WIT result must be `list<u8>`.
        if matches!(gr.strip_nominal(), Ty::Bytes)
            && matches!(wr, WitType::List(inner) if matches!(**inner, WitType::U8))
        {
            return Some((
                ResultLower::CopyBytes,
                vec![crate::backend::wasm::lir::ValType::I32.byte()],
            ));
        }
        // A payloadless-enum RESULT (all-nullary `Ty::Sum`, `db.is_enum_disc`): the def already returns the
        // raw i32 DISCRIMINANT (select's enum-disc build — no heap handle, no `sum-disc` read), which IS the
        // canonical-ABI core rep of a WIT `enum` (`flatten(Enum) = [I32]`). So it passes straight through as
        // the declared WIT enum — no memory spill. The declared `wr` (`WitType::Enum`, or an all-nullary
        // `WitType::Variant`) becomes `TypedFunc.result` and its enum DEFINED type is emitted + re-exported by
        // the `note` pass. GUARD (mirrors `canon_write_of`'s enum-disc arm): the guest decl-order case names
        // (kebab) MUST equal the WIT case order, else the guest disc would index the wrong WIT case — a
        // reorder needs a runtime disc remap (a later increment), so decline.
        if let Ty::Sum { decl, .. } = gr
            && db.is_enum_disc(*decl)
        {
            use crate::backend::common::export_name::kebab_extern_name;
            let wit_cases: Vec<String> = match wr {
                WitType::Enum(cs) => cs.clone(),
                WitType::Variant(cs) if cs.iter().all(|(_, p)| p.is_none()) => {
                    cs.iter().map(|(n, _)| n.clone()).collect()
                }
                _ => return None,
            };
            let guest_cases: Vec<String> = {
                let dr = db.type_decl_by_occ(*decl)?;
                dr.variants
                    .iter()
                    .map(|v| kebab_extern_name(&v.name))
                    .collect()
            };
            if guest_cases.len() != wit_cases.len() {
                return None; // a genuinely different case set (not just a reorder) — decline
            }
            // `perm[guest_disc] = wit_disc` (the WIT index of the guest's `i`th case, matched by name). A guest
            // case with no WIT name-match declines (`?` on `position`). Identity → Passthrough (order matches);
            // a genuine reorder → EnumRemap, which remaps the disc by name in the wrapper (SHAPE 64, the
            // name-keyed enum-boundary remap — the disc analogue of the record RESULT's write-by-name reorder).
            let mut perm: Vec<u32> = Vec::with_capacity(guest_cases.len());
            for gc in &guest_cases {
                perm.push(wit_cases.iter().position(|wc| wc == gc)? as u32);
            }
            let lower = if perm.iter().enumerate().all(|(i, &p)| p == i as u32) {
                ResultLower::Passthrough
            } else {
                ResultLower::EnumRemap { perm }
            };
            return Some((lower, vec![crate::backend::wasm::lir::ValType::I32.byte()]));
        }
        // A FLAT single-scalar-field record (`record{v: s64}`) flattens to ONE core value, returned DIRECTLY
        // (not by pointer — MAX_FLAT_RESULTS=1), so it does NOT spill. The def returns the record HANDLE; the
        // wrapper reads its one field (`arr-get(handle, 0)` → unbox) and returns that scalar. This is the
        // flat-1-value-record lower the SpillRecord path declined below (`!sig_needs_memory`). Restricted to a
        // record with exactly ONE field that is a scalar (a multi-field-but-1-flat record, e.g. a unit sibling,
        // or a nested-compound single field, is a later slice).
        if let Ty::Record(gfields) = gr.strip_nominal()
            && let WitType::Record(wfields) = wr
            && gfields.len() == 1
            && wfields.len() == 1
            && !wit_ctype::sig_needs_memory(&[], Some(wr))
        {
            let fgty = gfields.values().next()?.clone();
            if let Some((read, wrap_i64, _store)) = scalar_result_read_store(&fgty)
                && let Some(vt) = crate::backend::wasm::lir::valtype_of(&fgty)
            {
                return Some((
                    ResultLower::FlatScalarField {
                        field_cell: 0,
                        read,
                        wrap_i64,
                    },
                    vec![vt.byte()],
                ));
            }
        }
        // Otherwise a compound that SPILLS to memory (flat count > MAX_FLAT_RESULTS): build the recursive
        // canonical writer. A flat 1-value record whose single field is NOT a plain scalar (a nested compound)
        // still declines here — a later slice.
        if !wit_ctype::sig_needs_memory(&[], Some(wr)) {
            return None;
        }
        let write = canon_write_of(db, gr, wr)?;
        Some((
            ResultLower::SpillRecord {
                size: wit_ctype::canonical_size(wr),
                align: wit_ctype::canonical_align(wr),
                write,
            },
            vec![crate::backend::wasm::lir::ValType::I32.byte()],
        ))
    }
    let arenas = crate::codec::decode(world_bytes)?;
    let world = parse_target_world(&arenas, arenas.root)?;
    let export_iface = world.exports.first()?;
    let mut wrappers = Vec::new();
    let mut funcs = Vec::new();
    let mut any_record = false;
    // Set when a member has a TOP-LEVEL memory-bearing byte-leaf param (`Bytes`/`String` lifted via
    // `mem_leaf_params`): the typed-interface wrapper must take over to COPY that param out of linear memory
    // even when its result is a bare scalar (the `decode-check(list<u8>) -> bool` shape) — else the
    // `!any_record && !needs_result_wrapper && !any_mem_leaf_param` gate bails to the scalar path. The
    // param-side twin of `needs_result_wrapper`.
    let mut any_mem_leaf_param = false;
    // Set when a member has a TOP-LEVEL `option<scalar>`/`result<ok,err>` param (a two-variant sum lifted via
    // `sum_params`): the typed-interface wrapper must take over to branch on the boundary disc and build the
    // guest sum cell even when its result is a bare scalar. The sum-param twin of `any_mem_leaf_param`.
    let mut any_sum_param = false;
    // Set when a member has a TOP-LEVEL payloadless-ENUM param (a `Ty::Sum` `is_enum_disc` crossing as a WIT
    // `enum{…}` = one `i32` disc): the typed-interface wrapper must take over to DECLARE + re-export the enum
    // DEFINED type (via the `note` pass) even for an order-matching passthrough, and to REMAP the disc by name
    // when the WIT/guest case orders differ. The enum-param twin of `any_sum_param` (the PARAM side of #7036).
    let mut any_enum_disc_param = false;
    // Set when a member's RESULT spills to memory (a compound result-lower): the typed-interface wrapper must
    // then take over even for an all-scalar-param member, to WRITE the result to the retptr (else the compound
    // result leaks as a raw u32 handle via the provider path). The result-side twin of `any_record`.
    let mut needs_result_wrapper = false;
    for member in &export_iface.members {
        // Bind the WIT member to a guest export by kebab-normalized name (a Cadenza def `onMessage` binds to
        // the WIT `on-message` member — same rule as fields/variants/exports).
        let mk = crate::backend::common::export_name::kebab_extern_name(&member.name);
        let e = layout
            .exports
            .iter()
            .find(|e| crate::backend::common::export_name::kebab_extern_name(&e.name) == mk)?;
        if member.func.params.len() != e.params.len() {
            return None;
        }
        let mut param_vts: Vec<u8> = Vec::new();
        let mut params: Vec<Option<Vec<FieldRebuild>>> = Vec::new();
        let mut param_slots: Vec<Option<Vec<u32>>> = Vec::new();
        // Parallel to `params`: a TOP-LEVEL memory-bearing byte-leaf param (a `Bytes` crossing as `list<u8>`,
        // a `String` crossing as `string`) is lifted via `mem_leaf_params` (copy the boundary `(ptr, len)`
        // out of linear memory into a value-heap handle, passed DIRECTLY as the def arg — the same lift the
        // plain-export bare-wrapper route emits). `None` for a scalar/record param.
        let mut mem_leaf_params: Vec<Option<(serialize::MemLeafKind, bool)>> = Vec::new();
        // Parallel to `params`: a TOP-LEVEL `option<scalar>`/`result<ok,err>` param (a two-variant sum crossing
        // as a native `option<T>`/`result<ok,err>`) is rebuilt via `Some((rebuild, drop_after))` — branch on the
        // boundary disc, `sum-new` the guest cell, passed DIRECTLY as the def arg. `None` for a scalar/record/
        // mem-leaf param.
        let mut sum_params: Vec<Option<(serialize::SumArgRebuild, bool)>> = Vec::new();
        // Parallel to `params`: a TOP-LEVEL payloadless-ENUM param with a WIT/guest case-order MISMATCH carries
        // `Some(inv_perm)` (`inv_perm[wit_disc] = guest_disc`, by name) so the wrapper remaps the boundary disc
        // to the guest disc; an order-matching enum param (or a non-enum param) is `None`. The PARAM twin of
        // `ResultLower::EnumRemap`.
        let mut enum_disc_params: Vec<Option<Vec<u32>>> = Vec::new();
        for ((binder, gty), (_, wty)) in e.params.iter().zip(&member.func.params) {
            match gty {
                Ty::Record(map) => {
                    // Build the record's per-field rebuild in WIT ORDER + the name-lex SLOTS the wrapper
                    // `arr-set`s each field at (`record_param_rebuild`, permuted by name) — so a declaration-
                    // ordered WIT record (the real `message`) lands its fields in the def's name-lex cell
                    // slots. A nested-record field is a `Nested` rebuild; its own order must be name-lex
                    // (`record_fields_rebuild` guard), else decline.
                    let WitType::Record(wfs) = wty else {
                        return None;
                    };
                    let (rebuild, slots) = record_fields_rebuild(db, map, wfs, &mut param_vts)?;
                    params.push(Some(rebuild));
                    param_slots.push(Some(slots));
                    mem_leaf_params.push(None);
                    sum_params.push(None);
                    enum_disc_params.push(None);
                    any_record = true;
                }
                // A TOP-LEVEL `tuple<…>` param is a POSITIONAL record: the canon lift flattens it depth-first
                // (no field names, so no WIT-vs-name-lex permute — identity slots 0..n). Rebuild each element in
                // order via `param_field_rebuild` (scalar/bytes/nested-record/sum, recursing on a compound
                // element), and the wrapper builds the value-heap cell with the SAME `arr-alloc`/`arr-set` shape
                // as a record cell (a Cadenza tuple and record share the array rep — `Core::Tuple`/`Core::Record`
                // build identically). Without this a tuple param declined here and fell through to a scalar/
                // handle-erased emit (a `u32` param the driver could not marshal a tuple arg against).
                Ty::Tuple(gtys) => {
                    let WitType::Tuple(wtys) = wty else {
                        return None;
                    };
                    if gtys.len() != wtys.len() {
                        return None;
                    }
                    let mut rebuild = Vec::with_capacity(gtys.len());
                    for (gt, wt) in gtys.iter().zip(wtys) {
                        rebuild.push(param_field_rebuild(db, gt, wt, &mut param_vts)?);
                    }
                    // Positional slots: element i lands in cell slot i (identity — a tuple has no name-lex order).
                    let slots: Vec<u32> = (0..gtys.len() as u32).collect();
                    params.push(Some(rebuild));
                    param_slots.push(Some(slots));
                    mem_leaf_params.push(None);
                    sum_params.push(None);
                    enum_disc_params.push(None);
                    any_record = true; // a param needs the cell-rebuild wrapper (same gate as a record param)
                }
                // A TOP-LEVEL payloadless-ENUM param (`Ty::Sum` `is_enum_disc`) crosses as a WIT `enum{…}`,
                // whose canonical flatten is a single `i32` case index. The guest def receives the raw `i32`
                // disc (select's enum-disc rep — no heap handle), so the wrapper passes the boundary disc
                // straight through when the WIT/guest case orders MATCH, or REMAPS it by name (`inv_perm[wit]
                // = guest`) when they differ — the PARAM twin of `record_result_lower`'s enum-disc RESULT arm
                // (#7036, SHAPE 64). Guest ↔ WIT by kebab name; a genuinely different case SET declines. No
                // heap handle → no reclaim, no escape concern (a raw `i32` scalar).
                Ty::Sum { decl, .. } if db.is_enum_disc(*decl) => {
                    use crate::backend::common::export_name::kebab_extern_name;
                    let wit_cases: Vec<String> = match wty {
                        WitType::Enum(cs) => cs.clone(),
                        WitType::Variant(cs) if cs.iter().all(|(_, p)| p.is_none()) => {
                            cs.iter().map(|(n, _)| n.clone()).collect()
                        }
                        _ => return None, // WIT type is not an enum/all-nullary variant — decline
                    };
                    let guest_cases: Vec<String> = {
                        let dr = db.type_decl_by_occ(*decl)?;
                        dr.variants
                            .iter()
                            .map(|v| kebab_extern_name(&v.name))
                            .collect()
                    };
                    if guest_cases.len() != wit_cases.len() {
                        return None; // a genuinely different case set (not just a reorder) — decline
                    }
                    // `inv_perm[wit_disc] = guest_disc`: the guest case index of the WIT case at position
                    // `wit_disc`, matched by name. A WIT case with no guest name-match declines (`?`).
                    let mut inv_perm: Vec<u32> = Vec::with_capacity(wit_cases.len());
                    for wc in &wit_cases {
                        inv_perm.push(guest_cases.iter().position(|gc| gc == wc)? as u32);
                    }
                    // The enum crosses as a single `i32` disc.
                    param_vts.push(crate::backend::wasm::lir::ValType::I32.byte());
                    params.push(None);
                    param_slots.push(None);
                    mem_leaf_params.push(None);
                    sum_params.push(None);
                    // Identity → passthrough (the disc IS the guest disc; the `params` `None` arm forwards it);
                    // a genuine reorder → record the remap the wrapper emits before the def call.
                    if inv_perm.iter().enumerate().all(|(i, &g)| g == i as u32) {
                        enum_disc_params.push(None);
                    } else {
                        enum_disc_params.push(Some(inv_perm));
                    }
                    // Either way the typed wrapper must take over — even an identity passthrough needs it to
                    // DECLARE + re-export the enum DEFINED type (the `note` pass) instead of the provider path's
                    // bare `u32` handle (the param twin of the enum-RESULT `needs_result_wrapper`).
                    any_enum_disc_param = true;
                }
                // A TOP-LEVEL `option<scalar>` param crosses as a native component `option<T>`, flattened to
                // `(disc, payload…)` and rebuilt into the guest sum cell via `SumArgRebuild` (branch on the
                // boundary disc → `sum-new`), passed DIRECTLY as the def arg — the sum sibling of the mem-leaf
                // params, mirroring `try_bare_entry_param_component`'s sum-param arm onto the typed-interface-
                // MEMBER route. A `result<ok,err>` declines (its WIT is a `variant`, a disagreeing flatten/disc
                // convention — a later slice, as on the bare route). BORROW-ONLY: the wrapper owns + drops the
                // built shell after the call (the extracted payload escapes by its own copy, independent of the
                // shell); a param whose shell ESCAPES (the def returns/stores it) declines to a later slice.
                Ty::Sum { .. } => {
                    use crate::backend::wasm::envelope::ArgSlot;
                    let Some((slot, vts, rebuild)) =
                        crate::backend::wasm::arg_boundary::fixed_shape_option_scalar_arg(db, gty)
                    else {
                        return None; // not an option/result-shaped two-variant sum — a later slice
                    };
                    // The declared WIT must AGREE with the guest's classified sum shape: `option<T>` ↔ an
                    // Option-shaped classification (Some=disc 1, one payload), `result<ok,err>` ↔ a Result-shaped
                    // one (Ok=disc 0, the ok/err payload JOIN + `wrap_join`). The `SumArgRebuild` already carries
                    // the correct per-shape disc + join (option boundary_true_disc=1, result=0), so the SAME
                    // `emit_sum_field` lift serves both — the bare-entry route only DECLINED Result because it
                    // SYNTHESIZED the WIT as a `variant` (a disagreeing type); here the WIT is DECLARED as
                    // `result<ok,err>`, matching the Result rebuild (the same lift proven for a result<…> record
                    // FIELD, SHAPE 17). A WIT/guest shape mismatch (or an unclassified sum) declines.
                    let wit_agrees = matches!(
                        (wty, &slot),
                        (
                            WitType::Option(_),
                            ArgSlot::OptionScalar(_) | ArgSlot::OptionCompound(_)
                        ) | (WitType::Result { .. }, ArgSlot::Result(_, _))
                    );
                    if !wit_agrees {
                        return None; // WIT type disagrees with the guest sum shape — decline
                    }
                    if crate::backend::wasm::select::param_escapes_body(db, e.body, *binder) {
                        return None; // the built sum shell escapes — a later slice (borrow-only)
                    }
                    // Canonical `option<T>`/`result<ok,err>` flattening: `(disc: i32, payload…)`.
                    param_vts.push(crate::backend::wasm::lir::ValType::I32.byte());
                    for vt in &vts {
                        param_vts.push(vt.byte());
                    }
                    params.push(None);
                    param_slots.push(None);
                    mem_leaf_params.push(None);
                    // drop_after: the def only BORROWS the shell (matches it), so the wrapper (its owner) drops
                    // it after the call — the shell never escapes (checked above), so no double-free.
                    sum_params.push(Some((rebuild, true)));
                    enum_disc_params.push(None);
                    any_sum_param = true;
                }
                // A TOP-LEVEL memory-bearing leaf param — `Bytes` ↔ `list<u8>`, `String` ↔ `string`, or a
                // `list<scalar>` ↔ `list<T>` — all flatten to `(ptr, len)` and lift via `mem_leaf_params`: the
                // wrapper copies the incoming `(ptr, len)` out of linear memory into a value-heap handle and
                // passes it DIRECTLY as the def arg (the host→guest mirror of the import-side marshal; the
                // `decode-check(list<u8>) -> bool` half of the operator's two-export shape §2, plus its
                // `list<scalar>` sibling). Bytes/String copy a raw byte-leaf; a `list<scalar>` builds a
                // value-heap vec element-by-element (`list_scalar_elem` — a boxed-element vec, a DISTINCT value
                // rep from Bytes). BORROW-ONLY slice: the def must not consume/escape the param, so the wrapper
                // (its sole owner) reclaims the lifted handle after the call — a guaranteed 0-leak lift; a
                // consuming/escaping param, or a nested/compound list element, declines to a later slice
                // (mirroring `try_bare_entry_param_component`'s borrow-only rung). WIT must match the guest.
                Ty::Bytes | Ty::String | Ty::List(_) => {
                    let kind = match (gty, wty) {
                        (Ty::Bytes, WitType::List(inner)) if matches!(**inner, WitType::U8) => {
                            serialize::MemLeafKind::Bytes
                        }
                        (Ty::String, WitType::String) => serialize::MemLeafKind::Str,
                        // A `list<scalar>` (Int8/16/32/64, UInt*, Float32/64, Bool) — the per-element read+box
                        // descriptor; a nested/compound element (list<list>, list<record>) declines via `?`.
                        // A `Ty::List(UInt8)` crossing WIT `list<u8>` is a genuine `List UInt8` (boxed-u8 vec),
                        // NOT `Bytes` (packed byte-leaf) — distinct value reps behind the same WIT type.
                        (Ty::List(elem), WitType::List(_)) => {
                            serialize::MemLeafKind::List(list_scalar_elem(elem)?)
                        }
                        _ => return None, // a guest/WIT type mismatch — decline
                    };
                    if crate::backend::wasm::select::param_escapes_body(db, e.body, *binder) {
                        return None; // a consumed/escaping mem-leaf param — a later slice
                    }
                    // A mem-leaf param flattens to `(ptr, len)` = two i32 core values.
                    let i32b = crate::backend::wasm::lir::ValType::I32.byte();
                    param_vts.push(i32b);
                    param_vts.push(i32b);
                    params.push(None);
                    param_slots.push(None);
                    mem_leaf_params.push(Some((kind, true))); // drop_after = borrowed (checked above)
                    sum_params.push(None);
                    enum_disc_params.push(None);
                    any_mem_leaf_param = true;
                }
                Ty::Map(_, _) | Ty::Set(_) => {
                    return None;
                } // needs memory / a deeper wrapper — later
                scalar => {
                    // a scalar param passes straight through (no rebuild).
                    param_vts.push(valtype_of(scalar)?.byte());
                    params.push(None);
                    param_slots.push(None);
                    mem_leaf_params.push(None);
                    sum_params.push(None);
                    enum_disc_params.push(None);
                }
            }
        }
        // A signature whose flattened params EXCEED the canonical limit spills the whole param tuple to a
        // single pointer — the register-passing wrapper (which reads each field from its own flattened param)
        // does not handle that; decline (later slice). Within the limit, fields cross individually flattened.
        if param_vts.len() > crate::backend::wasm::wit_ctype::MAX_FLAT_PARAMS {
            return None;
        }
        let (result_lower, result_vts) = record_result_lower(db, &e.result, &member.func.result)?;
        // A member whose RESULT spills to memory (a compound result-lower, not a passthrough scalar) needs
        // this wrapper for the result WRITE even when its params are all bare scalars — the scalar-param +
        // compound-result case (co02: `f(x:s64) -> record{…}`). Without this the `!any_record` gate below bails
        // and the export falls through to the provider path, which hands the compound result back as its raw
        // u32 handle (a leaked pointer, not the lifted value). The result-write machinery (`SpillRecord` /
        // `canon_write_of`) is identical to the record-param route — only the param-shape gate excluded it.
        if matches!(result_lower, serialize::ResultLower::SpillRecord { .. }) {
            needs_result_wrapper = true;
        }
        // A `list<u8>`/Bytes result member (CopyBytes) likewise needs this wrapper for the bytes copy-out —
        // else the `!any_record && !needs_result_wrapper` gate bails and the export falls through to the
        // provider path, which crosses the Bytes as a raw `u32` handle instead of the declared `list<u8>`.
        if matches!(result_lower, serialize::ResultLower::CopyBytes) {
            needs_result_wrapper = true;
        }
        // A flat single-scalar-field record result (FlatScalarField) needs this wrapper to READ the field off
        // the def's record handle + return the scalar — else the export falls through to the provider path,
        // which crosses the record as a bare `u32` handle instead of the flattened scalar the WIT declares.
        if matches!(result_lower, serialize::ResultLower::FlatScalarField { .. }) {
            needs_result_wrapper = true;
        }
        // A payloadless-ENUM result is a `Passthrough` (raw i32 disc), NOT a `SpillRecord`, but it STILL needs
        // this typed wrapper: without it the `!any_record && !needs_result_wrapper` gate bails and the export
        // falls through to the PROVIDER path, which crosses the enum as a bare `u32` handle (`extern_abi_val_type`)
        // instead of the declared WIT `enum{…}` (the enum-export twin of the compound-result case above). A plain
        // scalar `Passthrough` must NOT trip this — a pure-scalar interface stays on `scalar_interface_export`.
        if matches!(&e.result, Ty::Sum { decl, .. } if db.is_enum_disc(*decl)) {
            needs_result_wrapper = true;
        }
        let def_abs = layout.abs(e.def)?;
        // `mem_leaf_params` was built per-param in the loop above: a TOP-LEVEL `Bytes`/`String` leaf param
        // lifts via `Some((kind, drop_after))`; a scalar/record param is `None` (a String/Bytes leaf INSIDE a
        // record rides via `FieldRebuild::BytesLeaf` instead).
        wrappers.push(serialize::WrapperDesc {
            name: member.name.clone(),
            param_vts,
            result_vts,
            // A TOP-LEVEL `option<scalar>` param lifts via `Some((rebuild, drop_after))` (built per-param in
            // the loop above); a scalar/record/mem-leaf param is `None` (a sum INSIDE a record rides via the
            // record-field `FieldRebuild::Sum` route instead).
            sum_params,
            enum_disc_params,
            params,
            param_slots,
            mem_leaf_params,
            def_abs,
            result: result_lower,
        });
        funcs.push(envelope::TypedFunc {
            name: member.name.clone(),
            params: member.func.params.clone(),
            result: match &member.func.result {
                WitType::Unit => None,
                r => Some(r.clone()),
            },
        });
    }
    // A pure-scalar interface (all-scalar params AND scalar/unit results) is handled (no wrapper) by
    // scalar_interface_export; take over when a member needs a wrapper — a RECORD param (rebuild), a
    // TOP-LEVEL memory-bearing byte-leaf param (`Bytes`/`String`/`list<scalar>` copy-in), a TOP-LEVEL
    // `option<scalar>` param (sum rebuild), OR a spilled COMPOUND result (retptr write). A scalar-param +
    // compound-result member needs only the last.
    if !any_record
        && !any_mem_leaf_param
        && !any_sum_param
        && !any_enum_disc_param
        && !needs_result_wrapper
    {
        return None;
    }
    // The exported instance must EXPORT each named (record/variant) type its funcs reference, or the
    // component-model validator rejects the func export ("instance not valid to be used as export"). The
    // world declares these inline (TargetWorld does not yet carry named types — the modeling gap), so
    // synthesize a name per DISTINCT one; `assemble_typed_interface`'s dedup makes each func param reference
    // the same exported type.
    // Recurse into every nested type so a distinct name is synthesized for a NESTED record/variant too (the
    // message's `sender` record inside `message`) — children first, so an outer type's field references an
    // already-noted inner named type; the instance must export ALL of them or the validator rejects the
    // export ("instance not valid to be used as export").
    fn note(t: &WitType, types: &mut Vec<(String, WitType)>) {
        match t {
            WitType::Record(fs) => fs.iter().for_each(|(_, ft)| note(ft, types)),
            WitType::Variant(cs) => cs.iter().for_each(|(_, p)| {
                if let Some(p) = p {
                    note(p, types)
                }
            }),
            WitType::Tuple(es) => es.iter().for_each(|e| note(e, types)),
            WitType::List(e) => note(e, types),
            WitType::Option(i) => note(i, types),
            WitType::Result { ok, err } => {
                if let Some(o) = ok {
                    note(o, types)
                }
                if let Some(e) = err {
                    note(e, types)
                }
            }
            _ => {}
        }
        // A named type the instance must re-export: record, variant, enum (an all-nullary variant, e.g. the
        // response `answer`'s `error`), or flags. A structural `list`/`option`/`result`/`tuple` is not named.
        if matches!(
            t,
            WitType::Record(_) | WitType::Variant(_) | WitType::Enum(_) | WitType::Flags(_)
        ) && !types.iter().any(|(_, u)| u == t)
        {
            types.push((format!("t{}", types.len()), t.clone()));
        }
    }
    let mut types: Vec<(String, WitType)> = Vec::new();
    for f in &funcs {
        for (_, pty) in &f.params {
            note(pty, &mut types);
        }
        if let Some(r) = &f.result {
            note(r, &mut types);
        }
    }
    Some((
        wrappers,
        envelope::TypedInterface {
            name: iface.to_string(),
            types,
            funcs,
        },
    ))
}

/// §3c — emit ANY WIT bytes-provider export member: a member the target WIT declares `list<u8> -> list<u8>`
/// crosses as CANONICAL VALUE-FORM bytes rather than exchanging a `u32` value-heap handle. NOTHING here is
/// specific to a reducer/fold — the fold's `apply` is merely the FIRST caller; the emit is driven purely off
/// the member's DECLARED WIT signature. The RESULT side reuses the runtime `value-encode` walker (as
/// [`emit_recursive_sum_resource`] does): the compound result is rendered to a value-form document and copied
/// out as `list<u8>`. The PARAM side is the inverse — the single compound `list<u8>` param is lifted to a
/// runtime Bytes handle and value-DECODEd to the compound rep before the member body runs.
///
/// This is the compiler↔platform-separation boundary: the compiler knows no specific contract — which members
/// cross as bytes is decided by [`world_bytes_crossing_export`] from the declared target WIT world (`db.wit_world`)
/// via [`crate::wit_world::bridge_decision`], and both descriptors come from [`crate::lower::sum_shape_descriptor`]
/// over the declared types, the same source the runtime value-encode escapes use — so the wire form is
/// byte-identical to a constant/collection value form. The "exactly one compound parameter" shape below is a
/// current-slice constraint of the declared signature (widened later), NOT a fold assumption.
fn emit_bytes_provider_member(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
    iface: &str,
    spans: Option<&crate::spans::SpanData>,
) -> Result<Vec<u8>, Reject> {
    // §2d STATIC-DATA on the provider path: BOTH the pure (`bytes_roundtrip_core_module`) and host-fused
    // (`bytes_roundtrip_host_core_module`) assemblers now emit their OWN build-once GLOBAL/START sections, so a
    // constant Bytes/String/Tuple/Record/small-List in the reducer body — including its RETURNED effect-list's
    // constant parts — hoists to a `global.get` immortal static built once per spawn (the platform per-event
    // amortization), whether or not the member uses host ops. No strip: the real static tables flow through to
    // selection (routing the constants to `global.get`) and to the assembler (which declares those globals).
    // (Probe host imports up front only to reuse the walk below.)
    let mut host_imports_probe: Vec<host::HostImport> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        host::collect_host_imports(db, body, &mut host_imports_probe);
    }
    // The member's declared param/result types drive both descriptors — the compiler reads them off the
    // export plan, never off a hard-coded contract shape.
    let (params, result_ty) = {
        let plan = layout
            .exports
            .iter()
            .find(|e| e.def == export_def)
            .ok_or_else(|| {
                Reject::decline("the bytes-crossing provider export is not in the emission order")
            })?;
        (plan.params.clone(), plan.result.clone())
    };
    // RESULT side: the runtime value-encode walker renders the compound result to a BARE value document
    // (root head `list`/`record`, `= name value` fields) — the fold boundary carries bare value forms both
    // directions (the kernel's `parse_effect_list` reads bare; the type is statically known both sides), so
    // root at the BARE inner shape, NOT `sum_shape_descriptor`'s `Named`/`Framed` frame (which would make
    // value-encode emit the `(: value Type)` typed-document the kernel rejects). v-ah+v-runtime ruling
    // 2026-08-12: value-encode frames only a Named/Framed descriptor root.
    let result_desc = crate::lower::bare_shape_descriptor(db, &result_ty).ok_or_else(|| {
        Reject::decline(
            "a bytes-crossing provider export's result has no value-form shape descriptor (not a \
             sum/collection/compound the runtime value-encode walker renders)",
        )
    })?;
    // PARAM side: exactly one compound param, value-DECODEd from the incoming document (a current-slice
    // shape constraint of the declared signature — a multi-param or scalar-param member is a later slice).
    let [(_, param_ty)] = &params[..] else {
        return Err(Reject::decline(
            "a bytes-crossing provider export must take exactly one compound parameter (the value-form \
             document the host passes the member)",
        ));
    };
    let param_ty = param_ty.clone();
    // PARAM side (SYMMETRIC with the result): value-DECODE the incoming BARE Event document. The kernel's
    // `build_event_document` (val_to_ast) emits the BARE value form (name-head `record`, `= name value`
    // fields), so the param descriptor must root at the BARE inner shape too — NOT `sum_shape_descriptor`'s
    // `Framed` wrap (which a Record CONTAINING a sum/option field takes), against which value-decode would
    // expect the `(: value Type)` typed-doc and reconstruct EMPTY fields from the kernel's bare Event
    // (v-pc issue 035391). The fold boundary is bare BOTH directions (v-ah+v-runtime ruling 2026-08-12).
    let param_desc = crate::lower::bare_shape_descriptor(db, &param_ty).ok_or_else(|| {
        Reject::decline(
            "a bytes-crossing provider export's parameter has no value-form shape descriptor (not a \
             sum/collection/compound the runtime value-decode walker reconstructs)",
        )
    })?;
    // ── EMIT ── select the guest's funcs + build the member's `(ptr,len)->retptr` core module (value-decode
    // the incoming document, run the member body, value-encode the result) + the provider component. FIRST
    // full-A slice: the CLOSURE-free, HOST/PEER-import-free member (the fold's pure `apply` is the first such
    // caller). A member whose body calls a host import (e.g. `kv`) or a peer, or uses first-class closures, is
    // a later slice — decline cleanly (gate-neutral) rather than mis-emit.
    let _ = spans; // no debug sections in the bytes-roundtrip core yet
    let member_name = db.defs[export_def].name.to_string();

    let mut used: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_module_used_ops(db, layout, &mut used)?;
    for op in [
        "value-decode",
        "value-encode",
        "bytes-alloc",
        "bytes-set",
        "bytes-len",
        "bytes-get",
        "drop",
    ] {
        used.insert(op);
    }
    // PROVIDER static hoisting (both pure + host-fused): the `start` init builds each hoisted constant with the
    // same ops the inline build used, then `mark-immortal`s it — the init isn't in any body (the body routes to
    // `global.get`), so force the full init op set when the static tables are non-empty (bytes-alloc/set already
    // forced above). Mirrors `core_module_impl`'s pre-pass forcing; no-op (byte-identical) when nothing hoisted.
    if !layout.static_bytes.is_empty() || !layout.static_compounds.is_empty() {
        for op in [
            "arr-alloc",
            "arr-set",
            "box-int",
            "box-bool",
            "vec-of-arr",
            "mark-immortal",
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
    // A SPILLED-COMPOUND host result's LIFT (`select::emit_result_lift`) CONSTRUCTS a value-heap value with
    // runtime ops the reducer's own Core may never use (it only destructures the lifted value) — so
    // `collect_module_used_ops` misses them. Declare EXACTLY the ops the lift emits, walking each op's WIT
    // result type in lockstep with `emit_result_lift` (Bytes → bytes-alloc/bytes-set; List → arr-alloc/
    // arr-set/vec-of-arr; Tuple/Record → arr-alloc/arr-set; option → sum-new). This GENERAL walk covers
    // `option<list<u8>>`, `list<tuple<…>>`, bare `list<u8>`, AND `list<list<u8>>` (graph.neighbors) with no
    // per-shape branch.
    {
        let mut host_probe: Vec<host::HostImport> = Vec::new();
        for &def in &layout.order {
            let body = def_body(db, def)?;
            host::collect_host_imports(db, body, &mut host_probe);
        }
        for hi in &host_probe {
            if let Some(ty) = hi.spilled_result.clone() {
                declare_result_lift_ops(db, &ty, &mut used);
            }
        }
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

    // A PEER-bound effect crosses as a handle over a SHARED runtime — not the host-fused bytes path; decline.
    if !db.effect_bindings.is_empty() {
        return Err(Reject::unsupported(
            "a bytes-crossing member bound to a peer interface is not supported (the host-fused \
             bytes-roundtrip path handles host imports, not peer-bound effects)",
        ));
    }
    // A HOST op whose DECLARED signature has no host-boundary form declines. The host-fused path LIFTS an
    // `option<list<u8>>` result into a value-heap `Option<Bytes>` (the kv.get select lift), so it is ALLOWED
    // here (`true`); a String result or any OTHER compound result/argument still declines.
    for &def in &layout.order {
        let body = def_body(db, def)?;
        if let Some((op, _, ty)) = host::first_unrepresentable_host_op(db, body, true) {
            return Err(Reject::declined(
                crate::diag::DeclineId::WasmBytesCrossingHostOpNoBoundaryForm,
                format!(
                    "a bytes-crossing member calls host op `{op}` whose `{ty}` signature has no host-boundary \
                     form (a compound host result like `option<list<u8>>` needs the host compound-result \
                     ABI); a representable host op is emitted"
                ),
            ));
        }
    }
    // Reuse the up-front host-import probe (same walk) that decided the static-strip above.
    let host_imports = host_imports_probe;

    // Host imports FIRST (core funcs 0..h), then runtime (h..h+k) — so `CallHostImport(i)=call i` resolves;
    // the pure (h=0) path keeps import base k, byte-identical. A host `list<u8>` arg copies to the shared
    // memory, so a host-fused reducer's layout carries `host_needs_memory`.
    let h = host_imports.len() as u32;
    let k = imports.len() as u32;
    // A host op returning `option<list<u8>>` (kv.get) makes the guest core module IMPORT `cabi_realloc`
    // (func index h+k) for the select lift's retptr alloc — shifting the DEFINED funcs by +1, so the import
    // base (which `member_body_abs`/self-calls resolve against) must account for it. No option-result op
    // (kv.put) → base h+k, byte-identical.
    let needs_realloc = host_imports.iter().any(|hi| hi.spilled_result.is_some()) as u32;
    let layout = layout
        .with_import_base(h + k + needs_realloc)
        .with_host_needs_memory(host::set_needs_memory(&host_imports));
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
    if !escape_lifted_table(layout).is_empty() {
        return Err(Reject::unsupported(
            "a bytes-crossing member using first-class closures is not supported (the bytes-roundtrip \
             core module lays no funcref table)",
        ));
    }
    let member_body_abs = layout
        .abs(export_def)
        .ok_or_else(|| Reject::decline("the bytes-crossing member is not in the emission order"))?;

    // PRE-ENCODE (Axis 2): if the member's RESULT is a compile-time constant (independent of the incoming
    // event), precompute its canonical BARE value-form bytes — byte-identical to the runtime `value-encode`
    // op's output for the same constant (canonical codec). The apply body then just writes these bytes and
    // returns, skipping value-decode + body + per-event value-encode entirely. `None` for a runtime-dependent
    // result (the usual reducer) — byte-identical to before.
    let const_result = {
        let body = def_body(db, export_def)?;
        crate::lower::constant_value_form_bare(db, body)
    };

    if host_imports.is_empty() {
        // PURE (host/peer-import-free) member — owns its memory, imports only the runtime.
        let core = serialize::bytes_roundtrip_core_module(
            &funcs,
            &imports,
            member_body_abs,
            &param_desc,
            &result_desc,
            &member_name,
            layout,
            const_result.as_deref(),
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_bytes_roundtrip_provider(
            &core,
            iface,
            &member_name,
            &imports,
            &runtime_import_name(),
        ));
    }

    // HOST-FUSED (GAP B): the member body calls a host interface (e.g. `kv`). The core module imports a
    // SHARED memory (host `list<u8>` args lower against it before the program instantiates); the envelope
    // provides the shared-memory module + imports the host interface (at the FQ name the WORLD declares,
    // matching the host bind) + the runtime. The kv import interface name is DERIVED from the world —
    // generic, no hard-coded host contract.
    let kv_iface = {
        let world_bytes = db.wit_world.as_deref().ok_or_else(|| {
            Reject::decline(
                "a host-fused bytes member requires a target WIT world (the host interface name is \
                 declared there)",
            )
        })?;
        let arenas = crate::codec::decode(world_bytes)
            .ok_or_else(|| Reject::decline("the target WIT world did not decode"))?;
        let world = crate::wit_world::parse_target_world(&arenas, arenas.root)
            .ok_or_else(|| Reject::decline("the target WIT world did not parse"))?;
        // FIRST full-A host slice: a single host import interface (the reducer's `kv`). Its component-import
        // name is the world's import interface name (FQ). A multi-host-interface member is a later slice.
        let [iface_import] = &world.imports[..] else {
            return Err(Reject::unsupported(
                "a host-fused bytes member with other than exactly one target-world import interface is \
                 not supported (a single host interface, e.g. kv, is supported)",
            ));
        };
        iface_import.name.clone()
    };
    // The spilled-RESULT component defined types, built GENERALLY from each op's WIT result type (one
    // recursion, no per-shape blocks). `result_defs` laid after the shared `(list u8)`; `result_crefs` the
    // per-op functype result reference. A bare `list<u8>` result interns to the `(list u8)` at index 0.
    let (needs_list, result_defs, result_crefs, arg_list_crefs) =
        build_host_result_types(db, &host_imports);
    // ENUM host-call args (graph.neighbors' `dir`): a nominal `enum` DEFINED type laid (DEFINE+EXPORT) after
    // `(list u8)` + the spilled-result defined types — `base` = that prepend count. A SINGLE distinct enum
    // type this slice; the op's `comp_functype` references its EXPORTED index. Composes WITH a spilled result
    // (base accounts for `result_defs`). (Record host-args take the typed-interface path, not this one.)
    let enum_param_cases: Vec<&Vec<String>> = host_imports
        .iter()
        .flat_map(|h| &h.params)
        .filter_map(|p| match p {
            host::HostParam::Enum(cases) => Some(cases),
            _ => None,
        })
        .collect();
    let base = needs_list as u32 + result_defs.len() as u32;
    let (nominal_defs, nominal_export_idx): (Vec<Vec<u8>>, u32) = if enum_param_cases.is_empty() {
        (Vec::new(), 0)
    } else if enum_param_cases.iter().all(|c| *c == enum_param_cases[0]) {
        let bytes = crate::backend::wasm::wit_ctype::emit_cdef(
            &crate::backend::wasm::wit_ctype::CDef::Enum(enum_param_cases[0].clone()),
        );
        (vec![bytes], base + 1)
    } else {
        return Err(Reject::unsupported(
            "a bytes-provider host set with more than one distinct enum parameter type is not supported \
             (a single enum type per set)",
        ));
    };
    let host_fns: Vec<envelope::HostFn> = host_imports
        .iter()
        .enumerate()
        .map(|(i, hf)| envelope::HostFn {
            op: hf.op.clone(),
            comp_functype: host_op_comp_functype(
                hf,
                0,
                nominal_export_idx,
                &arg_list_crefs[i],
                result_crefs[i].clone(),
            ),
            has_list_param: hf
                .params
                .iter()
                .any(|p| matches!(p, host::HostParam::Bytes)),
            core_functype: Vec::new(),
        })
        .collect();
    let core = serialize::bytes_roundtrip_host_core_module(
        &funcs,
        &imports,
        &host_imports,
        member_body_abs,
        &param_desc,
        &result_desc,
        &member_name,
        layout,
    )
    .map_err(Reject::decline)?;
    Ok(envelope::assemble_bytes_roundtrip_host_provider(
        &core,
        iface,
        &member_name,
        &host_fns,
        &kv_iface,
        &imports,
        &runtime_import_name(),
        needs_list,
        &result_defs,
        &nominal_defs,
    ))
}

/// The program's runtime import name: the interface (`cadenza:runtime/heap`) pinned to the semver
/// `0.0.0` with the runtime's content hash as build-metadata (`+<hash>`) — the versioned form `cdz-run`
/// matches against the composed runtime. Both parts come from the generated ABI table, so a runtime
/// change re-pins it. The interface identity (`RUNTIME_IFACE`) is fixed for every program the generation
/// emits, and the content hash (`REQUIRED_RUNTIME_HASH`) records the EXACT runtime the component
/// requires, right in the import name embedded in the emitted component — so the artifact is
/// self-describing and its execution is deterministic in the (program, runtime content address) pair.
///
/// This single well-known interface is the ONE import exempt from the capability manifest: importing it
/// constructs and inspects the program's runtime values and adds nothing to the escaping effect row, and
/// it is a closed allowlist of exactly one — every OTHER import a program carries is a host function and
/// therefore a capability the manifest enumerates.
//= spec/capabilities/capabilities-and-effects.md#the-value-heap-runtime-is-the-one-import-that-is-not-a-capability
//# The single, well-known value-heap runtime interface a program imports to construct and inspect its runtime values MUST NOT be counted as a host function, so that importing it adds nothing to the escaping effect row and a program that imports only it remains pure with an empty manifest.
//= spec/capabilities/capabilities-and-effects.md#the-value-heap-runtime-is-the-one-import-that-is-not-a-capability
//# Exactly one such runtime interface MUST be exempt — the value-heap runtime the compiler emits programs against, fixed at the declared-default location — and every other import a program carries MUST be treated as a host function and therefore a capability, so that the exemption is a closed allowlist of one and not an open class of non-effect imports.
///
//= spec/contracts/component-abi.md#the-value-heap-runtime-crosses-by-a-well-known-import
//# A derived program MUST reach its runtime values — constructing a compound value and inspecting a value's contents — through the single, well-known value-heap runtime interface it imports, rather than by open-coding a value heap into its own component, so that the heap representation is one shared artifact the compiler emits programs against.
//= spec/contracts/component-abi.md#the-value-heap-runtime-crosses-by-a-well-known-import
//# The identity of that runtime interface MUST be fixed at the declared-default location and MUST be the same for every program a generation emits, so that any conforming host can satisfy the import and the interface is a stable part of the ABI rather than a per-program choice.
///
//= spec/contracts/component-abi.md#the-value-heap-runtime-crosses-by-a-well-known-import
//# The concrete runtime a program is emitted against MUST be identified by the content address of that runtime component, so that a program's execution is deterministic in the pair (program, runtime content address) and a runtime built from different bytes is a distinct, explicitly-identified environment rather than a silent substitution (reproducible-derivation.md §Derivation Is A Function Of Source And Toolchain).
///
//= spec/contracts/component-abi.md#the-emitted-component-records-its-required-runtime
//# A derived program MUST record, in the emitted component itself, the content address of the runtime it requires, so that the component is self-describing: what interface it imports and which exact runtime implementation satisfies that import both travel with the artifact.
///
//= spec/contracts/component-abi.md#the-emitted-component-records-its-required-runtime
//# A compiler MUST be built against a fixed runtime interface and a fixed runtime content address, so that which runtime a generation targets is a property of the compiler rather than a per-invocation choice, and the compiler and its runtime are one versioned pair.
///
//= constitution.md#vi-the-runnable-form-is-a-verified-content-addressed-component
//# The compiler MUST emit a component that names the exact host-interface version it targets.
//= constitution.md#vi-the-runnable-form-is-a-verified-content-addressed-component
//# The runnable form of a program MUST be a content-addressed binary component behind a versioned host interface.
///
//= spec/contracts/reproducible-derivation.md#derivation-is-a-function-of-source-and-toolchain
//# The identity of the value-heap runtime a program is emitted against MUST be the content address of that runtime component, so that "which runtime" is a hash rather than a version label and a program's observable behavior — which depends on the runtime's construction, storage, and reclamation of values — is pinned to exact bytes (component-abi.md §The Value-Heap Runtime).
/// The recognizable prefix the runtime-hash flag-day renders into `runtime_abi.rs`'s committed hash
/// fallbacks (`0PLACEHOLDERnixInjectsTheReal…`) once part a flips them. nix (and native injection) set
/// `CDZ_RUNTIME_HASH` to the REAL content hash, which `option_env!` prefers; the committed placeholder is
/// only ever seen when the env was NOT injected — a bare `cargo build --bin cdz` or a missed native build
/// site. A hash carrying this prefix is therefore never a valid runtime reference.
const RUNTIME_HASH_PLACEHOLDER_PREFIX: &str = "0PLACEHOLDER";

/// Whether `hash` is the flag-day placeholder sentinel (the committed fallback leaked through because
/// `CDZ_RUNTIME_HASH` was not injected). Pure prefix check — the safe-by-construction discriminator.
fn hash_is_placeholder(hash: &str) -> bool {
    hash.starts_with(RUNTIME_HASH_PLACEHOLDER_PREFIX)
}

/// The versioned runtime import name (`cadenza:runtime/heap@0.0.0+<hash>`) stamped into every component
/// that references the value-heap runtime. This is the SINGLE stamp site for `REQUIRED_RUNTIME_HASH`.
///
/// STAMP-GUARD (runtime-hash flag-day (c), v-nix-elevated REQUIRED): if the committed hash is the
/// `0PLACEHOLDER` sentinel, the real runtime content hash was NOT injected (`CDZ_RUNTIME_HASH` unset —
/// a bare-cargo `cdz` or a missed native build site). Stamping it would emit a component importing a
/// placeholder-named runtime that resolves to the wrong/absent bytes and re-ships opaque heap traps
/// (the #5687 revert: 3770 traps). HARD-ERROR here instead, converting a silent miscompile into a
/// legible build-time failure. Fires ONLY on programs that reference the runtime (the ones that would
/// trap); programs that never call `runtime_import_name` are unaffected — safe by construction.
fn runtime_import_name() -> String {
    let hash = runtime_abi::REQUIRED_RUNTIME_HASH;
    assert!(
        !hash_is_placeholder(hash),
        "cdz build integrity [runtime-hash flag-day guard (c)]: REQUIRED_RUNTIME_HASH is the placeholder \
         sentinel `{hash}` — the real value-heap-runtime content hash was NOT injected. Set CDZ_RUNTIME_HASH \
         (i.e. build cdz via nix, not a bare `cargo build --bin cdz`). Refusing to stamp a placeholder \
         runtime import: it would emit a component that traps at run time."
    );
    format!("{}@0.0.0+{}", runtime_abi::RUNTIME_IFACE, hash)
}

#[cfg(test)]
mod stamp_guard_tests {
    use super::{hash_is_placeholder, runtime_import_name};
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};

    #[test]
    fn placeholder_discriminator_matches_only_the_sentinel() {
        // The flag-day committed placeholders (all `0PLACEHOLDER…`).
        assert!(hash_is_placeholder(
            "0PLACEHOLDERnixInjectsTheRealRUNTIMEhash00000"
        ));
        assert!(hash_is_placeholder(
            "0PLACEHOLDERnixInjectsTheRealDEBUGruntimehash"
        ));
        assert!(hash_is_placeholder(
            "0PLACEHOLDERnixInjectsTheRealNFCcomponenthash"
        ));
        // A REAL content hash (base62, never `0PLACEHOLDER…`) is NOT flagged.
        assert!(!hash_is_placeholder(
            "0541d3cotzDfAPqHfDMv8Y3uXW4TaIN9LSPEwwzL5oNiJ"
        ));
        assert!(!hash_is_placeholder(
            "05WQLypkiSML9PDP1Y7wpqJEsYukcgOZI9QZzXgfnrlFm"
        ));
        assert!(!hash_is_placeholder(""));
    }

    #[test]
    fn stamp_of_the_real_committed_hash_does_not_trip_the_guard() {
        // With the real committed hash (or a nix-injected one), the guard is dormant and the stamp is the
        // versioned runtime import. Only a `0PLACEHOLDER…` fallback (env not injected) hard-errors — that
        // path fires exactly on programs referencing the runtime (see runtime_import_name's guard doc).
        assert!(
            !hash_is_placeholder(REQUIRED_RUNTIME_HASH),
            "committed hash must be real, not placeholder"
        );
        let name = runtime_import_name();
        assert!(name.starts_with(RUNTIME_IFACE));
        assert!(name.contains("@0.0.0+"));
    }
}

/// The AST body occurrence of definition `def`, or a decline if it is malformed (no body).
fn def_body(db: &Db, def: usize) -> Result<crate::ast::StructId, Reject> {
    db.defs[def]
        .body
        .ok_or_else(|| Reject::decline(format!("definition `{}` has no body", db.defs[def].name)))
}

/// The escaping export's `make`-forwarded parameters, as BOTH the core valtypes (`make`'s wasm params +
/// body `local.get`s) and the component boundary bytes (`make`'s component functype params) — the two
/// reps `make` needs, built from the ONE param list so they cannot diverge. A NULLARY export gives two
/// empty vecs (the classic `make() -> own<t>`, byte-identical to before); a PARAMETERIZED export gives
/// its scalar params (`make(a, …) -> own<t>`), so a heap value that depends on the host's arguments
/// crosses via the resource escape. Read off the export plan (the same params the export's selected body
/// takes). A NON-scalar param declines — a parameterized heap return forwards scalar params only this
/// increment (a compound-param export is a later widening), the SAME boundary the closure-resource
/// `make(k)` and the plain multi-export path draw for their params.
fn export_make_params(
    db: &mut Db,
    layout: &Layout,
    export_def: usize,
) -> Result<MakeParams, Reject> {
    let params = match layout.export_plan(export_def) {
        Some(e) => e.params.clone(),
        None => crate::layout::def_params(db, export_def),
    };
    // Each parameter is either a genuine SCALAR (Int/Bool/Float — one flattened leaf, forwarded directly)
    // or a fixed-shape scalar TUPLE/RECORD (crosses as a native component `tuple<…>` the canonical ABI
    // flattens into a run of scalar leaves, which `make` rebuilds into the cell). Params compose freely:
    // any mix of scalar + compound, any number of each. The flattened leaves run left-to-right across all
    // params; each compound slot's rebuild reads its own run (a leaf cursor threaded across slots at emit).
    let mut leaf_vts = Vec::new();
    let mut slots: Vec<MakeSlot> = Vec::new();
    for (_, t) in &params {
        match t.strip_nominal() {
            crate::ty::Ty::Tuple(_) | crate::ty::Ty::Record(_) => {
                // A fixed-shape compound param — including NESTED ones (a tuple-of-tuples, a record with a
                // tuple field): `nested_fixed_shape_tuple_arg` flattens the whole tree to its depth-first
                // scalar leaves, the recursive `FieldRebuild` that rebuilds the (nested) cell, and the
                // `TupleFieldShape` tree the envelope mints the nested `tuple<…>` types from. A VARIABLE-
                // length field (a `List`/`Map`/`Set` inside the compound) or a non-scalar leaf → `None`,
                // which declines (a runtime-decoded field param is a separate, harder gap).
                let Some((_field_bytes, field_vts, rebuild_fields, shape)) =
                    nested_fixed_shape_tuple_arg(t)
                else {
                    return Err(Reject::unsupported(format!(
                        "a parameterized heap-return export's compound parameter `{}` is not a fixed-shape \
                         tuple/record of scalars (a variable-length field — a list/map/set inside the \
                         compound — is not supported)",
                        t.render_name(&db.name_ctx())
                    )));
                };
                leaf_vts.extend(field_vts);
                slots.push(MakeSlot::Tuple {
                    shape,
                    rebuild: rebuild_fields,
                });
            }
            _ => match (
                crate::backend::wasm::lir::valtype_of(t),
                closure_boundary_byte(t),
            ) {
                (Some(vt), Some(byte)) => {
                    leaf_vts.push(vt);
                    slots.push(MakeSlot::Scalar(byte));
                }
                _ => {
                    // A HARD gate (`Err`) that already declines the compile — so tagging with the catalogued
                    // DeclineId is a pure id-tag (code None→CDZ0900 via `declined`, no gating change; contrast
                    // the closure NO_REPR soft-poisons, which are non-gating codeless and would false-gate).
                    return Err(Reject::declined(
                        crate::diag::DeclineId::WasmHeapReturnParamNoBoundaryRep,
                        format!(
                            "a parameterized heap-return export forwards scalar params and fixed-shape scalar \
                             tuple/record params only; parameter of type `{}` has no boundary representation",
                            t.render_name(&db.name_ctx())
                        ),
                    ));
                }
            },
        }
    }
    Ok(MakeParams { leaf_vts, slots })
}

/// The `make`-forwarded parameter plan for a heap-returning export's resource escape: the flattened CORE
/// leaf valtypes `make` takes (all params' leaves, left-to-right) plus a per-parameter [`MakeSlot`] (a
/// scalar leaf, or a fixed-shape tuple/record the leaves rebuild). A nullary export gives empty vecs. Any
/// MIX of scalar + compound params, and any number of compound params, composes — the leaf cursor threads
/// across slots at emit; the envelope mints one `tuple<…>` type per compound slot in param order.
struct MakeParams {
    /// The flattened core valtypes `make` receives (every param's leaves, in param order).
    leaf_vts: Vec<crate::backend::wasm::lir::ValType>,
    /// One entry per PARAMETER, in order: a scalar leaf (forwarded) or a compound (rebuilt from its leaves).
    slots: Vec<MakeSlot>,
}

/// One `make` parameter: a scalar leaf (its component boundary byte) or a fixed-shape tuple/record — its
/// (possibly NESTED) `TupleFieldShape` tree the envelope mints the `tuple<…>` type(s) from, and the
/// recursive per-field rebuild the core cell build uses. The count of flattened leaves a slot consumes is
/// 1 for a scalar, else the depth-first leaf count of the shape/rebuild.
enum MakeSlot {
    Scalar(u8),
    Tuple {
        shape: Vec<crate::backend::wasm::envelope::TupleFieldShape>,
        rebuild: Vec<crate::backend::wasm::serialize::FieldRebuild>,
    },
}

impl MakeParams {
    /// Whether ANY parameter is a compound — the resource shapes that don't yet emit the per-slot rebuild
    /// (sum/bytes) decline via [`Self::scalars_only`] when this is true.
    fn any_compound(&self) -> bool {
        self.slots
            .iter()
            .any(|s| matches!(s, MakeSlot::Tuple { .. }))
    }

    /// The (core leaf valtypes, inline scalar boundary bytes) for an ALL-SCALAR param set — declines if any
    /// parameter is a compound (the caller's emitter path doesn't yet emit the tuple-arg rebuild). Used by
    /// the sum/bytes escape emitters; the recursive-sum + flat emitters handle compounds via the slot model.
    fn scalars_only(self) -> Result<(Vec<crate::backend::wasm::lir::ValType>, Vec<u8>), Reject> {
        if self.any_compound() {
            return Err(Reject::unsupported(
                "a compound parameter on this heap-return shape is not supported (only a runtime \
                 collection / recursive-sum / BigInt / Rational / fixed-compound result supports a \
                 compound parameter)",
            ));
        }
        let bytes = self
            .slots
            .iter()
            .map(|s| match s {
                MakeSlot::Scalar(b) => *b,
                MakeSlot::Tuple { .. } => unreachable!("guarded by any_compound above"),
            })
            .collect();
        Ok((self.leaf_vts, bytes))
    }

    /// The envelope-side per-parameter boundary slots — a scalar byte, or a `tuple<…>` shape the envelope
    /// mints. Shared with the closure slot-model helpers (`mint_call_arg_tuple_types` /
    /// `make_functype_slots`).
    fn boundary_slots(&self) -> Vec<crate::backend::wasm::envelope::ArgSlot> {
        use crate::backend::wasm::envelope::ArgSlot;
        self.slots
            .iter()
            .map(|s| match s {
                MakeSlot::Scalar(b) => ArgSlot::Scalar(*b),
                MakeSlot::Tuple { shape, .. } => ArgSlot::Tuple(shape.clone()),
            })
            .collect()
    }

    /// Feed each runtime op a COMPOUND param's cell rebuild references (`arr-alloc`/`arr-set` + a box op
    /// per scalar leaf) into `out`, so the emitter imports them. A no-op when every param is a scalar.
    fn collect_rebuild_ops(&self, out: &mut impl FnMut(&'static str)) {
        let mut any = false;
        for s in &self.slots {
            if let MakeSlot::Tuple { rebuild, .. } = s {
                any = true;
                for f in rebuild {
                    f.collect_box_ops(out);
                }
            }
        }
        if any {
            out("arr-alloc");
            out("arr-set");
        }
    }

    /// The per-parameter cell rebuilds `make`'s core body threads (one per param; a scalar contributes an
    /// empty rebuild it forwards directly). Paired with [`Self::boundary_slots`] positionally.
    fn core_slots(&self) -> Vec<crate::backend::wasm::serialize::MakeCoreSlot> {
        use crate::backend::wasm::serialize::MakeCoreSlot;
        self.slots
            .iter()
            .map(|s| match s {
                MakeSlot::Scalar(_) => MakeCoreSlot::Scalar,
                MakeSlot::Tuple { rebuild, .. } => MakeCoreSlot::Tuple(rebuild.clone()),
            })
            .collect()
    }
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
        // The looped owned-heap-param drop epilogue (`select_body`) imports `drop` iff it ACTUALLY reclaims
        // a param — computed here (the def index gives `self_def` + params, which `collect_used_ops` lacks),
        // so the import matches the emit exactly (no over-declaration).
        let params = match layout.export_plan(def) {
            Some(e) => e.params.clone(),
            None => crate::layout::def_params(db, def),
        };
        if select::def_drops_owned_param(db, body, &params, Some(def)) {
            used.insert("drop");
        }
        // §5 self-loop-tail SUM-SPINE reclaim: a member tail-call carrying a self-consuming `Payload` arg
        // makes `emit_loop_iteration` add a per-iteration `dup` (retain the carried payload) + `drop` (free
        // the walked spine node). Import BOTH iff the reclaim fires — precise import/emit agreement.
        if select::def_sum_spine_reclaims(db, body, &params, Some(def)) {
            used.insert("dup");
            used.insert("drop");
        }
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
        // The byte-store used by the host `_mem` runtime-arg marshaling (`Lir::I32Store8`).
        assert_eq!(
            wasm_abi::op::I32_STORE8,
            opcode(I::I32Store8(wasm_encoder::MemArg {
                offset: 0,
                align: 0,
                memory_index: 0
            }))
        );
        // The memory LOADS the B3 reducer `apply(event: list<u8>)` wrapper uses to copy the incoming
        // event bytes out of linear memory into a heap `Bytes` (the `(ptr,len)` param read).
        assert_eq!(
            wasm_abi::op::I32_LOAD,
            opcode(I::I32Load(wasm_encoder::MemArg {
                offset: 0,
                align: 0,
                memory_index: 0
            }))
        );
        assert_eq!(
            wasm_abi::op::I32_LOAD8_U,
            opcode(I::I32Load8U(wasm_encoder::MemArg {
                offset: 0,
                align: 0,
                memory_index: 0
            }))
        );
        // The width-specific loads the general result-lift's scalar-leaf boxing uses (opcode consts checked
        // against the authoritative encoder, exactly like the loads above).
        let m = || wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        assert_eq!(wasm_abi::op::I64_LOAD, opcode(I::I64Load(m())));
        assert_eq!(wasm_abi::op::F32_LOAD, opcode(I::F32Load(m())));
        assert_eq!(wasm_abi::op::F64_LOAD, opcode(I::F64Load(m())));
        assert_eq!(wasm_abi::op::I32_LOAD8_S, opcode(I::I32Load8S(m())));
        assert_eq!(wasm_abi::op::I32_LOAD16_S, opcode(I::I32Load16S(m())));
        assert_eq!(wasm_abi::op::I32_LOAD16_U, opcode(I::I32Load16U(m())));
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
