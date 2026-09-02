//! The Rust-source backend — a STRUCTURED backend that emits ordinary Rust source.
//!
//! Where the wasm backend linearizes the core into a flat instruction stream, this backend consumes
//! the typed structured core DIRECTLY and prints it as Rust — the core's `if`/`match`/`let`/`call`
//! become Rust's own (`backends-and-targets.md` §A Backend Linearizes The Core Only If Its Target Is
//! Linear: "a backend whose target has structured control flow consumes the typed structured core
//! directly … and never constructs the flat rung"). It reads the same columns every backend does
//! (`core_of`, `type_of`, the target-neutral [`Layout`]) — the concrete proof the pipeline above the
//! seam is target-neutral, not wasm-shaped.
//!
//! It emits a self-contained Rust module: one `pub fn` per export, named verbatim, with native scalar
//! parameter and result types. The point of the target is drop-in integration — a Cadenza-authored
//! module compiles to a `.rs` file that links into an existing Rust codebase as ordinary source, with
//! no component boundary, no runtime import, and no FFI.
//!
//! Value strategy (`backends-and-targets.md` §A Compound Value's Representation Is The Backend's
//! Choice): this backend uses the target's NATIVE aggregates rather than the shared value-heap runtime
//! (the "rust-ergonomic" strategy) — so a Cadenza integer is a Rust integer and no `cdz-runtime` is
//! linked. The scalar slice built here reaches only the scalar value language (integers, Bool, Unit);
//! a compound value or any construct the front already declined is DECLINED here too, attributed to
//! this target (`§A Backend Inherits The Front's Decline Boundaries`).
//!
//! CORRECTNESS: a Cadenza integer TRAPS on overflow, so a checked `+`/`-`/`*` emits `checked_*(…)`
//! with a trap on `None` — Rust's native `iN`/`uN` are exactly Cadenza's aliased widths with the same
//! wrapping-vs-checked distinction, so the numeric model maps across without a scratch-local guard
//! recipe (that recipe was a way to express checked arithmetic in the flat wasm rung; Rust expresses
//! it directly). The one executable semantics is the oracle either way (`§The meaning against which
//! every backend's output is judged MUST be the one executable semantics`).
//!
//! DEP-FREE, LIKE THE BYTE PATH: the Rust source is emitted as plain text (a `String`), exactly as the
//! wasm backend hand-emits bytes — no `syn`/`quote`/`prettyplease`. So this backend carries no new
//! dependency and ports to the Cadenza self-host on the same footing as the byte path (a source
//! string is as portable as a byte vector); `Target::Rust` is always available, not feature-gated.

mod enums;
mod expr;
mod types;
// The Ty-guided VALUE-DOC emit (operator seq-210 parser-elimination) — WIP, not yet wired into `emit`
// (the flag-gated driver call comes in a later increment), so `#[allow(dead_code)]` until then.
#[allow(dead_code)]
mod value_doc;

use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// If `ty` is an integer type whose FIXED width is ILL-FORMED (outside the admitted `1..=64`), return the
/// CDZ0302 reject that names the fault — else `None`. An out-of-range width (the sentinel `Int0` a
/// `reduce_ctor` clamp leaves after a malformed/negative/over-ceiling width like `(Int -8)` / `(UInt 65)`)
/// is an ILL-FORMED TYPE: no value of it can exist, so a boundary of that type is a REJECT, not a target
/// limitation. This mirrors the wasm backend, which reaches CDZ0302 by fit-checking the ground literal
/// against the empty width-0 range (`select.rs`); the Rust backend's type-mapping decline would otherwise
/// fire FIRST and mask the diagnostic (a codeless "no native Rust representation" → the gate reads it as an
/// unimplemented-construct `todo` instead of the typed rejection `pass` the wasm target gives). MUST be
/// distinguished from a VALID-but-non-aliased width (`UInt7`, `UInt24` — in `1..=64`): that is a genuine
/// backend limitation with no native Rust primitive, which stays a codeless decline (todo, correct).
/// Whether `ty` is a function type (a closure value) — after stripping a nominal newtype wrapper. Used to
/// decline a closure crossing the EXPORT boundary (no closure-handle ABI on the Rust target).
fn is_fn_ty(ty: &crate::ty::Ty) -> bool {
    matches!(ty.strip_nominal(), crate::ty::Ty::Fn(_, _))
}

/// Whether a factory CAPTURE (a `make` parameter the host supplies over the boundary) is a scalar the gate
/// harness passes as a literal at the make-split: Int/Bool OR Float. An aliased-width scalar crosses the
/// host→guest boundary directly (`make(1.5, 7)` — the harness renders each capture arg via `rust_call_arg`,
/// which spells a Float literal too), so a MIXED scalar capture environment (Float64 + Int64) is admitted.
/// A COMPOUND capture (a host-supplied Tuple/List/record/sum) still declines — it needs a host→guest decode
/// that does not exist (the "producer capturing a host-supplied COMPOUND parameter is declined" case).
fn is_capture_scalar(t: &crate::ty::Ty) -> bool {
    matches!(
        t.strip_nominal(),
        crate::ty::Ty::Int(_) | crate::ty::Ty::Bool | crate::ty::Ty::Float(_)
    )
}

/// A closure ARG type admitted by the S4-HIGHER-ORDER slice: an `s2_arg_ok` scalar/compound, OR a
/// closure (`Ty::Fn`) whose own arg/result spine is itself `arg_ok_or_fn` (recursively). A closure whose
/// arg is itself a closure (`(-> (-> Int64 Int64) Int64)`) is the higher-order round-trip shape — the
/// consumer applies it to an IN-GUEST-built inner closure (`(g (fn (y) …))`, which the emitter already
/// lowers), and the higher-order producer (`mk`) is passed by the harness as `Rc::new(mk)`. Distinct from
/// `s2_arg_ok` (which rejects every `Ty::Fn`) so the base non-higher-order gates keep their exact behavior.
fn arg_ok_or_fn(ncx: &crate::ty::NameCtx, t: &crate::ty::Ty) -> bool {
    if let crate::ty::Ty::Fn(_, _) = t.strip_nominal() {
        let mut cur = t.strip_nominal();
        while let crate::ty::Ty::Fn(p, r) = cur {
            if !arg_ok_or_fn(ncx, p) {
                return false;
            }
            cur = r.strip_nominal();
        }
        return arg_ok_or_fn(ncx, cur);
    }
    s2_arg_ok(ncx, t)
}

/// Whether a closure ARG type is OK for the host-closure FACTORY slice: a scalar (Int/Bool/Float — the
/// harness passes each as a literal), a String/Bytes applied IN-GUEST (a literal the emitter lowers — see
/// the arm's own comment for the in-guest-vs-host-boundary distinction), or a COMPOUND the harness's
/// `rust_call_arg` rebuilds structurally over OK elements: a TUPLE (`both(caps)((3, 4))` — the closure reads
/// the native `(i64, i64)`), a LIST (`…(vec![…])`), or Option/Result (the well-known 2-variant sums, `(Some
/// v)`→`Some(v)` etc.). A String/Bytes arg PASSED FROM THE HOST at the boundary (a different ABI) or a USER
/// sum arg stays DEFERRED — the factory still emits a valid `Rc<dyn Fn>`, so those cases stay a clean `todo`.
fn s2_arg_ok(ncx: &crate::ty::NameCtx, t: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    match t.strip_nominal() {
        Ty::Int(_) | Ty::Bool | Ty::Float(_) => true,
        // A String/Bytes closure ARG is fine when the closure is APPLIED IN-GUEST with a literal/constructed
        // value the emitter already lowers (`(g "hello")` — the consumer builds the `String` in its own body,
        // no host-supplied String crosses the boundary). `rust_call_arg` renders a String literal natively,
        // and the closure param type maps to `Rc<dyn Fn(String) -> …>`. (A String arg PASSED FROM THE HOST at
        // the boundary is a different, still-deferred ABI — that shape has no producing-sibling-driven synth.)
        Ty::String | Ty::Bytes => true,
        Ty::Tuple(elems) => elems.iter().all(|e| s2_arg_ok(ncx, e)),
        // A RECORD closure ARG — the harness's `rust_call_arg` rebuilds a `(record …)` literal into the
        // native positional Rust tuple in SORTED-key order (matching the emit's field layout). Admitted iff
        // every field type is. NOTE: a Tuple-arg and a same-field-type Record-arg closure erase to the
        // IDENTICAL `Rc<dyn Fn((i64,i64))>`, so the driver disambiguates producer↔consumer pairing via the
        // `// cdz-param-shapes` note (the pre-erasure arrow type) — without that, a distinct-sig two-closure
        // consumer could mispair (breaker; the reason this arm waited for the param-shapes note).
        Ty::Record(fields) => fields.values().all(|t| s2_arg_ok(ncx, t)),
        Ty::List(elem) => s2_arg_ok(ncx, elem),
        // Option/Result (the WELL-KNOWN 2-variant sums the harness rebuilds — `(Some v)`→`Some(v)`,
        // `(Ok v)`/`(Err e)`→`Ok(v)`/`Err(e)`, `(None …)`→`None`) over S2-OK payloads. Identified by the
        // sum's NAME (its type args ARE the payloads: `Option Int64`→`[Int64]`, `Result a b`→`[a, b]`); a
        // USER sum stays deferred (the harness has no constructor rebuild for it). Recurse over the args so
        // a nested `Option (Tuple …)` / `Result (List …) …` is admitted iff its payloads are.
        Ty::Sum { decl, args } if matches!(ncx.name_of(*decl), Some("Option") | Some("Result")) => {
            args.iter().all(|a| s2_arg_ok(ncx, a))
        }
        _ => false,
    }
}

/// Walk a result type collecting every NON-scale-1 QUANTITY leaf's PATH + its scale, for the per-element
/// display-scale notes (`// cdz-qty-at[ident]: <path> <num>/<den>`). The `path` is the render's descent
/// route into the Rust value — the SAME positional `.N` field indices `cdz_render_at` uses (a tuple field
/// `i` is `(<path>).i`, a record field `i` in sorted order is `.i`), joined by `.` (e.g. `0`, `1`, `0.1`
/// for a nested tuple's second field). COVERS: TUPLE + RECORD holes (positional `.i`), OPTION/RESULT
/// payloads (`?N`, N = type-arg index), and USER-DEFINED sum variant payloads (keyed by the LOCAL
/// `<variant>?<idx>` — the render's per-sum helper is reused across call-sites so it can't see an outer path
/// prefix; a monomorphic user sum only, a generic one's payload carries unsubstituted type params). A
/// RECURSIVE sum is cycle-guarded via `visited` (its own decl on the descent path → skip, else infinite
/// recursion → stack overflow). STILL a follow-up: a Qty inside a LIST element (the render's list binder is
/// per-iteration). The empty path (`""`) — a TOP-LEVEL bare Qty — is NOT collected: it carries its scale via
/// the single `// cdz-scale` note. Only a `qty_scale_supported` inner (Int/Float/Rational) at a non-1 scale
/// gets an entry (scale-1 renders as stored). Mirrors wasm `const_value_ast_scaled`'s per-element scale-fold.
fn collect_qty_scale_paths(
    db: &mut Db,
    t: &crate::ty::Ty,
    path: &str,
    out: &mut Vec<(String, i128, i128)>,
    visited: &mut Vec<crate::ast::StructId>,
) {
    use crate::ty::Ty;
    // `strip_nominal` borrows `t`; clone the stripped type so `db` can be reborrowed mutably in the sum arm.
    let t = t.strip_nominal().clone();
    match &t {
        // Skip the top-level bare Qty (empty path) — the existing single `// cdz-scale` note covers it; only
        // a NESTED Qty (non-empty path) at a non-1 scale over a supported inner needs a per-element note.
        Ty::Qty { inner, unit } if !path.is_empty() => {
            let (num, den) = unit.scale();
            if (num, den) != (1, 1) && types::qty_scale_supported(inner, (num, den)) {
                out.push((path.to_string(), num, den));
            }
        }
        Ty::Tuple(elems) => {
            for (i, e) in elems.iter().enumerate() {
                let child = if path.is_empty() {
                    i.to_string()
                } else {
                    format!("{path}.{i}")
                };
                collect_qty_scale_paths(db, e, &child, out, visited);
            }
        }
        Ty::Record(fields) => {
            // Fields are baked in sorted (BTreeMap) order — the SAME `.i` index the render reads.
            for (i, (_, ft)) in fields.iter().enumerate() {
                let child = if path.is_empty() {
                    i.to_string()
                } else {
                    format!("{path}.{i}")
                };
                collect_qty_scale_paths(db, ft, &child, out, visited);
            }
        }
        // An OPTION/RESULT payload (the well-known 2-variant sums the render descends into with a fresh
        // binder). Use a `?N` path segment (N = the type-arg index: Option's payload is `?0`, Result's Ok
        // is `?0` / Err is `?1`) — a segment the render's Option/Result arms mirror when they extend the
        // logical path. So a `(Option (Qty km))` result scales its payload; a nested
        // `(Tuple (Option (Qty km)) …)` composes (`0?0`).
        Ty::Sum { decl, args }
            if matches!(
                db.type_decl_by_occ(*decl).map(|t| t.name.as_str()),
                Some("Option") | Some("Result")
            ) =>
        {
            for (i, a) in args.iter().enumerate() {
                let child = if path.is_empty() {
                    format!("?{i}")
                } else {
                    format!("{path}?{i}")
                };
                collect_qty_scale_paths(db, a, &child, out, visited);
            }
        }
        // A USER-DEFINED sum — resolve each variant's payload types via the type decl and recurse, keying by
        // the LOCAL `<variant-name>?<payload-idx>` (NO outer path prefix). WHY local: the render routes a user
        // sum through a helper `fn __render_<Sum>` generated ONCE per sum type + REUSED across call-sites
        // (keyed by the sum name, not the descent path), so the helper cannot know an outer path prefix — it
        // keys its Qty payloads by the local `<variant>?<idx>`. To stay consistent, emit the SAME local key.
        // SCOPE: correct for a user sum at the TOP-LEVEL result (v-quantity's `Circle(3km)`) or wherever the
        // `<variant>?<idx>` is unambiguous; a user sum NESTED inside another compound is imperfect (the helper
        // loses the prefix) — a documented follow-up. MONOMORPHIC user sums only (a generic sum's payload
        // carries unsubstituted `T{k}` type params here). A `Circle(Qty km)` payload scales to reference.
        Ty::Sum { decl, .. } => {
            // GUARD a RECURSIVE sum (`IntList = Cons(Tuple Int64 IntList) | Nil`): its payload references the
            // sum itself, so descending unguarded recurses forever → stack overflow. Skip a decl already on
            // the descent path (its Qty leaves, if any, are collected at the first visit). `visited` tracks
            // sum decls currently being walked.
            if visited.contains(decl) {
                return;
            }
            if let Some(t) = db.type_decl_by_occ(*decl) {
                // Clone the (name, payload-occs) pairs first — `typeval_of` takes `&mut db`.
                let variants: Vec<(String, Vec<crate::ast::StructId>)> = t
                    .variants
                    .iter()
                    .map(|v| (v.name.clone(), v.payloads.clone()))
                    .collect();
                visited.push(*decl);
                for (vname, payload_occs) in variants {
                    for (i, occ) in payload_occs.iter().enumerate() {
                        if let Some(pty) = crate::eval::typeval_of(db, *occ) {
                            // LOCAL key (no `path` prefix) — matches the reused helper's keying.
                            let child = format!("{vname}?{i}");
                            collect_qty_scale_paths(db, &pty, &child, out, visited);
                        }
                    }
                }
                visited.pop();
            }
        }
        // A LIST / SET element, or a MAP KEY/VALUE: the scale is UNIFORM across every element/entry (all
        // share the collection's element type + unit), so a SINGLE note per element-position suffices — the
        // render applies it to each per-iteration binder. Path segments: a list/set element extends with
        // `.*`, a map value with `!v`, a map key with `!k` (segments the render's collection arms mirror).
        // A Qty in a Map VALUE (`(Map Int64 (Qty Float64 km))`) or a List element (`(List (Qty Float64 km))`)
        // thus display-scales — v-quantity's whole-Map-value / List-element gap. (Distinct from the KEY-side
        // float wrapper: this is the display-render scale-fold, a Map/List VALUE position.)
        Ty::List(e) | Ty::Set(e) => {
            let child = if path.is_empty() {
                "*".to_string()
            } else {
                format!("{path}.*")
            };
            collect_qty_scale_paths(db, e, &child, out, visited);
        }
        Ty::Map(k, v) => {
            let (kp, vp) = if path.is_empty() {
                ("!k".to_string(), "!v".to_string())
            } else {
                (format!("{path}!k"), format!("{path}!v"))
            };
            collect_qty_scale_paths(db, k, &kp, out, visited);
            collect_qty_scale_paths(db, v, &vp, out, visited);
        }
        _ => {}
    }
}

/// The type-name to put in the `// cdz-return[<ident>]` note — normally `result.render_name(&db.name_ctx())`, but for a
/// GENERIC nominal returned WHOLE it is the ERASED INNER's render_name instead. WHY: a monomorphic nominal
/// newtype gets a `// cdz-newtype[<Ident>]` descriptor the render uses to resolve `<Ident>` → its structural
/// inner; but a GENERIC nominal (`(type V3q (V3 a a a))`) is SKIPPED by `emit_newtype_descriptors`
/// (`!decl.params.is_empty()`), so at an instantiated whole-return the render would get the bare nominal name
/// `V3q`, find no descriptor, and fall to a scalar `Display` of the erased Rust tuple `(f64, f64, f64)` →
/// rustc E0277 (v-quantity/corpus-bugfix: "a record of quantities RETURNED as a value"). Noting the inner's
/// render_name (`(Tuple (Qty …) (Qty …) …)`) instead lets the render's structural Tuple/Record arm handle it
/// — and the per-element `// cdz-qty-at` notes already key by the same positional descent, so a nested Qty
/// field still display-scales. Only for a GENERIC nominal whose erased inner is a STRUCTURAL type the render
/// walks (Tuple/Record); a monomorphic nominal keeps its name (its newtype descriptor resolves it), and a
/// non-nominal result is unchanged.
fn boundary_return_render_name(db: &Db, result: &crate::ty::Ty) -> String {
    use crate::ty::Ty;
    if let Ty::Nominal { decl, inner, .. } = result {
        let is_generic = db
            .type_decl_by_occ(*decl)
            .map(|t| !t.params.is_empty())
            .unwrap_or(false);
        if is_generic && matches!(inner.as_ref(), Ty::Tuple(_) | Ty::Record(_)) {
            return inner.render_name(&db.name_ctx());
        }
    }
    result.render_name(&db.name_ctx())
}

/// Whether a closure RESULT type is renderable by the gate harness (S1 scalar OR S3 Tuple/List/Option/
/// Result). The factory result is rendered by `cdz_render_expr`, which walks the value's TYPE and emits the
/// corpus s-expr form — including the Option/Result arms (`(Some <p>)`/`(None unit)`/`(Ok <p>)`/`(Err <e>)`,
/// the SAME render a plain sum export uses). So an Option/Result RESULT over renderable payloads is admitted
/// (S4a), matching `s2_arg_ok`'s Option/Result arm on the arg side.
fn s3_result_ok(t: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    match t.strip_nominal() {
        Ty::Int(_) | Ty::Bool | Ty::Float(_) => true,
        // A String/Bytes RESULT crosses the host boundary AS `list<u8>` — the gate harness renders a factory
        // String/Bytes result as the byte-int list `(104 105)` (`cdz_render_bytes_list`), the observable form
        // the wasm `call` method produces (it copies the handle into linear memory + returns list<u8>). NOT
        // the quoted `"hi"`/`b"…"` a plain export uses — the factory-result render path special-cases these.
        Ty::String | Ty::Bytes => true,
        Ty::Tuple(elems) => elems.iter().all(s3_result_ok),
        // A RECORD RESULT renders like a Tuple — the factory-result render walks its SORTED-key fields
        // positionally (`cdz_render_expr`'s Record arm), the same native tuple the emit produces. Renderable
        // iff every field type is (recurse over the sorted values). (The record ARG side — `s2_arg_ok` — is
        // NOT yet widened for Record: `rust_call_arg` ALREADY rebuilds a record literal into the sorted-field
        // positional tuple, so it is not a rebuild gap; the blocker is a PRODUCER-PAIRING ambiguity — a
        // `(Tuple a b)`-arg and a `(Record (a)(b))`-arg closure erase to the IDENTICAL `Rc<dyn Fn((i64,i64))>`,
        // so the gate driver's `ty_matches` can mispair them in a distinct-sig two-closure case. Widening
        // `s2_arg_ok` needs a closure-arg-shape hint to disambiguate first — a separate follow-up slice.)
        Ty::Record(fields) => fields.values().all(s3_result_ok),
        Ty::List(elem) => s3_result_ok(elem),
        // A SET/MAP RESULT renders via `cdz_render_expr`'s Set/Map arm into the canonical `(set …)`/`(map (k
        // v) …)` form (members/entries in the runtime's canonical order — the SAME text the wasm `call`
        // value-encode produces). Renderable iff the element (Set) / key+value (Map) types are. A Set/Map-
        // returning closure passes on SYNC already (a nullary one eta-peels to a plain `BTreeSet`/`BTreeMap`-
        // returning fn the render handles); this arm lifts the async FACTORY form (which stays a factory,
        // not peeled) — the render side was never the gap, only this `s3_result_ok` gate.
        Ty::Set(e) => s3_result_ok(e),
        Ty::Map(k, v) => s3_result_ok(k) && s3_result_ok(v),
        // A SUM RESULT (S4a + user-sum extension) — Option/Result (the well-known 2-variant sums) OR a USER
        // sum (`(type Dir (N) (S))`). The harness renders all of them via `cdz_render_at` into the corpus
        // value form: `(Some <p>)`/`(None unit)`/`(Ok <p>)`/`(Err <e>)` for the built-ins, `(<Variant> <p>)`/
        // `(<Variant> unit)` for a user sum (from the emitted `// cdz-sum[…]` descriptor + generated helper).
        // The factory-result render wraps it in the type-annotated `(: <value> <type>)` value form (the shape
        // the wasm `call` value-encode produces). Recurse over the type ARGS (a generic sum's instantiation —
        // `Option Int64`, `Box (Tuple …)`); a monomorphic user sum has no args (trivially OK), its variant
        // payloads rendered by the descriptor. So a sum result is renderable iff its type-args are.
        Ty::Sum { args, .. } => args.iter().all(s3_result_ok),
        _ => false,
    }
}

/// Whether a closure-RESULT `Ty::Fn` is safe for host-closure FACTORY export. Each PARAMETER (the closure's
/// args) may be an S1 scalar OR an S2 compound (`s2_arg_ok` — Tuple/List/Option/Result the harness rebuilds),
/// and the final RESULT (S3) may be a scalar/Float, String/Bytes, Tuple/List, or a SUM (Option/Result/user)
/// the harness renders as the value form (`s3_result_ok`). The gate peels the factory's arrow to the final
/// result type and renders it via `cdz_render_expr`.
///
/// For a SUM result we ALSO verify the sum's VARIANT PAYLOADS are renderable (`sum_payloads_renderable`):
/// `s3_result_ok` alone only recurses over a sum's TYPE-ARGS, so a MONOMORPHIC user sum (no args) would be
/// admitted without checking its payloads — and a variant carrying a FUNCTION payload (`(type Holder (H
/// (-> Int64 Int64)) (Z))`) has no `Display`/value-form render, so it would MIS-RENDER (a gate-differential
/// FAIL). Reading the decl's variant payloads needs `db`, so this takes it.
fn fn_result_renderable(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    let mut cur = ty.strip_nominal();
    while let crate::ty::Ty::Fn(p, r) = cur {
        // NOTE: a returned closure's ARG stays `s2_arg_ok` (NOT `arg_ok_or_fn`) — a higher-order FACTORY
        // (a def whose RESULT is a closure whose arg is itself a closure, e.g. `mk`-ALONE returning
        // `(-> (-> Int64 Int64) Int64)`) has NO consumer sibling to drive it, so the host would have to
        // supply the inner closure over the boundary — which declines (the "closure-typed closure ARG on
        // the DIRECT-CALL path is declined" pin). The higher-order ROUND-TRIP cases work via the CONSUMER
        // path (`app` consumes `mk`), where `mk` is eta-peeled to a consumer — NOT this factory gate.
        if !s2_arg_ok(&db.name_ctx(), p) {
            return false;
        }
        cur = r.strip_nominal();
    }
    // `cur` is the final (non-arrow) result — renderable iff scalar/Float/String/Bytes/Tuple/List/sum
    // (`s3_result_ok`) AND, for a sum, its variant payloads are renderable too (the fn-payload guard).
    s3_result_ok(cur) && sum_payloads_renderable(db, cur)
}

/// For a SUM result type, whether every variant's PAYLOAD is renderable by the gate harness (`s3_result_ok`)
/// — the payload-level check `s3_result_ok`'s type-args recursion cannot do (a monomorphic sum has no args).
/// A FUNCTION payload (`(-> …)`) has no value-form render → NOT renderable → decline (the reviewer-flagged
/// fn-payload user-sum hole). A non-sum type is trivially renderable here (the arms above already gated it).
/// Reads each variant's payload type from the decl via `variant_payload_ty` (substituting this
/// instantiation's args), so a generic sum's concrete payloads are checked too.
fn sum_payloads_renderable(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    let stripped = ty.strip_nominal().clone();
    let Ty::Sum { decl, .. } = &stripped else {
        return true;
    };
    let n = match db.type_decl_by_occ(*decl) {
        Some(td) => td.variants.len(),
        None => return true,
    };
    for disc in 0..n as u32 {
        // A nullary variant has no payload (`None`) — always fine. A payload variant's type must be
        // renderable; recurse so a nested sum payload is checked to any depth.
        if let Some(pty) = expr::variant_payload_ty(db, &stripped, disc)
            && !(s3_result_ok(&pty) && sum_payloads_renderable(db, &pty))
        {
            return false;
        }
    }
    true
}

/// If exported definition `e` is a top-level IMMEDIATE, CAPTURE-FREE lambda — `(def (name) (fn (p…)
/// body))`, a nullary def whose whole body is a `(fn …)` value — return the lifted-lambda slot its body
/// builds; else `None`. This is the ETA-PEEL shape: on the wasm target such an export crosses as a
/// closure RESOURCE (the host retains a handle it later `call`s), but the Rust target has NO resource
/// model — the gate (and any real Rust consumer) applies the export DIRECTLY at full arity, so the
/// faithful Rust rendering is a plain `pub fn name(p…) -> R`, exactly the function the lambda denotes.
/// The peel is sound ONLY when the lambda CAPTURES NOTHING: a top-level `(fn …)` sits in an empty
/// environment (no binding is in scope at module top-level to capture), so its lifted form is a pure
/// combinator whose parameters ARE the export's parameters. A capturing lambda (impossible at top level,
/// but guarded anyway) or a body that is not an immediate lambda (a computed/returned closure) does NOT
/// peel — it stays the closure-resource shape the Rust target declines. Sync mode only (an async peel
/// would thread the `env`; deferred with the rest of async-closure work).
///
/// Returns the lifted slot `code` so the caller can (a) emit the export via that lambda's params/body and
/// (b) SUPPRESS the standalone `__lifted_{code}` (peeling inlines it into the `pub fn`; emitting it too
/// would be dead — and unreferenced, since no `Core::Closure` value survives once the export is a plain fn).
fn peelable_export_lambda(db: &mut Db, e: &crate::layout::ExportPlan, mode: Mode) -> Option<usize> {
    if mode.is_async() || !e.params.is_empty() || !is_fn_ty(&e.result) {
        return None;
    }
    match crate::lower::core_of(db, e.body) {
        crate::core::Core::Closure { code, captures } if captures.is_empty() => {
            // Only peel when the lambda's RESULT renders IDENTICALLY through a direct Rust return as it does
            // through the wasm closure-RESOURCE boundary — else the two backends would disagree on the SAME
            // corpus expectation (a real differential, not a parity win). See `peel_result_render_agrees`.
            let lam = layout_lifted_ret(db, code);
            if peel_result_render_agrees(&lam) {
                Some(code)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The result type of lifted-lambda slot `code`, cloned out of `db.lifted` (read before the peel commits).
fn layout_lifted_ret(db: &Db, code: usize) -> crate::ty::Ty {
    db.lifted[code].ret_ty.clone()
}

/// Whether a peeled export's RESULT type renders the SAME through a direct Rust `pub fn` return as it does
/// through the wasm closure-RESOURCE `call` boundary — the condition for the eta-peel to be a genuine
/// parity win rather than a cross-backend disagreement on one corpus expectation.
///
/// The wasm closure-resource ABI degrades some result types at the boundary in ways a direct Rust return
/// does NOT reproduce, so the corpus expectation (graded green on wasm) is keyed to that degradation.
/// `Bytes`/`String` cross the resource `call` as `list<u8>` / a byte-code list — the corpus expects
/// `(5 6)`/`(104 105)`, but a direct Rust return is a `Vec<u8>`/`String` the gate renders `b"…"`/`"…"`. A
/// `Float32` result trips an emit bug on the peeled path (an `f32 + f64`-literal body → rustc E0308). A
/// `Sum` result crosses with an ABI-specific DOUBLE type-annotation `(: (: v T) T)` the direct return
/// (single `(: v T)`) does not carry. `Symbol`/`Char` share the byte/degradation concern.
///
/// Everything else — integers of any width, `Bool`, `Unit`, `Float64`, tuples/records/lists/sets/maps of
/// safe elements, quantities, big/rational numerics, and a nominal wrapper over a safe type — renders the
/// same either way, so it peels. Applied RECURSIVELY: a compound result is safe only if every leaf is.
/// (A future slice can widen this once the resource-vs-direct render difference for Bytes/String/Sum is
/// resolved at the gate/spec level — filed to the vertical log.)
fn peel_result_render_agrees(ty: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    match ty.strip_nominal() {
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::BigInt | Ty::Rational | Ty::Qty { .. } => true,
        Ty::Float(ft) => ft.ground_width() == 64,
        Ty::Tuple(elems) => elems.iter().all(peel_result_render_agrees),
        Ty::Record(fields) => fields.values().all(peel_result_render_agrees),
        Ty::List(e) | Ty::Set(e) => peel_result_render_agrees(e),
        Ty::Map(k, v) => peel_result_render_agrees(k) && peel_result_render_agrees(v),
        // Bytes/String/Symbol/Char (byte-degraded), Sum (double-annotated), Fn/Var/Any/Type: not peeled.
        _ => false,
    }
}

fn ill_formed_int_width_reject(ty: &crate::ty::Ty) -> Option<Reject> {
    use crate::ty::{Ty, Width};
    let Ty::Int(it) = ty else { return None };
    let Width::Fixed(w) = it.width else {
        return None;
    };
    if (1..=64).contains(&w) {
        return None;
    }
    Some(Reject::coded(
        crate::diag::Code::IntOutOfRange,
        format!(
            "`{}{w}` is not a valid integer type: a width must be in 1..=64 (a fixed-size integer wider \
             than 64 bits is reserved to the big-integer layer, and 0 is not a width)",
            if it.ground_signed() { "Int" } else { "UInt" }
        ),
    ))
}

/// Which calling convention the Rust backend emits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Plain synchronous `fn`s — an ordinary Rust module, no runtime threading.
    Sync,
    /// ASYNC, GAS-METERED `async fn`s that thread a caller-supplied `env: &mut impl CdzEnv` and await
    /// `env.consume(1)` at each function entry, so the host meters fuel and can yield cooperatively.
    /// Every emitted call is `Box::pin(callee(env, …)).await` (the pin makes a recursive `async fn`
    /// well-sized — a recursive future is otherwise infinite; a non-recursive call inlines and never
    /// reaches here). The `CdzEnv` trait is emitted into the module preamble.
    Async,
}

impl Mode {
    fn is_async(self) -> bool {
        matches!(self, Mode::Async)
    }
}

// (The `__CdzE` generic env TYPE-PARAMETER was removed with the uniform-env change: every async fn now
// takes the object-safe `&mut dyn DynCdzEnv` — the same env a lifted closure fn takes — so there is no
// per-fn generic env type param to name. See the async signature emit + the `ENV_PARAM` note below.)

/// The Rust VALUE-PARAMETER name for the async gas/yield env (`async fn f(<this>: &mut dyn DynCdzEnv, …)`),
/// threaded into every emitted call. A `__`-prefixed RESERVED name — NOT a bare `env` — so it cannot
/// collide with a SOURCE parameter literally named `env` (`(def (ev e env) …)`), which a bare `env` would
/// duplicate in the signature (rustc E0415 "bound more than once"). Matches the `__CdzE`/`__pay`/`__p`
/// reserved-name convention; user idents never begin with `__` (the sanitizer does not emit it).
pub(super) const ENV_PARAM: &str = "__cdz_env";

/// The Rust type for a solved Cadenza type, mode-aware: in ASYNC mode a closure-typed position spells the
/// uniform `Rc<dyn EnvClosure<A,R>>` ABI (Option A) via [`types::async_closure_type`]; in SYNC mode (and for
/// any closure-free type in either mode) it is the plain [`types::rust_type`]. Use at every SIGNATURE-emit
/// site (def/lifted param + result) so a closure value (`Rc<dyn EnvClosure>` in async) fits its slot.
pub(super) fn async_or_rust_type(
    ncx: &crate::ty::NameCtx,
    ty: &crate::ty::Ty,
    mode: Mode,
) -> Option<String> {
    if mode.is_async() {
        types::async_closure_type(ncx, ty)
    } else {
        types::rust_type(ncx, ty)
    }
}

/// Emit a Rust-source artifact for the program in `db` under the boundary `layout`. Produces one
/// `pub fn` per export (verbatim name, native scalar signature), reading the shared columns on demand.
/// Declines — attributed to this target — for a construct the scalar slice does not yet render.
///
/// Emits EVERY reachable definition (`layout.order`), not just the exports: an export becomes a
/// `pub fn` (its verbatim name crosses the crate boundary), a reachable NON-export callee — a recursive
/// helper, a mutual-recursion partner — becomes a private `fn`. A `Core::Call` to such a callee then
/// renders as an ordinary Rust call of its emitted `fn`. Reachability is the SAME target-neutral set the
/// wasm backend emits (`layout::compute` closes it over `Core::Call` callees), so the two backends emit
/// the same functions; only the rendering differs. Recursion needs no special handling — a Rust `fn`
/// calls itself directly (native stack), so the wasm backend's tail-call-to-loop transform is simply
/// unnecessary here.
pub fn emit(db: &mut Db, layout: &Layout, mode: Mode) -> Result<Vec<u8>, Reject> {
    // An export's BOUNDARY NAME must be a valid component-model kebab extern name — a LANGUAGE-level
    // ill-formedness (CDZ0201), not a wasm-only load concern: two source names colliding under kebab
    // normalization (`fA` + `f-a` → `f-a`), or a name with a digit-/hyphen-led or non-ASCII segment
    // (`step-by-2`), is rejected on EVERY backend. The wasm backend rejects these at export planning
    // (`kebab_export_collision`/`invalid_kebab_export_name`); the rust backend emits no component, so it
    // would otherwise silently emit a `pub fn` where wasm rejects — a differential outcome. Apply the SAME
    // two checks here so both backends agree (the corpus grades these `(error CDZ0201)`).
    if let Some(reject) = crate::backend::common::export_name::kebab_export_collision(layout) {
        return Err(reject);
    }
    if let Some(reject) = crate::backend::common::export_name::invalid_kebab_export_name(db, layout)
    {
        return Err(reject);
    }
    let mut out = String::new();
    out.push_str(PREAMBLE);
    // In async/gas mode the emitted functions thread the `CdzEnv` gas/yield capability. That trait lives
    // in the SHARED `cdz-rt` crate (not re-declared per module), so an application implements it ONCE and
    // every emitted module interoperates over the same type — bring it into scope with a `use`.
    if mode.is_async() {
        out.push_str(CDZ_RT_IMPORTS);
    }
    // Every sum type the program declares becomes a Rust `enum` (emitted before the functions that
    // construct/match/return it). A declaration with no native form (a recursive sum, an unrepresentable
    // payload) is skipped — a use of it declines at selection, so no orphan enum is emitted.
    out.push_str(&enums::emit_enum_decls(db, mode));
    // A machine-readable descriptor per user sum (variant names + payload types in discriminant order) —
    // inert to rustc (`//` comments), read by the corpus gate to render a user-sum boundary value to its
    // canonical bare form. The enum decls above give rustc the types; these give the gate the structure.
    out.push_str(&enums::emit_sum_descriptors(db));
    // …and a descriptor per erased NEWTYPE (`// cdz-newtype[Pt]: <inner render_name>`), so the gate's value
    // renderer resolves a newtype-typed boundary value (`Pt`) to its erased inner type and renders it
    // structurally rather than `Display`-ing the erased Rust tuple. Inert to rustc (a `//` comment).
    out.push_str(&enums::emit_newtype_descriptors(db));
    // Lifted-lambda slots ETA-PEELED into an export's own `pub fn` (see `peelable_export_lambda`): their
    // body is emitted AS the export, so the standalone `__lifted_{code}` below is suppressed (it would be
    // dead — the peeled export builds no `Core::Closure` value that references it).
    let mut peeled_codes: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for &def in &layout.order {
        let f = match layout.export_plan(def) {
            // An exported definition — a `pub fn` under its verbatim boundary name.
            Some(e) => {
                if let Some(code) = peelable_export_lambda(db, e, mode) {
                    peeled_codes.insert(code);
                }
                emit_export(db, e, layout, mode)?
            }
            // A reachable non-export callee (reached via a runtime `Core::Call`) — a private `fn`.
            None => emit_fn(db, def, layout, mode)?,
        };
        out.push('\n');
        out.push_str(&f);
    }
    // VALUE-DOC EMIT (operator seq-210 parser-elimination, FLAG-GATED `CDZ_VALUE_DOC`, default-OFF). For each
    // NULLARY export whose result shape the Ty-guided walk covers, emit a `pub fn __cdz_doc_<ident>() -> String`
    // that calls the export and builds the self-describing `(: value type)` binary-AST codec doc → the
    // `CDZDOC:<hex>` marker (see `value_doc::emit_result_doc`). The gate driver, under the same flag, calls this
    // INSTEAD of cdz-rust-render's type-note-driven `cdz_render_at` string walk — moving the value walk into
    // rcdzc (Ty-direct, no sexpr re-parse), which is what lets us eventually delete `cdz_render_at` /
    // `parse_head_type` / `rust_call_arg`. GATED because the doc body references `cadenza_ast`, which the gate
    // driver only links (and only calls `__cdz_doc`) when the flag is set — so with the flag UNSET this emits
    // nothing and the module is byte-identical (zero gate impact). A `// cdz-value-doc: <name>` marker note
    // (inert to rustc) tells the driver-gen which exports have a `__cdz_doc` to call. An export whose result
    // shape is not yet covered emits no `__cdz_doc` (and no marker) → the driver falls back to `cdz_render_at`.
    if std::env::var("CDZ_VALUE_DOC").is_ok() {
        for &def in &layout.order {
            let Some(e) = layout.export_plan(def) else {
                continue;
            };
            // NULLARY exports only — the gate's value-doc render path invokes the export with no arguments
            // (`<name>()`); an arg-taking export's driver supplies args, a later slice.
            if !e.params.is_empty() {
                continue;
            }
            let e = e.clone();
            let ident = fn_ident(db, layout, def);
            let call = format!("{ident}()");
            if let Ok(body) = value_doc::emit_result_doc(db, &e.result, &call) {
                // KEY the marker by the sanitized `ident` (== `rust_ident`), NOT the boundary `e.name` — the
                // driver-gen reads its `// cdz-*` notes by `rust_ident` (a hyphenated `mk-b` export emits
                // `__cdz_doc_mk_b`, and the driver calls `prog::__cdz_doc_mk_b()`), so the marker must carry
                // the same key it will construct the fn name from.
                out.push_str(&format!("// cdz-value-doc: {ident}\n"));
                out.push_str(&format!(
                    "#[allow(dead_code)]\npub fn __cdz_doc_{ident}() -> String {{\n{body}}}\n"
                ));
            }
        }
    }
    // REJECT a closure escaping an effect (CDZ0406) — the SAME rule the wasm backend enforces
    // (`backend/wasm/mod.rs`): a lifted-lambda body that performs a host effect carries that effect OUT to
    // the host, to be run when the host later invokes the closure — outside the delegation's dynamic extent,
    // where the effect has no home. A closure's handler context does not travel with it across the boundary.
    // The RUST backend previously had NO such guard, so it tried to EMIT the escaping-effect closure and
    // produced un-compilable Rust (an unresolved host-shim call → E0061) — graded `todo` (BadArtifact) while
    // wasm rejected CDZ0406 (a cross-backend diagnostic differential). Scan the reached lifted bodies for a
    // host import and reject with the same code + message, so both backends agree. (A fully intra-program-
    // HANDLED effect leaves no `Core::HostCall` in the lifted body and is NOT caught here — only an escaping
    // one is.) Placed before the lifted-lambda emit so the reject fires instead of the broken emit.
    {
        // Find the FIRST escaping host effect, then STOP — the reject names one op and doesn't need the full
        // set, so break on the first non-empty scan rather than walking every lifted body (github-liaison
        // PR#1723 efficiency review).
        let mut escaping = Vec::new();
        for k in 0..layout.lifted.len() {
            // Scan a lifted body when it is REACHED (a `Core::Closure` builds it) OR ETA-PEELED into an
            // export (`peeled_codes`): a peeled closure's body IS emitted as the export's `pub fn`, and if
            // that body performs an effect the closure still escapes it to the host — the exact case here
            // (`(def (main) (host (ask) (fn (x) (+ x (ask.ask)))))` peels `main` to `fn main(x)` whose body
            // performs `ask.ask`). Scanning ONLY the non-peeled reached slots (the earlier bug) missed the
            // peeled export, so the reject didn't fire and the broken emit slipped through.
            let peeled = peeled_codes.contains(&k);
            let reached = layout.lifted_reached.get(k).copied().unwrap_or(false);
            if peeled || reached {
                crate::backend::wasm::host::collect_host_imports(
                    db,
                    layout.lifted[k].body,
                    &mut escaping,
                );
                if !escaping.is_empty() {
                    break; // one escaping effect is enough to reject — stop scanning
                }
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
    // Each REACHED lambda-lifted closure (`layout.lifted[k]`, reached by a `Core::Closure` in some body)
    // becomes a private `fn __lifted_{k}(<captures…>, <params…>) -> <ret>` — the closure VALUE a
    // `Core::Closure` builds calls into it. An UNREACHED slot is skipped (no `Core::Closure` names it, so
    // no closure value references it — emitting it would be dead code that might not even type). A slot
    // ETA-PEELED into an export (its body emitted as the `pub fn`) is likewise skipped. A lifted body that
    // declines (an unsupported construct) declines the whole module, exactly like any `fn`.
    for k in 0..layout.lifted.len() {
        if peeled_codes.contains(&k) {
            continue;
        }
        if layout.lifted_reached.get(k).copied().unwrap_or(false) {
            let f = expr::emit_lifted_lambda(db, k, layout, mode)?;
            // Same per-function emit-size backstop as an ordinary body (a lifted lambda can explode too).
            expr::enforce_fn_emit_budget(&f)?;
            out.push('\n');
            out.push_str(&f);
            // ASYNC (Option A uniform ABI): the lifted fn is an `async fn`, and its closure VALUE is
            // `Rc<dyn EnvClosure<A,R>>` — a per-closure synth struct + impl forwarding into the lifted fn.
            // Emit it right after the lifted fn (a `Core::Closure` builds `Rc::new(__Clos_k { … })`).
            if mode.is_async() {
                let s = expr::emit_closure_struct(db, k, layout)?;
                out.push('\n');
                out.push_str(&s);
            }
        }
    }
    // A Float-keyed set/map emits a total-order float wrapper (a bare `f32`/`f64` is not `Ord`). Two
    // width-specific wrappers — `__CdzF64` over `u64` bits, `__CdzF32` over `u32` bits — since the key type
    // maps `Float64`→`__CdzF64` / `Float32`→`__CdzF32` (a `__CdzF64` around an `f32` would not type-check).
    // Each is emitted ONLY when the body references it, detected by scanning for its unambiguous CONSTRUCTOR
    // marker `<name>::new(` (NOT a raw type-name substring — the `__`-prefixed name is backend-reserved so a
    // user ident can never produce it, and `::new(` cannot appear except where the wrap emits it). Inserted
    // right after the preamble, before any use. Gating on the emitted text keeps the wrapper out of the
    // common float-free program (where an unused struct would be dead code). ORDER: F32 then F64, both after
    // the preamble — a program may key on either or both width.
    let insert_at = PREAMBLE.len();
    let mut prelude = String::new();
    // Inject each wrapper's decl when the emitted source USES it. A wrapper name appears in exactly two
    // genuine contexts, so gate on EITHER:
    //  - a COLLECTION TYPE parameter — `BTreeSet<__CdzF64>` / `BTreeMap<__CdzF64, V>` — always spelled
    //    `<__CdzF64` (the key/element is the first type arg). This covers the context-typed EMPTY collection
    //    (`Map.empty`/`Set.of (list)` at a float-keyed type) that annotates the type with NO constructor —
    //    the gap a `::new(`-only gate missed (rustc "cannot find type `__CdzF64`").
    //  - the CONSTRUCTOR — `__CdzF64::new(` — for a collection whose type is INFERRED (a bare
    //    `BTreeMap::new()` seed) so the type name never appears in an annotation, only the wrapped key does.
    // Both markers are collision-free: `sanitize_ident` escapes a leading `__` in every user ident, so a
    // user `(type __CdzF64 …)` emits `enum cdz_user___CdzF64` — which contains the BARE substring `__CdzF64`
    // (why a plain `out.contains("__CdzF64")` would SPURIOUSLY inject the struct) but NEVER `<__CdzF64` (a
    // set-element user sum is `<cdz_user___CdzF64`) nor `__CdzF64::new(` (its ctor is `cdz_user___CdzF64::A`).
    // The F32/F64 markers are distinct substrings, so each fires only for its own width.
    let uses = |w: &str| out.contains(&format!("<{w}")) || out.contains(&format!("{w}::new("));
    if uses("__CdzF32") {
        prelude.push_str(CDZ_F32_DECL);
    }
    if uses("__CdzF64") {
        prelude.push_str(CDZ_F64_DECL);
    }
    // The declared-order Option key/element wrapper (#42 witness 2): a `BTreeSet<__CdzOpt<..>>` type param
    // (`<__CdzOpt`) or its ctor (`__CdzOpt::new(`) — same collision-free marker gating as the float wrappers
    // (the `__`-reserved name never appears in a user ident's emission).
    if uses("__CdzOpt") {
        prelude.push_str(CDZ_OPT_DECL);
    }
    if !prelude.is_empty() {
        out.insert_str(insert_at, &prelude);
    }
    Ok(out.into_bytes())
}

/// The `use` an async-mode module emits to bring the shared runtime traits into scope. The `CdzEnv`
/// gas/yield capability now lives in the `cdz-rt` crate (a single shared definition), NOT re-declared in
/// each module — so an application implements it ONCE for `RcRuntime`/its own env type and every emitted
/// module uses that same trait (two modules interoperate). A downstream build depends on `cdz-rt`; the
/// corpus gate links it via `--extern cdz_rt=<rlib>`.
// WHICH of the three traits a given async module actually references varies, so the `use` is
// `#[allow(unused_imports)]`:
//   - `DynCdzEnv` — every async fn takes `env: &mut dyn DynCdzEnv` and charges gas via `env.consume_boxed(1)`
//     (the uniform-env ABI), so a module with ANY async fn names it.
//   - `EnvClosure` — only a module that emits an async CLOSURE value (its per-closure struct `impl
//     EnvClosure`) names it directly.
//   - `CdzEnv` — the base trait an APPLICATION/the gate harness impls for its concrete env; an emitted
//     module no longer names it directly (async fns take `dyn DynCdzEnv`, not a generic `<__CdzE: CdzEnv>`).
//     It is kept in the `use` only to avoid conditionalizing the preamble on which traits a given module
//     references (simpler to always import all three); its presence does NOT affect the cdz-rt blanket
//     `impl<E: CdzEnv> DynCdzEnv for E` — a blanket impl applies regardless of whether the trait is imported
//     at the use site. Harmless under the `#[allow(unused_imports)]` below.
// A closure-free (or trait-unreferencing) async program imports some of these unused, which `-D warnings`
// (`unused_imports`) would reject — so `#[allow(unused_imports)]` the whole `use` (a mechanically-emitted
// preamble import, like the backend's other synthesized `#[allow(dead_code)]` helpers). Simpler + robust
// than threading per-module trait-usage presence up here.
const CDZ_RT_IMPORTS: &str =
    "#[allow(unused_imports)] use cdz_rt::{CdzEnv, DynCdzEnv, EnvClosure};\n";

/// The file preamble — a header comment marking the source as generated, and the lint allowances a
/// mechanically-emitted file needs (its names come verbatim from the source program, so they will not
/// follow Rust's `snake_case`/`UpperCamelCase` conventions; a nullary export takes no parameters and
/// may return a constant, which trips `clippy`'s "unused" and "trivial" lints — none of which is a
/// defect in generated code).
const PREAMBLE: &str = "\
// @generated by rcdzc (Cadenza → Rust backend). Do not edit by hand.
#![allow(non_snake_case, non_camel_case_types, unused_parens, clippy::all)]
";

/// A TOTAL-ORDER Float64 wrapper for use as a `BTreeSet` element / `BTreeMap` key — the ONE ordered
/// position a bare `f64` cannot occupy (it is `PartialOrd`, not `Ord`: NaN breaks totality). It stores the
/// float's CANONICAL BIT PATTERN and orders/compares by those bits, exactly mirroring the value-heap
/// runtime's `box-float` (`cdz-runtime` `op_box_float`): every NaN — of any incoming bit pattern —
/// canonicalizes to the ONE quiet NaN `f64::NAN.to_bits()` on construction, so two NaNs are the SAME key
/// (the corpus's "a set of two NaN floats dedups to one" / "a NaN map key is found by a differently-produced
/// NaN"); a non-NaN keeps its bits verbatim, so `-0.0` stays DISTINCT from `0.0`. Ordering is by the raw
/// `u64` bits — NOT numeric order — matching the runtime, which orders a float key by its canonical bytes
/// (`Set.to-list` / map enumeration order is by those bytes, not by magnitude). The name is `__`-prefixed
/// (backend-RESERVED — a user ident never begins with `__`, so it can never collide with a `(type CdzF64 …)`
/// the way the bare `CdzF64` did → rustc E0428). Emitted ONLY when a Float64-keyed set/map is present
/// (gated on the `__CdzF64::new(` marker); an unused struct would trip dead-code lints, so `#[allow(dead_code)]`.
const CDZ_F64_DECL: &str = "\
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct __CdzF64(u64);
#[allow(dead_code)]
impl __CdzF64 {
    fn new(v: f64) -> Self { __CdzF64(if v.is_nan() { f64::NAN.to_bits() } else { v.to_bits() }) }
    pub fn get(self) -> f64 { f64::from_bits(self.0) }
}
impl PartialEq for __CdzF64 { fn eq(&self, other: &Self) -> bool { self.0 == other.0 } }
impl Eq for __CdzF64 {}
impl PartialOrd for __CdzF64 { fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) } }
impl Ord for __CdzF64 { fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.0.cmp(&other.0) } }
";

/// The Float32 twin of [`CDZ_F64_DECL`] — a total-order wrapper over the `u32` bit pattern, canonicalizing
/// every NaN to the one quiet `f32::NAN.to_bits()` (the f32 twin of the runtime's `box-float32`). Needed
/// because a `Float32`-keyed set/map maps to `__CdzF32` (a `__CdzF64` around an `f32` value would not
/// type-check, and a lossy `as f64` widen would collapse distinct f32 keys). Same `__`-reserved name +
/// `::new(`-marker gating as the F64 wrapper.
const CDZ_F32_DECL: &str = "\
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct __CdzF32(u32);
#[allow(dead_code)]
impl __CdzF32 {
    fn new(v: f32) -> Self { __CdzF32(if v.is_nan() { f32::NAN.to_bits() } else { v.to_bits() }) }
    pub fn get(self) -> f32 { f32::from_bits(self.0) }
}
impl PartialEq for __CdzF32 { fn eq(&self, other: &Self) -> bool { self.0 == other.0 } }
impl Eq for __CdzF32 {}
impl PartialOrd for __CdzF32 { fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) } }
impl Ord for __CdzF32 { fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.0.cmp(&other.0) } }
";

/// A DECLARED-ORDER `Option` wrapper for use as a `BTreeSet` element / `BTreeMap` key — the ordered
/// position where std `Option<T>`'s DERIVED `Ord` is WRONG for Cadenza. Cadenza declares `Some` (disc 0)
/// `< None` (disc 1) (`prelude sums.rs:80`, core-semantics §Compound Ordering Is Lexicographic), but std
/// `Option` derives `None < Some` — the REVERSE. A `BTreeSet<Option<T>>` would therefore enumerate in the
/// WRONG order (`Set.to-list` head `None` on rust vs `Some 1` on wasm — breaker/corpus-bugfix #42 witness
/// 2). `__CdzOpt<T>` wraps `std::option::Option<T>` and gives it a HAND-WRITTEN `Ord` that puts `Some`
/// before `None` (and orders two `Some`s by payload) — the declared order, matching wasm + the runtime's
/// canonical enumeration. `T: Ord` (the payload's own order); `.get()` reads the inner `Option` back for the
/// value-side (the `to-list` element is a bare `Option<T>`). Generic (one decl serves every payload type),
/// unlike the width-specific float wrappers. `__`-reserved name + emitted ONLY when used (gated on the
/// `__CdzOpt` markers), `#[allow(dead_code)]` for the unused-in-a-non-Option-keyed-program case.
const CDZ_OPT_DECL: &str = "\
#[derive(Clone, PartialEq, Eq)]
#[allow(dead_code)]
struct __CdzOpt<T: Ord>(std::option::Option<T>);
#[allow(dead_code)]
impl<T: Ord> __CdzOpt<T> {
    fn new(v: std::option::Option<T>) -> Self { __CdzOpt(v) }
    fn get(self) -> std::option::Option<T> { self.0 }
}
impl<T: Ord> PartialOrd for __CdzOpt<T> { fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) } }
impl<T: Ord> Ord for __CdzOpt<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Cadenza declared order: Some (disc 0) < None (disc 1); two Somes by payload.
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => a.cmp(b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    }
}
";

/// Emit one exported definition as a `pub fn` — its verbatim boundary name, solved parameter types,
/// and solved result type, from the target-neutral [`ExportPlan`] computed above the seam.
fn emit_export(
    db: &mut Db,
    e: &crate::layout::ExportPlan,
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    // ETA-PEEL: an export whose body is an immediate capture-free lambda `(def (name) (fn (p…) body))`
    // renders as a plain `pub fn name(p…) -> R` — the lambda's OWN parameters + result + body, NOT the
    // (empty) parameter list of the nullary def. On the wasm target this is a closure resource; the Rust
    // target has no resource ABI and applies the export directly, so the lambda IS the function. Emit via
    // `emit_signature` supplying the lifted lambda's params/result/body (its body reads its params as
    // `Core::Param`, which `emit_body` binds by the same binder occurrences carried here). `def` stays the
    // export's def — a self-tail-call inside the lambda body still resolves to it, and `fn_ident` derives
    // the same name for the declaration + every call.
    if let Some(code) = peelable_export_lambda(db, e, mode) {
        let lam = layout.lifted[code].clone();
        // A Unit-RESULT eta-peeled closure export DECLINES — mirroring the wasm target, where an exported
        // closure whose result is `Unit` has no host-boundary form (`closure_boundary_byte(Unit) = None`):
        // a Unit-result closure only makes sense as an effect callback, and effect-escaping closures are
        // forbidden, so a `Unit` result has no boundary role. Without this, the eta-peel emits a
        // `pub fn mk(x) -> ()` the gate driver cannot call as a closure-resource export (E0061). This is
        // the EXPORT-boundary twin of the internal-lift Unit exception in `lower_lambda_value`: an INTERNAL
        // Unit-result closure (boxed, applied via a runtime dispatch) compiles on every backend; only the
        // exported-to-host closure declines. Both faces agree with the wasm target now.
        if matches!(lam.ret_ty, crate::ty::Ty::Unit) {
            return Err(Reject::decline(
                "a closure returning Unit does not cross the Rust export boundary — a Unit-result \
                 closure has no host-boundary form (only an effect callback returns Unit, and \
                 effect-escaping closures are forbidden), matching the wasm target's decline",
            ));
        }
        return emit_signature(
            db,
            &e.name,
            true,
            e.def,
            &lam.params,
            &lam.ret_ty,
            lam.body,
            layout,
            mode,
        );
    }
    emit_signature(
        db, &e.name, true, e.def, &e.params, &e.result, e.body, layout, mode,
    )
}

/// Emit a reachable NON-export definition as a private `fn` — a recursive helper or a mutual-recursion
/// partner a `Core::Call` names. Its name is the source name; its parameters come from
/// [`crate::layout::def_params`] (core types, no boundary-representability constraint — an internal
/// callee never crosses the crate edge); its result type is the body's solved type.
fn emit_fn(db: &mut Db, def: usize, layout: &Layout, mode: Mode) -> Result<String, Reject> {
    let name = db.defs[def].name.clone();
    let body = db.defs[def]
        .body
        .ok_or_else(|| Reject::decline(format!("definition `{name}` has no body")))?;
    let params = crate::layout::def_params(db, def);
    let result = crate::infer::type_of(db, body);
    emit_signature(db, &name, false, def, &params, &result, body, layout, mode)
}

/// Emit a function definition (shared by the export and non-export paths): `[pub] fn <name>(<params>)
/// -> <ret> { <body> }`. Each parameter renders as `<name>: <rust-type>`; a parameter type with no
/// native mapping (an unresolved/ambiguous or not-yet-supported type) declines. The result type maps
/// the same way (unit → `()`; a compound declines in the scalar slice). The body is the core of `body`
/// rendered as a Rust expression, with the parameters in scope by their emitted names.
#[allow(clippy::too_many_arguments)]
fn emit_signature(
    db: &mut Db,
    name: &str,
    public: bool,
    def: usize,
    params: &[(crate::ast::StructId, crate::ty::Ty)],
    result: &crate::ty::Ty,
    body: crate::ast::StructId,
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    // A closure PARAMETER crossing the EXPORT boundary declines: an exported fn is called by the gate
    // driver (and any real consumer) with VALUE arguments written as literals, and there is no way to
    // synthesize an `Rc<dyn Fn>` argument at that boundary. (The corpus's closure round-trip cases pass a
    // scalar where the export's closure PARAM sits, expecting the wasm handle-ABI to route it; the Rust
    // target has no such boundary.) An INTERNAL closure — passed to a recursive helper — is unaffected: it
    // never crosses an export edge, so this guard (gated on `public`) does not touch it. Decline cleanly.
    //
    // A closure RESULT (a "closure FACTORY" export — `(def (both (: a)(: b)) (fn (x) …))`) IS representable:
    // the def's captured params stay ordinary leading params, and the returned `(fn …)` is already emitted
    // as an `Rc<dyn Fn(…)->…>` VALUE (the internal-closure lowering — `Core::Closure` → `Rc::new(move |args|
    // __lifted_k(captures, args))`), which `types::rust_type(Ty::Fn)` maps to the `-> Rc<dyn Fn(…)->…>`
    // return type. So the host calls `both(10, 20)` → an `Rc<dyn Fn>` handle, then applies it `(5)` — the
    // native equivalent of the wasm make/call handle ABI (the gate harness splits the flat call args at the
    // factory param count). SCOPE (S1+S2): allow it when the returned closure's ARGS are each an S1 scalar
    // OR an S2 compound (`fn_result_is_scalar_only` → `s2_arg_ok`: Tuple/List the harness rebuilds) AND the
    // RESULT is an S1 scalar. A compound RESULT (S3) still declines cleanly (a `todo`, not a wrong-value/
    // non-build fail): the factory emits a valid `Rc<dyn Fn>`, but the harness result-render for a compound
    // is a later slice. Also require the factory's OWN params (the CAPTURES — the leading `both(a, b)` args
    // the host supplies at `make`) to be S1 SCALARS: a compound capture (a host-supplied Tuple/record — the
    // "producer capturing a host-supplied COMPOUND parameter is declined" case) is not yet rebuilt at the
    // make-split. Defer compound captures + compound results + Float/String/Bytes/sum args to later slices.
    if public
        && is_fn_ty(result)
        && (!fn_result_renderable(db, result) || !params.iter().all(|(_, t)| is_capture_scalar(t)))
    {
        return Err(Reject::unsupported(format!(
            "`{name}`: a closure-returning export with a non-scalar capture/arg/result is not supported by the Rust backend (host-closure S2/S3)"
        )));
    }
    // A closure PARAMETER crosses the export boundary as an `Rc<dyn Fn(…)->…>` argument (which
    // `rust_type` renders) and the body applies it (`Core::CallClosure` → `(g)(x)` — already emitted). The
    // host supplies the closure: the gate harness builds it from a companion PRODUCER export and passes it
    // (see `run_program_rust`'s consumer branch). SCOPE (this slice = the SIMPLE closure-param shape): the
    // closure's arg AND result are SCALARS (Int/Bool/Float), so the harness's producer→consumer synthesis
    // is unambiguous. A HIGHER-ORDER closure param (its arg is itself a closure), a COMPOUND closure
    // arg/result, or an export whose OWN result is compound (needs the S3 factory-style render the consumer
    // path doesn't do yet) still DECLINES — a clean `todo`, a later increment — rather than emit an
    // artifact the harness drives wrongly (a FALSE gate FAIL).
    let sig_ncx = db.name_ctx();
    let closure_param_is_simple = |t: &crate::ty::Ty| -> bool {
        // Peel the arrow spine. Each closure ARG may be an S1 scalar OR an S2 compound the harness rebuilds
        // (`s2_arg_ok`: Tuple/List/Option/Result over OK elements) — the consumer applies `(g <arg>)` where
        // `<arg>` is built in ITS OWN body (a literal/constructed value the emitter already lowers), and the
        // producing sibling that supplies `g` already emits a matching compound-arg closure via the factory
        // S2 path (`fn_result_renderable`). A HIGHER-ORDER arg (an arg that is itself a `Fn`) stays declined
        // — `s2_arg_ok` returns false for `Ty::Fn`. The final RESULT may be an S1 scalar OR an S2 compound
        // (`s2_arg_ok`: Tuple/List/Option/Result over OK elements): the closure result flows into the
        // CONSUMER's body — `(g <args>)` yields it and the consumer returns/reads it — and the producing
        // factory ALREADY emits a matching compound-RESULT closure (its `fn_result_renderable` S3 path). The
        // consumer's OWN export result is rendered/declined SEPARATELY (`result_render_unsupported` below),
        // so a compound closure result is admitted exactly when it is a type the emitter lowers natively (a
        // Tuple/List the `(g …)` call yields directly). A HIGHER-ORDER result (`Ty::Fn`) stays declined —
        // `s2_arg_ok` returns false for it. (This lands the S3-compound-closure-result consumer shape.)
        let mut cur = t.strip_nominal();
        while let crate::ty::Ty::Fn(p, r) = cur {
            // S4-HIGHER-ORDER: a closure ARG may itself be a closure (`arg_ok_or_fn` admits `Ty::Fn`), so a
            // consumer taking `g: (-> (-> Int64 Int64) Int64)` is admitted — the inner closure it applies
            // `g` to is built IN-GUEST (`(g (fn (y) …))`, already lowered), and the harness supplies `g`
            // from its higher-order producer sibling (`Rc::new(mk)`).
            if !arg_ok_or_fn(&sig_ncx, p) {
                return false;
            }
            cur = r.strip_nominal();
        }
        s2_arg_ok(&sig_ncx, cur)
    };
    // A closure param needs a PRODUCING sibling export to supply its closure — either a FACTORY (an export
    // whose result is that closure type) or a PEELED producer (a nullary `(fn …)` eta-peeled to a direct
    // `fn(args)->ret` whose signature IS the closure's arg/result shape). Without one, the host would have
    // to supply the closure DIRECTLY at the boundary, which has no rust (or wasm) representation — wasm
    // declines this "closure argument … has no scalar host-boundary representation"; match it. Check by
    // type-equality against sibling exports' plans (this export excluded — a self-produced closure is not
    // a thing here). A closure `(-> A… R)` matches a factory result of the SAME `Ty::Fn`, or a peeled
    // producer whose params are `A…` and result is `R`.
    let has_producer_for = |closure_ty: &crate::ty::Ty| -> bool {
        let ct = closure_ty.strip_nominal();
        // The closure's arg types + result, for matching a peeled producer's params/result.
        let mut arg_tys = Vec::new();
        let mut cur = ct;
        while let crate::ty::Ty::Fn(p, r) = cur {
            arg_tys.push((*p).clone());
            cur = r.strip_nominal();
        }
        let closure_ret = cur;
        layout.exports.iter().any(|ep| {
            if ep.def == def {
                return false; // not self
            }
            // FACTORY: sibling result IS the closure type.
            if ep.result.strip_nominal() == ct {
                return true;
            }
            // PEELED: sibling is a plain fn whose params == the closure's arg types and result == closure
            // result (a nullary `(fn …)` eta-peeled to a direct fn).
            ep.params.len() == arg_tys.len()
                && ep
                    .params
                    .iter()
                    .zip(&arg_tys)
                    .all(|((_, pt), at)| pt.strip_nominal() == at.strip_nominal())
                && ep.result.strip_nominal() == closure_ret
        })
    };
    // S4-HIGHER-ORDER: whether THIS def is itself a PRODUCER for some sibling export's closure param — i.e.
    // this def's own signature, viewed as a closure `(-> param0 param1 … result)`, MATCHES a `Ty::Fn` param
    // of another export. In the higher-order round-trip, `mk : (-> (-> Int64 Int64) Int64)` is `app`'s `g`
    // param's producer; `mk` itself takes a closure param `f : (-> Int64 Int64)` that has NO producer
    // sibling — but that's fine, because `mk`'s `f` is supplied IN-GUEST by `app` (`(g (fn (y) …))`), never
    // by the host. So the host NEVER calls `mk` directly with a synthesized `f`; it passes `mk` (as
    // `Rc::new(mk)`) as `app`'s `g`, and `app`'s body builds the inner closure. Thus a consumer whose
    // closure param lacks a producer is STILL emittable when the consumer is itself such a producer — its
    // in-guest-fed closure param needs no host synthesis. Emit it as a plain `pub fn` (rustc compiles it;
    // the harness drives only the OUTER consumer `app`).
    let def_is_producer_for_sibling = || -> bool {
        // Build this def's own closure type: `(-> p0 (-> p1 … result))` (curried), matching how a closure
        // param's arrow spine is written. A def with NO params can't be a closure producer here.
        if params.is_empty() {
            return false;
        }
        let mut own_fn = result.clone();
        for (_, pt) in params.iter().rev() {
            own_fn = crate::ty::Ty::Fn(Box::new(pt.clone()), Box::new(own_fn));
        }
        let own_fn = own_fn.strip_nominal();
        // Does any OTHER export have a `Ty::Fn` param equal to this def's own closure type?
        layout.exports.iter().any(|ep| {
            ep.def != def
                && ep
                    .params
                    .iter()
                    .any(|(_, pt)| is_fn_ty(pt) && pt.strip_nominal() == own_fn)
        })
    };
    // Whether a closure param has a FACTORY producer specifically (a sibling whose RESULT is the closure
    // type) — a STRICTER form of `has_producer_for` (which also accepts a PEELED producer). In ASYNC mode
    // the driver can drive a factory producer through `block_on` (the factory is an `async fn` returning a
    // sync `Rc<dyn Fn>`), but a PEELED async producer is an `async fn` whose fn-item does NOT coerce to the
    // `fn(args)->ret` the closure value needs — so an async consumer is admitted only when EVERY closure
    // param has a factory producer (see the async gate below). In SYNC mode both producer shapes work.
    let closure_has_factory_producer = |closure_ty: &crate::ty::Ty| -> bool {
        let ct = closure_ty.strip_nominal();
        layout
            .exports
            .iter()
            .any(|ep| ep.def != def && ep.result.strip_nominal() == ct)
    };
    // The export's OWN result: a Tuple/Option/Result/List/sum result renders fine via the driver's
    // `cdz_render_expr`, and a BARE Bytes/String result now renders too — the driver's consumer path routes
    // it through `cdz_render_bytes_list` (the byte-int list `(104 105)` form the value takes crossing the
    // host boundary as `list<u8>`, mirroring the factory branch), so a bare String/Bytes result no longer
    // declines. (A String/Bytes nested in a COMPOUND result renders via `cdz_render_expr` as before — not
    // gated here, unchanged by this slice.)
    let result_render_unsupported = false;
    // ASYNC closure-PARAMETER consumers: the async gate driver builds each closure from its producer sibling
    // and drives it through `block_on`, threading `&mut env`. This works when every closure param has a
    // FACTORY producer (an `async fn` returning a sync `Rc<dyn Fn>` the driver `block_on`s + binds to a
    // `let`). A PEELED producer (a nullary `(fn …)` eta-peeled to a direct fn) does NOT work in async — its
    // `async fn` item does not coerce to the `fn(args)->ret` the closure value needs — so an async consumer
    // with a peeled (non-factory) producer for any closure param still DECLINES (clean `todo`, a follow-up).
    if mode.is_async()
        && public
        && params.iter().any(|(_, t)| is_fn_ty(t))
        && !params
            .iter()
            .all(|(_, t)| !is_fn_ty(t) || closure_has_factory_producer(t))
    {
        return Err(Reject::unsupported(format!(
            "`{name}`: an async closure-PARAMETER consumer whose closure has no FACTORY producer sibling is not supported by the Rust backend"
        )));
    }
    // S4-HIGHER-ORDER: a closure param is admissible when it is shape-OK (`closure_param_is_simple`) AND
    // EITHER it has a producer sibling (`has_producer_for` — the host synthesizes it) OR this def is itself
    // a producer for a sibling's closure param (`def_is_producer_for_sibling` — the param is fed IN-GUEST by
    // that sibling, so no host synthesis is needed and the def just emits as a plain `pub fn`). The
    // `def_is_producer_for_sibling` check is def-wide (not per-param), evaluated once — it says "this whole
    // def is a producer, so its closure params are guest-internal", which is the higher-order producer case.
    let is_producer_for_sibling = def_is_producer_for_sibling();
    if public
        && params.iter().any(|(_, t)| is_fn_ty(t))
        && (!params.iter().all(|(_, t)| {
            !is_fn_ty(t)
                || (closure_param_is_simple(t) && (has_producer_for(t) || is_producer_for_sibling))
        }) || result_render_unsupported)
    {
        return Err(Reject::decline(format!(
            "`{name}`: this closure-PARAMETER export shape (higher-order / compound arg / Bytes-String result / no producing sibling) does not cross the Rust export boundary"
        )));
    }
    // Whether this function is compiled as a `loop` (it self-tail-calls). A looped function REASSIGNS
    // its parameter locals each iteration, so they are declared `mut`. Detected once here and again in
    // `emit_body` (both read the same predicate), so the signature's `mut` and the body's loop agree.
    let loops = !params.is_empty() && expr::body_loops(db, def);
    let mut param_src = String::new();
    // In async/gas mode, the FIRST parameter is the caller-supplied gas/yield env, threaded into every
    // call. It precedes the source parameters; the source params keep their positions after it. The env is
    // the OBJECT-SAFE `&mut dyn DynCdzEnv` — NOT a generic `&mut __CdzE: CdzEnv` — so a top-level async fn
    // takes the SAME env type a lifted CLOSURE fn does (a closure value is `Rc<dyn EnvClosure>` holding a
    // `&mut dyn DynCdzEnv`, and its body CALLS these top-level fns; a generic `__CdzE` param would reject the
    // `dyn DynCdzEnv` the closure passes — `dyn DynCdzEnv: CdzEnv` is unsatisfied). Uniform env everywhere =
    // one env type across fns + closures, and it also drops the per-fn `<__CdzE: CdzEnv>` generic. A concrete
    // caller env (`&mut GateEnv`) unsizes to `&mut dyn DynCdzEnv` at the call site. Gas charges via the
    // object-safe `consume_boxed` (the RPITIT `consume` is not callable on a `dyn`).
    if mode.is_async() {
        param_src.push_str(&format!("{ENV_PARAM}: &mut dyn DynCdzEnv"));
    }
    for (i, (binder, ty)) in params.iter().enumerate() {
        if i > 0 || mode.is_async() {
            param_src.push_str(", ");
        }
        let pname = param_name(db, *binder, i);
        // A sum type whose Rust `enum` did NOT emit (a recursive sum needs `Box`, deferred) has a name
        // but no declaration — a signature naming it would not compile (`cannot find type IntList`). So a
        // function that takes such a type declines HERE, consistently with the skipped enum decl.
        if !enums::sum_representable(db, ty) {
            let reason = enums::unrepresentable_reason(db, ty);
            return Err(Reject::decline(format!(
                "`{name}`: parameter type {} is {reason}",
                ty.render_name(&db.name_ctx())
            )));
        }
        // An ILL-FORMED integer width in a parameter type is a REJECT (CDZ0302), not a target decline —
        // catch it before the codeless "no native rep" decline so the diagnostic matches the wasm target.
        if let Some(reject) = ill_formed_int_width_reject(ty) {
            return Err(reject);
        }
        // In ASYNC mode a closure-typed param spells the uniform `Rc<dyn EnvClosure<A,R>>` ABI (a closure
        // value flowing in is `Rc<dyn EnvClosure>`, not `Rc<dyn Fn>`); `async_closure_type` == `rust_type`
        // for any closure-free type, so a scalar/compound param is byte-identical in both modes.
        let rty = async_or_rust_type(&db.name_ctx(), ty, mode).ok_or_else(|| {
            Reject::decline(format!(
                "`{name}`: parameter type {} has no native Rust representation",
                ty.render_name(&db.name_ctx())
            ))
        })?;
        // A looped function's params are reassigned per iteration → `mut`.
        let mut_kw = if loops { "mut " } else { "" };
        param_src.push_str(&format!("{mut_kw}{pname}: {rty}"));
    }
    // Same guard on the RESULT: a function returning a recursive sum (no emitted enum) declines.
    if !enums::sum_representable(db, result) {
        let reason = enums::unrepresentable_reason(db, result);
        return Err(Reject::decline(format!(
            "`{name}`: result type {} is {reason}",
            result.render_name(&db.name_ctx())
        )));
    }
    // A DIVERGING body — one that provably never returns a value (a bare `(trap …)`, a zero-arm match on a
    // `Never` scrutinee, OR a compound whose every path diverges: a both-branches-diverge `(if b (trap)
    // (trap))`, an all-arms-diverge match, a `let`/`seq` whose tail diverges) — has a `Never` result type
    // (a fresh `Ty::Var`/`Any`) with no native Rust rep, but NO value ever returns: the body `panic!`s on
    // every path. Emit Rust's NEVER type `!` as the return type (`fn main() -> ! { … }` is valid),
    // mirroring the wasm backend which crosses such an export as a no-result function. Uses the SHARED
    // `body_diverges` predicate (the Core-level divergence check the wasm backend also drives — ONE
    // definition, so the two backends agree on what diverges) instead of matching a bare `Core::Trap`,
    // which missed a both-diverge `if`/match (v-wasm-opt + breaker's Never-in-emit-position family).
    // Checked BEFORE the `rust_type` decline so a diverging `Any`/`Var` result is not misdiagnosed as an
    // unrepresentable type. A genuinely-unconstrained (non-diverging) result var still declines below.
    let diverges = types::rust_type(&db.name_ctx(), result).is_none()
        && crate::backend::common::diverge::body_diverges(db, body);
    // An ILL-FORMED integer width in the RESULT type is a REJECT (CDZ0302), not a decline — the twin of
    // the parameter check above, matching the wasm target (`(: 5 (Int -8))` → CDZ0302, not a codeless
    // decline). NOT for a DIVERGING body: it produces no value, so a `!` return is legitimate regardless of
    // the nominal result width (the `diverges` guard below wins). Checked before the type-mapping decline.
    if !diverges && let Some(reject) = ill_formed_int_width_reject(result) {
        return Err(reject);
    }
    let ret = if diverges {
        "!".to_string()
    } else {
        // Async: a closure-typed RESULT (a factory export returning a closure) spells the `EnvClosure` ABI.
        async_or_rust_type(&db.name_ctx(), result, mode).ok_or_else(|| {
            Reject::decline(format!(
                "`{name}`: result type {} has no native Rust representation",
                result.render_name(&db.name_ctx())
            ))
        })?
    };
    // Render the body against the parameter environment. Selection reads the core + type columns on
    // demand, so a fault deep in the body surfaces here as a decline. In async mode the body's calls
    // become `Box::pin(callee(env, …)).await`; a self-tail-recursive body becomes a `loop` (so `def` is
    // passed to detect a self-call).
    let body_src = expr::emit_body(db, body, params, def, layout, mode)?;
    // Per-function emit-size backstop: a handler-derived Core DAG re-descended per reference can serialize
    // into ONE multi-MB function body rustc cannot build ("artifact did not build"); decline it cleanly
    // instead. See `expr::RUST_FN_EMIT_BUDGET`. Durable linear fix = sharing-aware emit (separate increment).
    expr::enforce_fn_emit_budget(&body_src)?;
    let vis = if public { "pub " } else { "" };
    // The function NAME via `fn_ident` — sanitized (`sum-to` → `sum_to`) and UNIQUED per definition when a
    // β-copied do-local worker would otherwise emit two `fn`s of the same name (E0428). The SAME mapping a
    // `Core::Call` uses at the call site (it also calls `fn_ident`), so the declaration and every call
    // agree — including a recursive self-call, which resolves to this def and so re-derives this ident.
    let ident = fn_ident(db, layout, def);
    // A machine-readable note of the fn's CADENZA result type — its `render_name` (e.g. `Int64`,
    // `(Tuple Int64 Bool)`, `(Record (a Int64) (b Int64))`). The Rust return type erases the structural
    // detail a boundary render needs (field NAMES, `Tuple`-vs-`Record` distinction), so a consumer that
    // must reproduce the value's canonical text form — the corpus gate — reads it from here. Inert to
    // rustc (a `//` comment); present on every emitted fn, keyed by ident so a caller finds the right one.
    // For a DIVERGING body the emitted return type is `!` (not a value type); note it as `!` so the gate
    // driver recognizes the export never returns and CALLS it without a `println!` (binding/printing a `!`
    // is an "unreachable statement" + `()`-not-`Display` build error). A `Never` result's `render_name` is
    // `_` — indistinguishable from other holes and NOT one of the driver's diverging markers — so keying the
    // note on the actual emitted `!` type is what makes the driver's divergence handling fire.
    let ret_note = if diverges {
        format!("// cdz-return[{ident}]: !\n")
    } else {
        format!(
            "// cdz-return[{ident}]: {}\n",
            boundary_return_render_name(db, result)
        )
    };
    // For a QUANTITY result, ALSO emit the unit's canonical VALUE-form spelling (`// cdz-unit[ident]:
    // <value-form>`) beside the type note. `render_name` carries the unit as `Unit::render` — the TYPE
    // surface (bare `(Unit.base …)`, `Unit.*`/`Unit.^ -1` for a derived unit) — but cdz-run prints a
    // quantity VALUE with the DOTTED value-form unit (`((. Unit base) …)`, a `Unit./` quotient for a
    // derived unit). The gate's boundary render needs THAT spelling, and reconstructing it from the type
    // string is fragile; `render_value_form` produces it directly (mirroring `lower::unit_value_ast`), so
    // the driver splices it verbatim. Inert to rustc; keyed by ident like the return note.
    let unit_note = match result {
        crate::ty::Ty::Qty { unit, .. } => {
            // A quantity DISPLAYS at its dimension's REFERENCE unit (scale dropped) — `5 kilometer` prints
            // `5000 meter`, NOT `5 kilometer`. So the value-form unit is `unit.at_reference()` (the same
            // exponent map at scale 1/1). For a scale-1 unit this is `unit` itself (byte-neutral). Plus, a
            // NON-scale-1 unit needs the magnitude SCALED to that reference: emit its `num/den` in a
            // `// cdz-scale[ident]:` note so the harness multiplies the boundary value (a scale-1 unit emits
            // NO scale note — the magnitude is displayed as stored). Both notes are inert `//` comments.
            let (num, den) = unit.scale();
            let scale_note = if (num, den) == (1, 1) {
                String::new()
            } else {
                format!("// cdz-scale[{ident}]: {num}/{den}\n")
            };
            format!(
                "// cdz-unit[{ident}]: {}\n{scale_note}",
                unit.at_reference().render_value_form()
            )
        }
        _ => String::new(),
    };
    // PER-ELEMENT quantity display-scale notes for a COMPOUND result carrying non-scale-1 Qty leaves (a
    // `(Tuple (Qty Float64 km) (Qty Rational mile))` — each element display-scales to its reference
    // INDEPENDENTLY). The single `// cdz-scale[ident]` note above only scales a TOP-LEVEL bare Qty; a Qty
    // nested in a tuple/record has no per-element scale, so the harness rendered it RAW (`5.0`/`5/1` instead
    // of `5000.0`/`201168/25` — the rust-red v-quantity/v-core-opt found). Emit one `// cdz-qty-at[ident]:
    // <path> <num>/<den>` per non-scale-1 Qty leaf (path = the render's positional `.i` descent); the harness
    // multiplies that leaf's magnitude by the scale in its inner type (Float IEEE, Int trunc, Rational exact),
    // mirroring wasm `const_value_ast_scaled`. Scale-1 leaves emit no note (rendered as stored).
    let qty_at_notes = {
        let mut paths = Vec::new();
        collect_qty_scale_paths(db, result, "", &mut paths, &mut Vec::new());
        paths
            .iter()
            .map(|(p, num, den)| format!("// cdz-qty-at[{ident}]: {p} {num}/{den}\n"))
            .collect::<String>()
    };
    // CLOSURE-PARAM SHAPE note (`// cdz-param-shapes[<ident>]: <arrow> | <arrow> | …`) — one entry per
    // fn-typed (closure) parameter, IN PARAMETER ORDER, each the param's Cadenza arrow type via `render_name`
    // (`(-> (Tuple Int64 Int64) Int64)` vs `(-> (Record (a Int64) (b Int64)) Int64)`). WHY: a Tuple-arg and a
    // Record-arg closure ERASE to the identical Rust `Rc<dyn Fn((i64,i64)) -> i64>`, so the gate driver's
    // producer↔consumer pairing (which compares that erased type) can MISPAIR them in a distinct-signature
    // two-closure consumer. This note carries the pre-erasure shape the driver matches against each producer
    // factory's `cdz-return` arrow type (also `render_name`, same distinction) to disambiguate. Emitted only
    // for a PUBLIC export with ≥1 fn-typed param (a consumer); inert `//` comment, keyed by ident. The `|`
    // separator can't occur in a `render_name` (which uses `(-> …)`/spaces), so the driver splits cleanly.
    let param_shapes_note = {
        let shapes: Vec<String> = params
            .iter()
            .filter(|(_, t)| is_fn_ty(t))
            .map(|(_, t)| t.render_name(&db.name_ctx()))
            .collect();
        if public && !shapes.is_empty() {
            format!("// cdz-param-shapes[{ident}]: {}\n", shapes.join(" | "))
        } else {
            String::new()
        }
    };
    // PRODUCES-CLOSURE note (`// cdz-produces-closure[<ident>]: <arrow>`) — the CADENZA arrow a PEELED
    // producer supplies (`(-> <param shapes> <result>)`, via `render_name`). WHY: a peeled producer (a
    // nullary `(fn (p) …)` eta-peeled to `pub fn mka(p: (i64,i64)) -> i64`) loses its arrow — its
    // `cdz-return` is the SCALAR result (`Int64`), not the closure shape — so the driver cannot tell a
    // Tuple-arg peeled producer from a Record-arg one (both erase to `fn((i64,i64))->i64`). This note gives
    // the pre-erasure arrow the driver matches against a consumer's `cdz-param-shapes` (the async FACTORY
    // producer already carries the arrow in its own `cdz-return`, so this covers the SYNC peeled case). Only
    // for a PUBLIC export with NO closure param + a non-closure result (the peeled-producer candidate shape);
    // inert `//` comment. The arrow uses the same `render_name` the consumer note does, so they string-match.
    let produces_closure_note = {
        let is_peeled_producer_shape =
            public && !is_fn_ty(result) && !params.iter().any(|(_, t)| is_fn_ty(t));
        if is_peeled_producer_shape && !params.is_empty() {
            let arrow = params
                .iter()
                .rev()
                .fold(result.render_name(&db.name_ctx()), |acc, (_, t)| {
                    format!("(-> {} {acc})", t.render_name(&db.name_ctx()))
                });
            format!("// cdz-produces-closure[{ident}]: {arrow}\n")
        } else {
            String::new()
        }
    };
    let ret_note =
        format!("{ret_note}{unit_note}{qty_at_notes}{param_shapes_note}{produces_closure_note}");
    if mode.is_async() {
        // `async fn <name>(env: &mut dyn DynCdzEnv, …) -> <ret> { env.consume_boxed(1).await; <body> }` —
        // the per-call fuel charge + cooperative-yield point at entry. The env is the OBJECT-SAFE `&mut dyn
        // DynCdzEnv` (see the param assembly above): uniform with a lifted closure fn's env, so a closure
        // body can call these top-level fns. Gas charges via `consume_boxed` (the object-safe method — the
        // RPITIT `CdzEnv::consume` is not callable through a `dyn`). No per-fn `<__CdzE: CdzEnv>` generic.
        Ok(format!(
            "{ret_note}{vis}async fn {ident}({param_src}) -> {ret} {{\n    {ENV_PARAM}.consume_boxed(1).await;\n{body_src}\n}}\n"
        ))
    } else {
        Ok(format!(
            "{ret_note}{vis}fn {ident}({param_src}) -> {ret} {{\n{body_src}\n}}\n"
        ))
    }
}

/// The Rust identifier for parameter `index`, from its source name occurrence. Falls back to a
/// positional `p{index}` when the occurrence carries no readable name (a defensive default — an
/// exported parameter always has a name in practice).
fn param_name(db: &Db, binder: crate::ast::StructId, index: usize) -> String {
    db.ast
        .as_name(binder)
        .map(sanitize_ident)
        .unwrap_or_else(|| format!("p{index}"))
}

/// Make a source name a valid, non-colliding Rust identifier. Cadenza names allow characters Rust
/// identifiers do not (notably `-`, the idiomatic word separator — `sum-to`), so each such character
/// becomes `_`; a name that would start with a digit is prefixed. The mapping is deterministic, so a
/// reference to the same name maps the same way everywhere it appears.
pub(crate) fn sanitize_ident(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()) {
            s.push(c);
        } else if c.is_ascii_digit() {
            // A leading digit: prefix so the identifier is valid.
            s.push('_');
            s.push(c);
        } else {
            // Any other character (notably `-`) becomes an underscore.
            s.push('_');
        }
    }
    if s.is_empty() {
        s.push('_');
    }
    // A LEADING `__` is the backend-RESERVED namespace: the emitter injects `__`-prefixed idents for its
    // own machinery — `__CdzF32`/`__CdzF64` (the float-key wrappers), `__CdzE`/`__cdz_env` (the async gas
    // env), `__pay`/`__p` (match-payload locals), `__lifted_N`, the render `__r`/`__e{n}`/… locals. A
    // Cadenza name CAN legally begin with `_` (the lexer's `is_ident_start` accepts it) and would otherwise
    // pass through here UNCHANGED — so a user `(type __CdzF64 …)` / `(def __pay …)` would emit the SAME
    // Rust ident as the injected one → rustc E0428 duplicate definition / a captured local. Escape a
    // leading `__` with a `cdz_user_` prefix so a user ident can NEVER land in the `__`-reserved space
    // (a generated `__…` never starts with `cdz_user_`, and this map is applied at BOTH the declaration and
    // every reference — all through `sanitize_ident` — so they still agree). A single leading `_` is left
    // alone (only the DOUBLE underscore is reserved), keeping the common `_unused`-style name readable.
    if s.starts_with("__") {
        return format!("cdz_user_{s}");
    }
    // A Cadenza identifier may be a RUST KEYWORD (`loop`, `type`, `while`, `for`, `mut`, `impl`, …) — a
    // valid Cadenza name but reserved in Rust, so emitting it verbatim as a `fn`/binder name is invalid Rust
    // (`fn loop(…)` → rustc "expected `{`, found `(`"). rustc round-trips a keyword-named symbol as a RAW
    // identifier `r#loop`, accepted for EVERY keyword except a handful (`crate`/`self`/`Self`/`super` can't
    // be raw — and `_` is the wildcard, not a name) — mangle those with a reserved prefix instead. This is
    // the identifier-emission twin of the `-`→`_` sanitization above; the SAME mapping applies at the `fn`
    // declaration and every call/reference (all go through `sanitize_ident`), so they agree. wasm is
    // unaffected (function names there are indices, not identifiers).
    if is_rust_raw_ident_exception(&s) {
        return format!("cdz_kw_{s}");
    }
    if is_rust_keyword(&s) {
        return format!("r#{s}");
    }
    s
}

/// The Rust keywords that CANNOT be written as a raw identifier (`r#…`) — so a Cadenza def/binder named one
/// is mangled with a reserved prefix instead. `_` is the wildcard (never a raw ident); the rest are the
/// path-sensitive keywords rustc rejects after `r#`.
fn is_rust_raw_ident_exception(s: &str) -> bool {
    matches!(s, "crate" | "self" | "Self" | "super" | "_")
}

/// Whether `s` is a Rust reserved word (a strict OR reserved keyword) — one that must be emitted as a raw
/// identifier `r#s` when it is a Cadenza-source name. Excludes the raw-ident exceptions (handled by
/// [`is_rust_raw_ident_exception`]). The set is the Rust 2021 keyword list; `match`/`fn`/`if`/`else`/`let`
/// are Cadenza reserved words too (they never reach here as a def name) but are listed for completeness so
/// any surviving occurrence is escaped rather than emitted raw.
fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        // strict keywords
        "as" | "break" | "const" | "continue" | "dyn" | "else" | "enum" | "extern" | "false"
            | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move"
            | "mut" | "pub" | "ref" | "return" | "static" | "struct" | "trait" | "true" | "type"
            | "unsafe" | "use" | "where" | "while"
            // 2018+ strict
            | "async" | "await"
            // reserved (future) keywords
            | "abstract" | "become" | "box" | "do" | "final" | "macro" | "override" | "priv"
            | "typeof" | "unsized" | "virtual" | "yield" | "try" | "gen"
    )
}

/// The Rust `fn` identifier for a reachable definition — the ONE name both the declaration
/// ([`emit_fn`]/[`emit_export`]) and every `Core::Call` to it must agree on.
///
/// An EXPORT keeps its verbatim boundary name (it crosses the crate edge; export names are unique). A
/// non-export uses [`sanitize_ident`], UNIQUED per definition when its sanitized name COLLIDES with another
/// emitted definition's. The collision arises from β-copying: a helper with a do-local recursive worker
/// (`def helper(x) = (def (fac n) …); fac(x)`) called from N sites is inlined N times, each copy carrying
/// its OWN `fac` DEFINITION (a distinct `db.defs` index) but the SAME source name — so N `fn fac` at module
/// scope, which rustc rejects (E0428 "the name `fac` is defined multiple times"). The wasm backend never
/// collides because a function's identity there is its INDEX, not its name; the Rust backend must likewise
/// give each colliding copy a distinct name. Suffixing the def INDEX (`fac_7`) is deterministic and unique,
/// and — read identically at the declaration and the call site — keeps the recursive self-call pointing at
/// its own copy. A def whose name is unique among the emitted set is left un-suffixed (the common case, so
/// ordinary programs are byte-identical).
pub(crate) fn fn_ident(db: &Db, layout: &crate::layout::Layout, def: usize) -> String {
    // CONTENT-ADDRESSED SPEC DEDUP: canonicalize a merged recursive-effectful spec to its representative
    // FIRST. The layout congruence-dedup drops a merged spec from `order` (never emitted) and redirects the
    // wasm func-index via `order_pos`; the rust backend resolves a `Core::Call` callee BY NAME, so without
    // this a call to a merged-away spec names a `fn` that was never declared → rustc E0425 (the exact
    // regression that reverted the dedup). `spec_representative` maps a merged spec to the emitted,
    // structurally-identical representative (identity for any non-merged def / empty-merge program, so this
    // is byte-identical when no dedup fired). Applied to BOTH the declaration site (a representative names
    // itself) and every reference site (a merged callee names its rep), so the two always agree.
    let def = layout.spec_representative(def);
    // The Rust identifier for ANY def is its SANITIZED name (`sum-to` → `sum_to`) — the `-` etc. that
    // Cadenza allows are not Rust ident chars, so a boundary name is still sanitized for the emitted `fn`.
    let base = match layout.export_plan(def) {
        Some(e) => sanitize_ident(&e.name),
        None => sanitize_ident(&db.defs[def].name),
    };
    // An EXPORT is never suffixed: export names are unique, its `pub fn` name is the crate's public entry,
    // and a call to it (from another def) must name it stably. Only a NON-export can collide (a β-copied
    // do-local worker inlined at N sites yields N same-named definitions), so only it disambiguates.
    if layout.export_plan(def).is_some() {
        return base;
    }
    // Does ANY other emitted definition resolve to the same sanitized ident? If so, this non-export must
    // disambiguate against it (whether the other is an export or another β-copy).
    let collides = layout.order.iter().any(|&other| {
        other != def
            && base
                == match layout.export_plan(other) {
                    Some(e) => sanitize_ident(&e.name),
                    None => sanitize_ident(&db.defs[other].name),
                }
    });
    if collides {
        // The def index is a stable per-definition unique key (the wasm backend's function-index identity,
        // surfaced here as a name suffix). Underscore-joined so it stays a valid identifier.
        format!("{base}_{def}")
    } else {
        base
    }
}

#[cfg(test)]
mod tests;
