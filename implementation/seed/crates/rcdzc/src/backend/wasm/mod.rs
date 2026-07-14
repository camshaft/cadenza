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

/// The kebab-case component EXTERN name for a source export identifier. A Cadenza identifier is broader
/// than a component-model extern name: it may contain uppercase letters (`fA`, `Foo`) or underscores
/// (`my_func`) — all valid source names — but the component model requires an export's extern name to be
/// kebab-case (`[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*`). Emitting a non-kebab name verbatim yields a component
/// that fails to validate ("export name `fA` is not a valid extern name") — an unloadable artifact. So a
/// non-kebab source name is NORMALIZED here at the component boundary. The rule (matches
/// `cadenza-syntax`'s `extern_name::kebab_extern_name`, which `cdz-run` uses to resolve a `--call` name):
///   * an UPPERCASE letter begins a word — insert a `-` before it (unless the output is empty / ends in
///     `-`), then lowercase it (`fA`→`f-a`, `myFunc`→`my-func`, `Foo`→`foo`);
///   * `_` becomes a `-` separator (`my_func`→`my-func`); runs of separators collapse; a trailing
///     separator is trimmed;
///   * a lowercase letter, a digit, or a `-` is kept — so an ALREADY-kebab name is the IDENTITY (every
///     corpus export is unchanged, byte-for-byte).
///
/// A source identifier always starts with a letter (a digit-led token is a numeric literal, rejected in
/// the reader), so the result always starts with a valid kebab word. Deterministic — the compiler and
/// the runner agree without threading a mapping across the boundary; a COLLISION (two source names → one
/// extern name) is rejected at export planning (`kebab_export_collision`) before emit.
pub(crate) fn kebab_extern_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '_' || c == '-' {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// If two DISTINCT source export names normalize to the SAME kebab extern name, return a reject naming
/// the collision (else `None`). Two exports that share a normalized extern name cannot both cross the
/// component boundary — the component would carry a duplicate export name (invalid) or silently drop one
/// — so the compiler declines rather than miscompile, exactly as the duplicate-export check does for
/// identical names. (Exports with the SAME source name are the duplicate-export case, caught earlier; a
/// name colliding with ITSELF under normalization is not a collision.) The FIRST such pair is reported.
fn kebab_export_collision(layout: &Layout) -> Option<Reject> {
    let mut seen: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for e in &layout.exports {
        let extern_name = kebab_extern_name(&e.name);
        if let Some(&prior) = seen.get(&extern_name)
            && prior != e.name
        {
            return Some(Reject::coded(
                crate::diag::Code::Malformed,
                format!(
                    "exports `{prior}` and `{}` both normalize to the component extern name `{extern_name}` \
                     — rename one so each export has a distinct kebab-case boundary name",
                    e.name
                ),
            ));
        }
        seen.insert(extern_name, &e.name);
    }
    None
}

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
fn crosses_as_resource_escape(ty: &crate::ty::Ty) -> bool {
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
            | Ty::Nominal { .. }
            | Ty::Qty { .. }
    )
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
    if let Some(reject) = kebab_export_collision(layout) {
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
    for &def in &layout.order {
        let body = def_body(db, def)?;
        if let Some((op, pos, ty)) = host::first_unrepresentable_host_op(db, body) {
            // "an argument" / "a result" — the article agrees with the position word.
            let article = if pos == "argument" { "an" } else { "a" };
            return Err(Reject::decline(format!(
                "the host operation `{op}` has {article} {pos} of type `{ty}`, which has no component \
                 boundary form this compiler emits yet (only scalar and unit results, and \
                 scalar/string/unit arguments, cross the host boundary; a string or compound result \
                 is a later increment)"
            )));
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
    if let [e] = &layout.exports[..]
        && e.params.is_empty()
        && crosses_as_resource_escape(&e.result)
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
        if matches!(&e.result, crate::ty::Ty::Nominal { .. })
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
            crate::ty::Ty::List(_) | crate::ty::Ty::Map(_, _) | crate::ty::Ty::Set(_)
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
        } else if let Some(tpl) = crate::lower::runtime_value_form_template(&result) {
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
    // The per-program HOST-import set (a `Core::HostCall`), first-encountered order. An unrepresentable
    // host-op boundary type was already declined honestly at the top of `emit` (the hoisted guard), so
    // every op reaching here has an emittable scalar/unit/string signature.
    let mut host_imports: Vec<host::HostImport> = Vec::new();
    for &def in &layout.order {
        let body = def_body(db, def)?;
        host::collect_host_imports(db, body, &mut host_imports);
    }
    // A program mixing a host effect AND the value-heap runtime composes BOTH import spaces
    // (`envelope::assemble_host_runtime`, wired in the host block below). One remaining combination still
    // declines: a host op with a STRING parameter (needs the shared-memory shape) composed with the runtime
    // — the memory + two-interface fusion is a further increment. A scalar/unit host op + runtime is emitted.
    if !host_imports.is_empty() && !imports.is_empty() && host::set_needs_memory(&host_imports) {
        return Err(Reject::decline(
            "a host op with a string parameter composed with the value-heap runtime is not yet emitted \
             (the shared-memory host shape and the runtime import compose in a later increment)",
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
    let mut boundary: Vec<BoundaryExport> = Vec::new();
    for e in &layout.exports {
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
        // A DIVERGING export — its body provably traps (`Core::Trap`: a bare `(trap …)`, a zero-arm match
        // on a `Never` scrutinee, or a call to such a function) — has a `Never` result type (a fresh var /
        // `Any`) with no boundary representation, but NO value ever crosses: the guest traps. Cross it as a
        // UNIT (no-result) export — the core function is already emitted 0-result (`select_function` maps a
        // diverging `ret` to `Ty::Unit`), and the host observes the trap. Checked BEFORE the escape/valtype
        // declines so a diverging `Any`/`Var` result is not misdiagnosed as an undetermined-type fault.
        if serialize::export_result_valtype(&e.result).is_err()
            && matches!(crate::lower::core_of(db, e.body), crate::core::Core::Trap)
        {
            boundary.push(BoundaryExport {
                name: e.name.clone(),
                params: {
                    let mut params = Vec::new();
                    for (_, ty) in &e.params {
                        let vt = serialize::export_result_valtype(ty)
                            .map_err(Reject::decline)?
                            .ok_or_else(|| {
                                Reject::decline(format!(
                                    "a diverging export's parameter `{}` has no boundary representation",
                                    ty.render_name()
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
            && serialize::export_result_valtype(&e.result).is_err()
        {
            // AMBIGUOUS TYPE first — a result whose payload/element type is an UNRESOLVED variable (a bare
            // `(None)` : `Option ?0`, an empty `(list)` : `List ?0`) has no defined serialization
            // REGARDLESS of export shape. A single NULLARY export with an unresolved payload reaches
            // here (the escape guard above tried and its value-form template returned `None`), so it must
            // NOT be diagnosed as an export-shape problem — the shape is fine; the TYPE is undetermined.
            // Report a type error naming the annotation fix (CDZ0203, the type-determination fault code),
            // NOT the parameterized/multi-export message. `e.params.is_empty()` distinguishes it from a
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
                // THE ARITY constraint — the real reason a compound/heap export declines here. Names it as
                // such (a compound crosses as the SOLE export), NOT "the type has no boundary
                // representation" (false — it crosses fine alone via the resource escape).
                "a heap value (a compound, string, or collection) crosses the host boundary only as the program's SINGLE export; this program has multiple exports (make it the only export, or return a scalar)"
            } else if !e.params.is_empty() {
                // A single PARAMETERIZED export — the resource-escape path covers only a NULLARY export,
                // so a heap return from a function that takes a parameter declines here.
                "a heap value escapes to the host as a resource only from a NULLARY export; this export takes a parameter (a parameterized heap return is not yet supported)"
            } else {
                // A single NULLARY export whose heap result reached here — the resource-escape path above
                // TRIED and its value-form template was `None`: the result has no runtime value form yet.
                // This is the RECURSIVE-sum / dynamic-shape / runtime-collection case (a self-referential
                // sum, or a runtime-built list/map/set built at runtime has an UNBOUNDED static shape, so
                // the `encode()` walker would need to LOOP to a runtime-determined depth — the analogue of
                // the runtime-`Bytes` looping walker, a later increment). The honest reason is the missing
                // walker; consuming such a value to a scalar already works, only rendering it as the
                // boundary result is deferred.
                "rendering this value as the host result needs a value-form walker that loops to a runtime-determined depth (a recursive-sum / runtime-collection result is not yet emitted); folding it to a scalar works"
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
        // A program that ALSO uses the value-heap runtime (a host op result fed into a runtime collection
        // op — `imports` non-empty) composes BOTH imported interfaces: `assemble_host_runtime` imports the
        // effect (as `"host"`) AND the runtime (as its versioned name, `"heap"`), aliases + lowers both op
        // sets, and instantiates the program with both bound. (A string-param host op + runtime declined
        // above; here the host set is scalar/unit, so no shared memory.) The core module already composed
        // both import spaces (`core_module_with_host` above).
        if !imports.is_empty() {
            let import_name = runtime_import_name();
            return Ok(envelope::assemble_host_runtime(
                &core,
                &boundary,
                &iface,
                &host_fns,
                &imports,
                &import_name,
            ));
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
    } else if matches!(result, crate::ty::Ty::Bytes | crate::ty::Ty::String) {
        // Mode S mirrors Mode E: a runtime String escapes through the SAME bytes walker (its String value
        // form), so route it here too — else its sidecar would decline while the embedded component emits.
        let form = if matches!(result, crate::ty::Ty::String) {
            crate::lower::runtime_string_form(db)
        } else {
            crate::lower::runtime_bytes_form(db)
        };
        let Some(form) = form else {
            return Err(Reject::decline(
                "a DWARF sidecar for this runtime-Bytes/String export is not yet supported (no value form)",
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
/// The decline for a closure `role` (`"argument"` / `"result"` / `"parameter"`) whose type `ty` cannot
/// cross the closure `call` host boundary. When `ty` is `Any` — the parameter was never constrained to
/// a concrete type — the "no scalar representation" phrasing is misleading (it reads as if a real type
/// is unsupported): the usual cause is a PARTIAL APPLICATION escaping as an export result (e.g. an
/// entrypoint returning `(f 1)` for a two-parameter `f`), whose remaining parameter has no solved type.
/// Say THAT. A concrete-but-unrepresentable type (a compound, a nested closure) keeps the precise
/// "no scalar host-boundary representation (only aliased widths cross yet)" message.
fn closure_boundary_reject(role: &str, ty: &crate::ty::Ty) -> Reject {
    if matches!(ty, crate::ty::Ty::Any) {
        Reject::decline(format!(
            "a closure crossing the host boundary has an unconstrained {role} type — the entrypoint's \
             result is a closure whose {role} type inference never fixed (a partial application like \
             `(f 1)` for a two-parameter `f`, or a closure with an unannotated parameter); a closure \
             crosses the boundary only with concrete, aliased-width scalar {role}s",
        ))
    } else {
        Reject::decline(format!(
            "a closure {role} of type {} has no scalar host-boundary representation (only aliased-width \
             scalars — every s8/u8/…/s64/u64 int, bool, f32/f64 — cross the closure `call` boundary)",
            ty.render_name()
        ))
    }
}

/// The COMPONENT-model boundary byte a closure arg/result/param crosses under — every ALIASED-WIDTH
/// SCALAR the ordinary export boundary supports (each of s8/u8/s16/u16/s32/u32/s64/u64, bool, f32/f64).
/// Wider than the runtime-op ABI table's `abi_val_type` (which models only u32/s64/bool/f64 for the
/// value-heap ops), because a closure's `call` boundary is a plain component functype — it needs only the
/// component primitive byte + the core valtype (from `valtype_of`), neither tied to the runtime-op set.
///
/// ⚠ Restricted to genuine SCALARS: `comp_valtype_of` ALSO returns a byte for a `Tuple` (the u32 HANDLE it
/// is threaded as BETWEEN in-program functions) and for a `Nominal`-over-compound, but those are opaque
/// runtime handles, NOT values the host can construct or read across the `call` boundary — a closure with a
/// COMPOUND arg/result must decline (that widening is a separate later increment). So this accepts only
/// Int/Bool/Float (peeling a nominal to its underlying scalar first); everything else is `None`.
fn closure_boundary_byte(ty: &crate::ty::Ty) -> Option<u8> {
    use crate::ty::Ty;
    match ty.strip_nominal() {
        Ty::Int(_) | Ty::Bool | Ty::Float(_) => crate::backend::wasm::lir::comp_valtype_of(ty),
        _ => None,
    }
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
    // Boundary bytes (component valtypes) for the `call` method's ARGS — always aliased scalar widths (a
    // compound closure arg is the host→guest DECODE direction, not yet supported).
    let arg_bytes: Vec<u8> = arg_tys
        .iter()
        .map(|t| closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("argument", t)))
        .collect::<Result<_, _>>()?;
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
        crate::lower::runtime_value_form_template(ret_ty.strip_nominal())
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
    let result_byte = if ret_is_bytes || ret_is_compound || ret_is_collection {
        0 // unused by the list-returning paths; `call` returns list<u8>, not a scalar byte
    } else {
        closure_boundary_byte(&ret_ty).ok_or_else(|| closure_boundary_reject("result", &ret_ty))?
    };
    // BRICK (d): a closure export whose build-time `make` code delegates a host effect (`host_imports`
    // non-empty, collected above) composes the host interface into the closure resource. This increment
    // handles a SCALAR closure result + a single scalar/unit host effect; other shapes decline cleanly.
    if !host_imports.is_empty() {
        if ret_is_bytes || ret_is_compound || ret_is_collection {
            return Err(Reject::decline(
                "a closure export that BOTH delegates a build-time host effect AND returns a \
                 byte-rope/compound/collection is not yet emitted (the host-composed closure core supports \
                 a scalar result this increment)",
            ));
        }
        let iface = host_imports[0].effect.clone();
        if host_imports.iter().any(|hi| hi.effect != iface) {
            return Err(Reject::decline(
                "a closure export delegating more than one host effect is not yet emitted (one interface \
                 per closure envelope)",
            ));
        }
        if host::set_needs_memory(&host_imports) {
            return Err(Reject::decline(
                "a closure export delegating a host op with a string parameter is not yet emitted (the \
                 shared-memory host shape and the closure resource envelope compose in a later increment)",
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
    // Core valtypes for the `call` method's args + result (used to build the core `call` signature +
    // the `call_indirect` lifted functype shape). A `Bytes` result is an i32 heap handle.
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = arg_tys
        .iter()
        .map(|t| valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype")))
        .collect::<Result<_, _>>()?;
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
            closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
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
    // A `Bytes`-result closure crosses `call` as `list<u8>` (through linear memory): the bytes-result core
    // serializer + the memory/realloc-lifting envelope. A scalar result takes the by-value path.
    if ret_is_bytes {
        let main_core = serialize::closure_bytes_resource_core_module(
            &funcs,
            &imports,
            export_abs,
            &arg_vts,
            &make_param_vts,
            lifted_type_idx,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes,
        ));
    }
    // A COMPOUND result crosses `call` as `list<u8>` carrying the value form — same `list<u8>` boundary as
    // the bytes path (so the SAME envelope), but the core walks the closure's returned handle to fill the
    // value-form template. The host decodes the bytes to `(: value T)`.
    if let Some(template) = &ret_template {
        let main_core = serialize::closure_value_resource_core_module(
            &funcs,
            &imports,
            export_abs,
            &arg_vts,
            &make_param_vts,
            lifted_type_idx,
            template,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes,
        ));
    }
    // A VARIABLE-LENGTH collection result → the value-encode core (dispatch → the collection handle, build
    // the descriptor Bytes, `value-encode(rep, desc)` → the value-form document, copy out). Same `list<u8>`
    // envelope as the bytes/compound paths; cdz-run try-decodes to `(: (list …) (List <e>))` etc.
    if let Some(descriptor) = &ret_descriptor {
        let main_core = serialize::closure_value_encode_resource_core_module(
            &funcs,
            &imports,
            export_abs,
            &arg_vts,
            &make_param_vts,
            lifted_type_idx,
            descriptor,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &make_param_bytes,
            &arg_bytes,
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
            closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
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
            comp_functype: host_op_comp_functype(hi),
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
    let arg_bytes: Vec<u8> = arg_tys
        .iter()
        .map(|t| closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("argument", t)))
        .collect::<Result<_, _>>()?;
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
        crate::lower::runtime_value_form_template(ret_ty.strip_nominal())
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
    let result_byte = if ret_is_bytes || ret_is_compound || ret_is_collection {
        0 // unused by the list-returning paths; `call` returns list<u8>
    } else {
        closure_boundary_byte(&ret_ty).ok_or_else(|| closure_boundary_reject("result", &ret_ty))?
    };
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = arg_tys
        .iter()
        .map(|t| valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype")))
        .collect::<Result<_, _>>()?;
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
                closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
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
    // A COMPOUND shared result → the N-makes-one-list-`call` VALUE-FORM core (walks each closure's returned
    // handle into the value-form template) + the SAME memory/realloc envelope as the bytes path. cdz-run
    // try-decodes the `list<u8>` result to the typed `(: value T)` form.
    if let Some(template) = &ret_template {
        let main_core = serialize::multi_closure_value_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &[],
            &arg_vts,
            lifted_type_idx,
            template,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_multi_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            &[],
        ));
    }
    // A VARIABLE-LENGTH collection shared result → the N-makes-one-list-`call` VALUE-ENCODE core (each `call`
    // dispatches, then value-encodes the returned collection handle) + the SAME memory/realloc envelope.
    if let Some(descriptor) = &ret_descriptor {
        let main_core = serialize::multi_closure_value_encode_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &[],
            &arg_vts,
            lifted_type_idx,
            descriptor,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_multi_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            &[],
        ));
    }
    // A byte-rope shared result → the N-makes-one-list-`call` bytes core + memory/realloc envelope. No plain
    // (non-closure) exports on the pure multi-export path.
    if ret_is_bytes {
        let main_core = serialize::multi_closure_bytes_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &[],
            &arg_vts,
            lifted_type_idx,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_multi_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            &[],
        ));
    }
    let main_core = serialize::multi_closure_resource_core_module(
        &funcs,
        &imports,
        &ser_makes,
        &[], // no plain (non-closure) exports on the pure multi-export path
        &arg_vts,
        ret_vt,
        lifted_type_idx,
        &layout,
    )
    .map_err(Reject::decline)?;
    Ok(envelope::assemble_multi_closure_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_makes,
        &arg_bytes,
        result_byte,
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
    let arg_bytes: Vec<u8> = arg_tys
        .iter()
        .map(|t| closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("argument", t)))
        .collect::<Result<_, _>>()?;
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
        crate::lower::runtime_value_form_template(ret_ty.strip_nominal())
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
    let result_byte = if ret_is_bytes || ret_is_compound || ret_is_collection {
        0 // unused by the list-returning paths; `call` returns list<u8>
    } else {
        closure_boundary_byte(&ret_ty).ok_or_else(|| closure_boundary_reject("result", &ret_ty))?
    };
    let arg_vts: Vec<crate::backend::wasm::lir::ValType> = arg_tys
        .iter()
        .map(|t| valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype")))
        .collect::<Result<_, _>>()?;
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
                closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
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
                closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
            })
            .collect::<Result<_, _>>()?;
        let result_byte = closure_boundary_byte(&e.result).ok_or_else(|| {
            Reject::decline(format!(
                "a plain export `{}` returning {} has no scalar host-boundary representation — a \
                 compound result alongside a closure export is a later widening",
                e.name,
                e.result.render_name()
            ))
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
    // A COMPOUND shared closure result → the VALUE-FORM mixed core (N makes + shared list-`call` walking each
    // closure's returned handle into the value-form template + the plain exports as top-level funcs), same
    // `list<u8>` envelope as the bytes path. cdz-run try-decodes the result to the typed `(: value T)` form.
    if let Some(template) = &ret_template {
        let main_core = serialize::multi_closure_value_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &ser_plain,
            &arg_vts,
            lifted_type_idx,
            template,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_multi_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            &abi_plain,
        ));
    }
    // A VARIABLE-LENGTH collection shared closure result → the mixed VALUE-ENCODE core (N makes + shared
    // value-encode `call` + the plain exports as top-level funcs), same `list<u8>` envelope.
    if let Some(descriptor) = &ret_descriptor {
        let main_core = serialize::multi_closure_value_encode_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &ser_plain,
            &arg_vts,
            lifted_type_idx,
            descriptor,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_multi_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            &abi_plain,
        ));
    }
    // A byte-rope shared closure result → the mixed BYTES envelope (N makes + shared list-`call` + the plain
    // exports as top-level funcs). A scalar result takes the by-value mixed envelope.
    if ret_is_bytes {
        let main_core = serialize::multi_closure_bytes_resource_core_module(
            &funcs,
            &imports,
            &ser_makes,
            &ser_plain,
            &arg_vts,
            lifted_type_idx,
            &layout,
        )
        .map_err(Reject::decline)?;
        return Ok(envelope::assemble_multi_closure_bytes_resource(
            &main_core,
            &dtor_core,
            &imports,
            &import_name,
            &abi_makes,
            &arg_bytes,
            &abi_plain,
        ));
    }
    let main_core = serialize::multi_closure_resource_core_module(
        &funcs,
        &imports,
        &ser_makes,
        &ser_plain,
        &arg_vts,
        ret_vt,
        lifted_type_idx,
        &layout,
    )
    .map_err(Reject::decline)?;
    Ok(envelope::assemble_mixed_closure_resource(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_makes,
        &arg_bytes,
        result_byte,
        &abi_plain,
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
        let arg_bytes: Vec<u8> = arg_tys
            .iter()
            .map(|t| closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("argument", t)))
            .collect::<Result<_, _>>()?;
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
            crate::lower::runtime_value_form_template(ret_ty.strip_nominal())
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
        let result_byte = if ret_is_bytes || ret_template.is_some() || ret_descriptor.is_some() {
            0
        } else {
            closure_boundary_byte(&ret_ty)
                .ok_or_else(|| closure_boundary_reject("result", &ret_ty))?
        };
        let arg_vts: Vec<ValType> = arg_tys
            .iter()
            .map(|t| {
                valtype_of(t).ok_or_else(|| Reject::decline("closure arg has no machine valtype"))
            })
            .collect::<Result<_, _>>()?;
        let ret_vt = valtype_of(&ret_ty)
            .ok_or_else(|| Reject::decline("closure result has no machine valtype"))?;
        ginfos.push(GroupInfo {
            arg_vts,
            ret_vt,
            arg_bytes,
            result_byte,
            ret_is_bytes,
            ret_template,
            ret_descriptor,
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
                closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
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
                closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
            })
            .collect::<Result<_, _>>()?;
        let result_byte = closure_boundary_byte(&e.result).ok_or_else(|| {
            Reject::decline(format!(
                "a plain export `{}` returning {} has no scalar host-boundary representation — a \
                 compound result alongside a closure export is a later widening",
                e.name,
                e.result.render_name()
            ))
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
        lifted_shapes.iter().position(|(ps, rv)| {
            ps.as_slice() == ginfo.arg_vts.as_slice() && *rv == Some(ginfo.ret_vt)
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
        ser_groups.push(serialize::SigGroup {
            makes: ser_makes,
            arg_vts: ginfos[gi].arg_vts.clone(),
            ret_vt: ginfos[gi].ret_vt,
            lifted_slot: slot,
            ret_is_bytes: ginfos[gi].ret_is_bytes,
            ret_template: ginfos[gi].ret_template.clone(),
            ret_descriptor: ginfos[gi].ret_descriptor.clone(),
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
    let main_core = serialize::distinct_sig_resource_core_module(
        &funcs,
        &imports,
        &ser_groups,
        &ser_plain,
        &layout,
    )
    .map_err(Reject::decline)?;
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_distinct_sig_resource_mixed(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &abi_groups,
        &abi_plain,
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
        return Err(Reject::decline(format!(
            "the export `{}` both RECEIVES a closure (a parameter) and RETURNS one (its result) — a \
             closure transformer. That is not yet supported: the host would pass a closure in and get one \
             out of the same call, which needs the closure to cross as `own<t>` in both directions of one \
             boundary function (DESIGN-closure-host-resource-rcdzc.md, closure transformers)",
            t.name
        )));
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
            return Err(Reject::decline(
                "a round-trip program mixing closures of DIFFERENT signatures is not yet supported \
                 (one resource type per signature is a later slice)",
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
            return Err(Reject::decline(
                "a round-trip consumer's closure parameter has a different signature than the produced \
                 closure (mixed signatures are a later slice)",
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
                t.render_name()
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
            ret_ty.render_name()
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
                        t.render_name()
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
                        t.render_name()
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
            crate::lower::runtime_value_form_template(c.result.strip_nominal())
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
                        c.result.render_name()
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
                closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
            })
            .collect::<Result<_, _>>()?;
        let result_byte = closure_boundary_byte(&e.result).ok_or_else(|| {
            Reject::decline(format!(
                "a plain export `{}` returning {} has no scalar host-boundary representation — a \
                 compound result alongside a round-trip closure is a later widening",
                e.name,
                e.result.render_name()
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
        return Err(Reject::decline(format!(
            "the export `{}` both receives and returns a closure (a closure transformer) — not yet \
             supported (DESIGN-closure-host-resource-rcdzc.md, closure transformers)",
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
                return Err(Reject::decline(
                    "a distinct-signature round-trip consumer with more than one closure parameter is \
                     not yet supported",
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
            valtype_of(&dom).ok_or_else(|| closure_boundary_reject("argument", &dom))?;
            cur = *rng;
        }
        valtype_of(&cur).ok_or_else(|| closure_boundary_reject("result", &cur))?;
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
                    closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
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
                    closure_boundary_byte(t).ok_or_else(|| closure_boundary_reject("parameter", t))
                })
                .collect::<Result<_, _>>()?;
            let result_byte = closure_boundary_byte(&e.result).ok_or_else(|| {
                Reject::decline(format!(
                    "a plain export `{}` returning {} has no scalar host-boundary representation — a \
                     compound result alongside a round-trip closure is a later widening",
                    e.name,
                    e.result.render_name()
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
                        .ok_or_else(|| closure_boundary_reject("parameter", t))?;
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
                crate::lower::runtime_value_form_template(e.result.strip_nominal())
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
                    .ok_or_else(|| closure_boundary_reject("result", &e.result))?
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

    // VM-1/VM-3: a Bytes result crosses as a resource carrying make + encode + `len : borrow<t> -> u32`
    // (= `bytes-len(rep)`) + `is-empty : borrow<t> -> bool` (= `bytes-len == 0`) + `to-bytes : borrow<t>
    // -> list<u8>` (the RAW payload). The core emits `t-len`/`t-is-empty`/`t-to-bytes` (`bytes-len`/
    // `bytes-get` already imported for the encode walker), and the envelope lifts the three extra methods.
    let core_methods = [
        serialize::CoreMethod::Len,
        serialize::CoreMethod::IsEmpty,
        serialize::CoreMethod::ToBytes,
    ];
    let mut main_core = serialize::runtime_resource_core_module_form_ex(
        &funcs,
        &imports,
        export_abs,
        serialize::EscapeForm::RuntimeBytes(form),
        &core_methods,
    )
    .map_err(Reject::decline)?;
    // DEBUG: same as the flat/sum resource paths — the user bodies lead the escape core's code section,
    // so the `name` + `.debug_*` sections attribute correctly; the synthesized bytes walker has no
    // `src_body` and gets no row.
    append_debug_sections(db, layout, &funcs, &imports, spans, &mut main_core);
    let dtor_core = serialize::resource_dtor_module_with_drop();
    let import_name = runtime_import_name();
    Ok(envelope::assemble_runtime_resource_with_scalar_methods(
        &main_core,
        &dtor_core,
        &imports,
        &import_name,
        &[
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
        ],
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
///
//= spec/contracts/reproducible-derivation.md#derivation-is-a-function-of-source-and-toolchain
//# The identity of the value-heap runtime a program is emitted against MUST be the content address of that runtime component, so that "which runtime" is a hash rather than a version label and a program's observable behavior — which depends on the runtime's construction, storage, and reclamation of values — is pinned to exact bytes (component-abi.md §The Value-Heap Runtime).
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
