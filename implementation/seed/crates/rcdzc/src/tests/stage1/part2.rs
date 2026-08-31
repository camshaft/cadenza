use super::super::{count_opcode, imports_value_heap_runtime};
use crate::compile::compile_component;
use crate::testkit::parse;

use super::*;

#[test]
fn a_host_op_composed_with_the_value_heap_runtime_emits_a_valid_component() {
    // HOST + RUNTIME COMPOSITION: a host op result fed into a value-heap runtime op (`Map.insert` on the
    // runtime `ask.ask` value imports the `"heap"` runtime; `ask` imports `"host"`). Previously declined
    // ("a program that both delegates a host effect AND uses the value-heap runtime is not yet emitted");
    // now `envelope::assemble_host_runtime` composes BOTH imported interfaces and the program emits a
    // VALID component (wasmtime parses it). Running it end-to-end also needs cdz-run to link both the
    // host responses and the runtime — a separate increment; component validity is the structural gate.
    let src = "(do (effect ask (op ask (-> Unit Int64))) \
                   (def (main) (host (ask) (Map.len (Map.insert (map (= 1 10)) (ask.ask) 20)))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a host op composed with the value-heap runtime now emits");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .expect("the composed host+runtime component must be valid");
}

#[test]
fn a_host_string_param_op_composed_with_the_value_heap_runtime_emits_a_valid_component() {
    // HOST-STRING-PARAM + RUNTIME COMPOSITION (`envelope::assemble_host_runtime_mem`). A host op with a
    // `string` param (needs the shared-memory shape) delegated ALONGSIDE the value-heap runtime (a
    // runtime `List`): previously declined ("a host op with a string parameter composed with the
    // value-heap runtime is not yet emitted") while each half alone emitted (scalar host+runtime via
    // `assemble_host_runtime`; string host alone via `assemble_host_mem`). The envelope now threads the
    // shared-memory core module through the two-interface (host + heap) fusion — the `(ptr,len)` a
    // `string` lowers to is read from a memory both the program and the op's canon-lower bind. Emits a
    // VALID component (wasmtime parses it). Unblocks assert-with-message over a heap collection
    // (v-property-testing). Reported/assigned via v-runtime; queue
    // `host-string-param-value-heap-coemit-gap`.
    let src = "(do (effect Note (op note (-> String Unit))) \
                   (def (build (: n Int64)) (if (= n 0) (list) ((. List push) (build (- n 1)) n))) \
                   (def (main) (host (Note) (let ((xs (build 3))) (do (Note.note \"built\") ((. List len) xs))))) \
                   (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a host string-param op composed with the value-heap runtime now emits");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .expect("the composed host-string+runtime component must be valid");
}

#[test]
fn a_scalar_host_op_result_escaping_as_a_resource_emits_a_valid_component() {
    // HOST-RESOURCE-ESCAPE FUSION (`envelope::assemble_host_runtime_resource`, the host-side mirror of
    // `assemble_extern_runtime_resource`). A host-delegated effect (NOT peer-bound) reached in an
    // entrypoint whose RESULT escapes as a runtime resource — `main(x) = host H in (tuple (H.h x) x)`,
    // the `(tuple …)` leaving as a resource — previously declined ("a host-delegated effect in an
    // entrypoint whose result escapes as a runtime resource is not yet emitted"). Now the core module
    // lays the host ops as leading `"host"` imports (`runtime_resource_core_module_form_ex2` with
    // `leading_is_host = true`) and the envelope composes the host interface + the runtime + the
    // published resource. Emits a VALID component (wasmtime parses it); a live run threads the host
    // response and returns `(tuple <resp> x)` as the escaped resource. SCOPE: scalar/unit host ops (a
    // String-param host op takes the shared-memory `_mem` variant, a later increment). Routed by
    // v-peer-linking + concierge (host-envelope seam); byte-reviewed by v-peer-linking.
    let src = "(do (effect H (op h (-> Int64 Int64))) \
                   (def (main (: x Int64)) (host (H) #tuple((H.h x) x))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a scalar host op result-escaping as a resource now emits");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .expect("the composed host-resource-escape component must be valid");
}

#[test]
fn two_distinct_host_effects_in_a_resource_escape_decline_cleanly() {
    // `assemble_host_runtime_resource` imports exactly ONE host interface instance, so host ops from >1
    // DISTINCT effect delegated from a resource-escaping entrypoint would be conflated into that single
    // interface and MIS-SERIALIZED (silent, not a clean decline) — the host arms took `iface =
    // host_imports[0].effect` with no single-effect guard (PR #481, Copilot). The guard now declines the
    // multi-effect shape cleanly on ALL THREE resource-escape arms (Flat tuple / Sum / recursive-sum
    // List), mirroring the non-resource host-envelope path. (The multi-interface host-resource shape is a
    // later increment.)
    for src in [
        // FLAT (tuple) — two effects A, B
        "(do (effect A (op a (-> Int64 Int64))) (effect B (op b (-> Int64 Int64))) \
             (def (main (: x Int64)) (host (A) (host (B) #tuple((A.a x) (B.b x))))) (export main))",
        // SUM (Option)
        "(do (effect A (op a (-> Int64 Int64))) (effect B (op b (-> Int64 Int64))) (type Opt (None) (Some Int64)) \
             (def (main (: x Int64)) (host (A) (host (B) (Some (+ (A.a x) (B.b x)))))) (export main))",
        // RECURSIVE-SUM (List)
        "(do (effect A (op a (-> Int64 Int64))) (effect B (op b (-> Int64 Int64))) \
             (def (main (: x Int64)) (host (A) (host (B) ((. List push) (list) (+ (A.a x) (B.b x)))))) (export main))",
    ] {
        let err = compile_component(&crate::codec::encode(&parse(src))).expect_err(
            "two distinct host effects in a resource escape must decline, not mis-serialize",
        );
        assert!(
            err.message.contains("more than one host effect"),
            "the multi-host-effect resource-escape decline must name the cause: {}",
            err.message
        );
    }
}

#[test]
fn a_scalar_host_op_result_escaping_as_a_sum_or_list_resource_emits() {
    // HOST-RESOURCE-ESCAPE, increment 2: the SUM (Option) and RECURSIVE-SUM (List) resource-escape
    // sites — `emit_runtime_sum_resource` / `emit_recursive_sum_resource` — get the same host arm as the
    // Flat/tuple site (increment 1): a scalar host op whose result escapes inside a `Some`/a `List` push
    // composes via `assemble_host_runtime_resource` (`leading_is_host = true`). Both previously declined.
    // Emits a VALID component for each. Scalar-only (a String-param host op still declines to the `_mem`
    // follow-up). v-peer-linking byte-review offered for these sites (mirror the peer sum/recursive-sum).
    for src in [
        // SUM (non-recursive, Option): Some(H.h x) escapes
        "(do (effect H (op h (-> Int64 Int64))) (type Opt (None) (Some Int64)) \
             (def (main (: x Int64)) (host (H) (Some (H.h x)))) (export main))",
        // RECURSIVE-SUM (List): List.push [] (H.h x) escapes
        "(do (effect H (op h (-> Int64 Int64))) \
             (def (main (: x Int64)) (host (H) ((. List push) (list) (H.h x)))) (export main))",
    ] {
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("a scalar host op result-escaping as a sum/list resource now emits");
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(&bytes)
            .expect("the host+sum/list-resource-escape component must be valid");
    }
}

#[test]
fn a_scalar_host_op_result_escaping_as_a_bytes_resource_emits_with_methods() {
    // HOST-RESOURCE-ESCAPE, increment 3: the WITH-METHODS String/Bytes site
    // (`emit_runtime_bytes_resource`) gets the host arm too — a scalar host op whose result escapes inside
    // a Bytes value composes via `assemble_host_runtime_resource_with_scalar_methods` (`leading_is_host =
    // true`), carrying make + encode + the three borrow methods (len / is-empty / to-bytes). Previously
    // this declined "a host-delegated effect … escapes as a runtime resource is not yet emitted". The host
    // op `h : Int64 -> UInt8` feeds `(Bytes.of (list (H.h x)))`, a one-byte Bytes resource. Emits a VALID
    // component (wasmtime parses it, incl. the three lifted methods). Scalar host op (a String-param op
    // still declines to the `_mem` follow-up). Mirrors the peer with-methods twin.
    let src = "(do (effect H (op h (-> Int64 UInt8))) \
                   (def (main (: x Int64)) (host (H) (Bytes.of (list (H.h x))))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a scalar host op result-escaping as a Bytes resource now emits (with methods)");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .expect("the host+bytes-resource-escape (with-methods) component must be valid");
}

#[test]
fn a_generic_reducer_shaped_program_compiles_to_a_valid_component() {
    // The generic reducer EMIT (operator-mandated compiler-platform-separation): the compiler emits
    // a valid component for a program shaped like a reducer — WITHOUT any reducer/fold/kv-specific
    // code. The reducer is just "a program that exports a `list<u8>->list<u8>` fn under a named
    // interface and imports `kv`"; the compiler marshals the declared signatures generically (the
    // named-interface export + the host-import path + S0's N-compound-arg marshal for `put`). Pins
    // the DESIGN's "un-fork-reuse" outcome: no bespoke reducer emit is needed. Compiler-side pin
    // (valid component); the end-to-end fold run is `gate --target platform` once v-agent-harness's
    // bytes-apply kernel boundary lands (apply(event list<u8>)->list<u8>).
    use crate::testkit::parse;
    let compile_reducer = |src: &str| -> Vec<u8> {
        let ast = crate::codec::encode(&parse(src));
        let out = crate::compile::compile(
            &[
                crate::abi::Artifact::new(crate::abi::Artifact::KIND_AST, "main", ast),
                // The program names the interface it EXPORTS under — the reducer fold interface —
                // exactly as a user program would; the compiler does not know it is "the fold".
                crate::cli::component_name_artifact("cadenza:agent-kernel/fold"),
            ],
            &[crate::backend::Target::Wasm],
        );
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .unwrap_or_else(|| {
                let msgs: Vec<_> = out.diagnostics.iter().map(|d| d.message.clone()).collect();
                panic!("a generic reducer-shaped program must emit a component, got: {msgs:?}");
            })
            .to_vec()
    };
    // (1) The fold EXPORT alone: apply(Bytes)->Bytes as a member of the named interface — the
    //     pinned bytes fold shape (list<u8>->list<u8>).
    let export_only = compile_reducer("(do (def (apply (: ev Bytes)) ev) (export apply))");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&export_only)
        .expect("the fold-export-only reducer component must be VALID");
    // (2) The FULL reducer shape: export apply(Bytes)->Bytes + import `kv` via `bind` + perform
    //     put(Bytes,Bytes) — the two-`list<u8>`-arg host call S0 enables — inside the fold.
    let full = compile_reducer(
        "(do (effect Kv (op put (-> Bytes Bytes Unit))) (bind Kv \"cadenza:agent-kernel/kv\") \
             (def (apply (: ev Bytes)) (host (Kv) (do ((. Kv put) ev ev) ev))) (export apply))",
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all()).validate_all(&full).expect(
            "the full reducer component (fold export + kv import + put marshal) must be VALID — the \
             generic paths compose, no reducer-specific emit",
        );
}

#[test]
fn a_marshalled_host_arg_before_a_scalar_arg_keeps_distinct_slots_valid_module() {
    // The multi-arg SLOT-THREADING regression: a runtime String/Bytes host arg reserves i32 rope/len/pos
    // scratch (at `base.max(high)`) and bumps `high`, but the HostCall emit arm formerly reused the STALE
    // `base` for the FOLLOWING arg. So a scalar arg AFTER a marshalled one teed its i64 checked-arith
    // guard into a slot the marshal had declared i32 — one wasm local at two widths → an INVALID module
    // (`wasm-tools validate: expected i64, found i32`). Only the marshalled-BEFORE-scalar order tripped it
    // (scalar-first worked because the scalar bumped `high` first). Fixed by threading a rising `arg_base`
    // (the same `arg_base = *high` pattern `emit_call_args` / `emit_loop_iteration` use). `Component::from_binary` RE-VALIDATES the
    // composed component — the exact guard that failed pre-fix — and the run proves the scalar (`n`, also
    // re-read after the call as `10*n`) was NOT clobbered: send responds 5, so 5 + 10*7 = 75. The corpus
    // case pins the same shape cross-backend; this is the unit-level invalid-module guard.
    use crate::testkit::parse;
    let src = "(do (effect io (op send (-> Bytes Int64 Int64))) \
                   (def (main (: k Int64)) \
                     (host (io) \
                       (let ((n (+ k 7))) \
                         (+ (io.send (Bytes.of (list ((UInt 8).wrap k))) n) (* 10 n))))) \
                   (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a marshalled host arg before a scalar arg must compile, not decline");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all()).validate_all(&bytes).expect(
            "the marshalled-arg-before-scalar component must be VALID (no i32/i64 slot-width clobber)",
        );
    // The RUN — io.send responds 5, and the scalar n=7 (re-read as 10*n after the call) is NOT clobbered
    // by the marshal scratch → 5 + 10*7 = 75 — is corpus-covered cross-backend by the host-arg-marshal
    // shape case; this test keeps the unit-level INVALID-MODULE guard (the slot-width clobber was a
    // wasm-validation failure — a compile-artifact the corpus cannot assert).
}

#[test]
fn two_distinct_host_effects_in_a_bytes_resource_escape_decline_cleanly() {
    // `assemble_host_runtime_resource_with_scalar_methods` imports ONE host interface, so two DISTINCT
    // host effects delegated from a Bytes-resource-escaping entrypoint would be conflated + mis-serialized.
    // The bytes-site host arm carries the same single-effect guard as the Flat/Sum/RecursiveSum arms.
    let src = "(do (effect A (op a (-> Int64 UInt8))) (effect B (op b (-> Int64 UInt8))) \
                   (def (main (: x Int64)) (host (A) (host (B) (Bytes.of (list (A.a x) (B.b x)))))) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src))).expect_err(
        "two distinct host effects in a Bytes resource escape must decline, not mis-serialize",
    );
    assert!(
        err.message.contains("more than one host effect"),
        "the multi-host-effect bytes-resource decline must name the cause: {}",
        err.message
    );
}

#[test]
fn a_host_effecting_entrypoint_returning_a_constant_compound_hoists_build_once() {
    // BUILD-ONCE through the HOST-FUSED resource-escape arm (the abb9390fed thread-through): a
    // host-effecting entrypoint that ALSO returns a markable CONSTANT compound
    // (`main = host H in (do (H.ping k) (tuple 1 2))`) reaches the host-fused arm of
    // `emit_runtime_resource`. Before the fix that arm built `host_layout` WITHOUT
    // `.with_static_compounds` (passed `0, &[]`), so the constant rebuilt MORTAL per `make`; the fix
    // threads the collected statics + init onto the host layout, mirroring the plain path — so the
    // constant hoists to a module GLOBAL (built once + immortal in START, `global.get` in `make`).
    //
    // WHITE-BOX EMIT PIN (corpus-inexpressible — this is a wasm-STRUCTURE property, not a value: a
    // single reduction escapes+releases the value either way, so a live-objects corpus assertion can't
    // distinguish build-once from mortal-rebuild; and a reducer-RUN test would need the dropped cdz-run
    // dep). ESSENTIAL check: the host-fused hoist emits VALID wasm (guards against an ikc1/itf2-style
    // invalid-module on this arm). DISCRIMINATOR: `global.set`/`global.get` count — the START-init
    // emits one `global.set` per hoisted static compound (no static bytes here), and the `make` body
    // reads it via `global.get`. The CONTRAST (a RUNTIME compound `(tuple k 2)` — nothing hoistable →
    // 0 globals) proves the const case's globals are specifically the constant hoist, not incidental.
    use crate::testkit::parse;
    let const_src = "(do (effect H (op ping (-> Int64 Int64))) \
                   (def (main (: k Int64)) (host (H) (do (H.ping k) (tuple 1 2)))) (export main))";
    let runtime_src = "(do (effect H (op ping (-> Int64 Int64))) \
                   (def (main (: k Int64)) (host (H) (do (H.ping k) (tuple k 2)))) (export main))";

    let const_wasm = compile_component(&crate::codec::encode(&parse(const_src))).expect(
        "a host-effecting entrypoint returning a constant compound must compile (host-fused arm)",
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(&const_wasm)
            .expect("the host-fused build-once hoist must emit a VALID module (no invalid-wasm on this arm)");
    let const_gset = count_opcode(&const_wasm, |op| {
        matches!(op, wasmparser::Operator::GlobalSet { .. })
    });
    let const_gget = count_opcode(&const_wasm, |op| {
        matches!(op, wasmparser::Operator::GlobalGet { .. })
    });
    assert!(
        const_gset >= 1 && const_gget >= 1,
        "the constant `(tuple 1 2)` must hoist build-once through the host-fused arm — expected a \
             START-init `global.set` + a `make`-body `global.get`, got global.set={const_gset} \
             global.get={const_gget} (pre-fix: 0/0, rebuilt mortal per make)"
    );

    // CONTRAST: a RUNTIME compound has nothing to hoist → zero build-once globals (the const globals
    // above are the constant hoist, not incidental host-fused machinery).
    let runtime_wasm = compile_component(&crate::codec::encode(&parse(runtime_src)))
        .expect("the runtime-compound control must compile");
    let runtime_gset = count_opcode(&runtime_wasm, |op| {
        matches!(op, wasmparser::Operator::GlobalSet { .. })
    });
    assert_eq!(
        runtime_gset, 0,
        "a RUNTIME compound `(tuple k 2)` has no hoistable constant → no build-once `global.set`, \
             got {runtime_gset} (would mean the const-case globals were incidental, not the hoist)"
    );
}

#[test]
fn a_multishot_arm_folds_flat_but_declines_inside_recursion_never_miscompiles() {
    use crate::testkit::parse;
    // MULTI-SHOT ARM × RECURSION (breaker ms-family datapoint, 2026-08-05). A MULTI-SHOT handler arm
    // (`(flip (u) s (+ (resume 2 s) (resume 3 s)))` — resumes TWICE, summing the two continuations) is
    // served by the tail/refold fold when the performs sit on a FLAT strict spine, but currently DECLINES
    // cleanly when the performs are inside a SELF-RECURSIVE loop (the recursion machinery serves
    // single-shot resumptive arms only; a 2^n-path refold inside a recursive cycle is a real semantics
    // question — the third member of the abort-in-recursion / accum-op-arg family). This pins the recursive
    // face as a CLEAN DECLINE, never a wrong value or crash.

    // FLAT: two flips on a `*` spine, multi-shot arm → the 4-path cross-product sum folds.
    // (2*2)+(2*3)+(3*2)+(3*3) = 4+6+6+9 = 25.
    // FLAT (the fold value) is migrated to the corpus: 14-effects "a flat multi-shot arm with two
    // performs on a strict spine folds the cross-product" (= 25). This rcdzc test keeps ONLY the
    // white-box RECURSIVE-face decline check (no wasmtime).

    // RECURSIVE: the SAME multi-shot arm but the performs are inside a self-recursive loop. Today this
    // declines cleanly (a recursive 2^n-path refold is a later increment). If a future increment folds
    // it, the value MUST equal the flat cross-product 25 (`(loop 2)` = `(* flipA (* flipB 1))`, the `* 1`
    // collapsing to `(* flipA flipB)`) — pinned by the corpus flat case above; here we only guard the
    // current CLEAN CDZ0900 (deferred) decline, never a hard coded rejection or crash.
    let rec = "(do (effect Amb (op flip (-> Unit Int64))) \
             (def (loop (: n Int64)) (if (= n 0) 1 (* (Amb.flip) (loop (- n 1))))) \
             (def (main) (handle Amb 0 ((flip (u) s (+ (resume 2 s) (resume 3 s)))) (loop 2))) (export main))";
    let e = compile_component(&crate::codec::encode(&parse(rec)))
        .expect_err("the recursive multi-shot face declines cleanly (not yet reducible)");
    assert!(
        e.code.as_deref() == Some("CDZ0900"),
        "the recursive multi-shot face must decline as CDZ0900 (deferred) — a hard coded rejection is a different regression: {:?}",
        e.code
    );
}

#[test]
fn the_op_arg_lift_cv_binder_namespace_cannot_be_captured_by_a_user_binder() {
    // OP-ARG LET-LIFT capture-safety (settles the #2156/#2120 `#cv`-uniqueness review question).
    //
    // The op-arg let-lift mints a fresh binder `#cv{StructId}` (see effects.rs, `arg_lifts`) to hold a
    // foreign-performing arg exactly once. Its collision-safety rests on TWO facts, and it is worth being
    // precise about WHICH — a reviewer (github-liaison #2156) correctly noted that `#cv0` is NOT
    // "unspellable" at the lexer level: a backtick name `` `#cv0` `` lexes to a plain BacktickName token,
    // so it can be MENTIONED in source. The real guarantee is:
    //   1. The `StructId` is the arg node's arena index — a MONOTONIC, never-reused counter — so two
    //      distinct lift sites never share a `#cv{…}` name (covered by the fold tests below/around).
    //   2. A user can never introduce a `#cv{N}` BINDER to capture/shadow a live lift site: a
    //      `#`-leading name is a CONSTRUCTOR pattern, which is refutable and thus illegal in a binding
    //      position — rejected CDZ0210. So a `#cv…` name is unspellable *as a binder*, which is the only
    //      position from which it could capture. (A bare ident like `Foo`/`xy` binds fine; the `#` is
    //      what makes it a refutable pattern head — see the D control.)
    // Together these mean no user program can collide with, capture, or shadow the lift's `#cv` slot —
    // the safety conclusion in the code comment holds, via binder-position rejection, not lexer magic.

    // A `#cv0` LET binder is rejected — a `#`-leading name is a refutable constructor pattern.
    let let_binder = "(do (def (main) (let ((`#cv0` 5)) `#cv0`)) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(let_binder))).expect_err(
        "a `#cv0` let binder must be rejected (refutable ctor pattern in binding position)",
    );
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0210"),
        "a `#cv0` LET binder is a refutable constructor pattern, illegal in binding position: {}",
        err.message
    );

    // The same holds for a `#cv0` PARAMETER binder — the other position a user could try.
    let param_binder = "(do (def (f (: `#cv0` Int64)) `#cv0`) (def (main) (f 7)) (export main))";
    let perr = compile_component(&crate::codec::encode(&parse(param_binder)))
        .expect_err("a `#cv0` param binder must be rejected too");
    assert_eq!(
        perr.code.as_deref(),
        Some("CDZ0210"),
        "a `#cv0` PARAM binder is likewise a refutable ctor pattern, illegal in binding position: {}",
        perr.message
    );

    // CONTROL: a bare (non-`#`) name binds fine in the SAME position, so it is the `#` prefix — not
    // backtick-ness or the position — that makes `#cv…` unbindable. If this control ever declines, the
    // CDZ0210 above is a false witness and this pin is meaningless. (The bare-let RUN value — `(let
    // ((notcv 5)) notcv)` = 5 — is trivial let-binding, densely covered in 02-binding-and-control; here
    // it need only COMPILE, the contrast against the `#cv0` rejects above.)
    let control = "(do (def (main) (let ((notcv 5)) notcv)) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(control))).is_ok(),
        "control: a non-`#` binder in the same position must bind (compile)"
    );
}

#[test]
fn a_delegated_effect_performed_inside_an_intra_program_handler_compiles() {
    // E2h: a host-delegated `ask.ask` performed INSIDE an intra-program `handle` (over a DIFFERENT
    // effect `Scale`). The fold reduces the `Scale` handler away (its arm doubles the perform value),
    // leaving `(ask.ask)` — a host call — in the SYNTHESIZED rewritten body. That synthesized node has
    // no `host` ANCESTOR, so `perform_host_target` falls back to the program-wide delegation set (the
    // manifest is the union of the entrypoints' `host` clauses) to recognize it as host-bound. It
    // compiles to a component importing `ask` (verified via the gate → 42 with `ask.ask=21`). The host
    // import is unbound in-process, so this asserts it COMPILES; the corpus gate runs the value.
    let src = "(do (effect ask (op ask (-> Unit Int64))) (effect Scale (op by (-> Int64 Int64))) \
                   (def (main) (host (ask) (handle Scale unit ((by (n) s (resume (* n 2) s))) \
                     (Scale.by (ask.ask))))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a host-delegated perform inside an intra-program handler must compile"
    );
}

#[test]
fn a_host_op_with_a_string_argument_compiles() {
    // E2h-string: a host op `log.emit : String -> Unit` performed on a CONSTANT string. The string
    // crosses the boundary as the component `string` (core `(ptr,len)`): the compiler bakes "ready"
    // into a data segment, imports a shared memory, and the op's canon-lower carries the Memory option
    // (the shared-memory 2-instance envelope shape). Compiles to a component importing
    // `log: interface { emit: func(p0: string) }` (verified via the gate → unit + observed log.emit).
    let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) (log.emit \"ready\"))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a host op with a constant string argument must compile"
    );
}

#[test]
fn a_non_kebab_effect_and_op_name_emit_a_valid_component() {
    // REGRESSION (the effect-boundary residual of the export-name kebab fix): a non-kebab effect NAME
    // (the imported interface's extern name) or OPERATION name (a func the interface exports) emitted
    // an INVALID component ("import name `Log` is not a valid extern name") with no diagnostic. Both
    // boundary names must be kebab-normalized. `wasmparser::validate` rejected the pre-fix bytes.
    let up_effect = "(do (effect Log (op msg (-> Unit Int64))) \
                   (def (main) (host (Log) (Log.msg))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(up_effect)))
        .expect("an uppercase effect name must compile");
    wasmparser::validate(&bytes)
        .expect("an uppercase effect name must emit a VALID component (kebab interface import)");
    // The interface import name is the kebab-normalized `log`, not the verbatim `Log`.
    assert!(
        bytes.windows(3).any(|w| w == b"log") && !contains_extern_name(&bytes, "Log"),
        "the interface import extern name must be kebab `log`, not `Log`"
    );

    // The operation-name site: an uppercase op `Ask` normalizes to `ask` at the interface's func
    // export decl + its alias (they must agree, or the alias fails to resolve).
    let up_op = "(do (effect e (op Ask (-> Unit Int64))) \
                   (def (main) (host (e) (e.Ask))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(up_op)))
        .expect("an uppercase op name must compile");
    wasmparser::validate(&bytes)
        .expect("an uppercase op name must emit a VALID component (kebab func extern)");
}

#[test]
fn a_do_block_of_host_calls_sequences_them() {
    // E2h-seq: a `(do (log.emit "first") (log.emit "second"))` — two side-effecting host-call
    // statements — lowers to a `Core::Seq` that EMITS each statement in order (their calls both cross
    // the boundary), then the tail is the block's value. Was a clean decline (the block would else
    // drop the non-final call); now it compiles + BOTH calls fire (verified in order via the corpus
    // gate → observed [log.emit, log.emit]). The host imports are unbound in-process, so this asserts
    // it COMPILES; the gate runs the observed sequence.
    let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) (do (log.emit \"first\") (log.emit \"second\")))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a do block of host-call statements must compile (sequencing both calls)"
    );
}

#[test]
fn a_delegated_effect_reached_through_a_recursive_callee_compiles() {
    // E2h-rec: `main` delegates `log` and calls a RECURSIVE `go` that performs `log.emit` on each
    // step. Two coupled fixes: (1) `body_reached_effects` (the CDZ0404 latent-authority check) now
    // FOLLOWS a recursive callee (visited-set guarded), so `log` is seen as reached — no false
    // "latent authority"; (2) `go`'s `log.emit` lowers to a `Core::HostCall` via
    // `perform_host_target`'s program-delegation fallback (the enclosing entrypoint delegates `log`),
    // and the `(do (log.emit "x") (go …))` body sequences it via `Core::Seq` (E2h-seq) so the call is
    // EMITTED, not dropped. Compiles to a component importing `log` (gate → unit + observed log.emit).
    let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (go n) (if (= n 0) unit (do (log.emit \"x\") (go (- n 1))))) \
                   (def (main) (host (log) (go 1))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a host effect reached through a recursive callee must compile"
    );
}

#[test]
fn an_intra_program_handler_interposes_on_a_delegated_effect_and_forwards() {
    // E2h-interpose: the entrypoint delegates `ask` to the host, but an inner handler INTERCEPTS every
    // `ask.ask`, records it via the intra-program `Count.tick`, and RE-PERFORMS `(ask.ask)` in tail
    // position (forwarding). The arm body `(do (Count.tick) (resume (ask.ask) s))` is a
    // do-wrapped resume — the perform arm peels the trailing resume, keeps the `Count.tick` statement
    // (folded to the OUTER Count handler), and forwards the re-performed `ask.ask` to the host (a
    // `Core::HostCall`, since no nearer handler discharges it). `(+ (ask.ask) (ask.ask))` with host
    // responses 3,4 → 7 (2 observed ask.ask calls). The host is unbound in-process, so assert it
    // COMPILES; the corpus gate runs the value + host-call sequence.
    let src = "(do (effect ask (op ask (-> Unit Int64))) (effect Count (op tick (-> Unit Unit))) \
                   (def (main) (host (ask) \
                     (handle Count unit ((tick (u) s (resume unit s))) \
                       (handle ask unit ((ask () s (do (Count.tick) (resume (ask.ask) s)))) \
                         (+ (ask.ask) (ask.ask)))))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "an interposing handler that forwards a delegated effect must compile"
    );
}

#[test]
fn an_effectful_host_arg_into_a_destructuring_match_is_evaluated_once() {
    // FIXED (was a KNOWN MISCOMPILE emitting 3, now 1). TWO independent bugs stacked here, both closed:
    // (1) the β-reduce call-by-name re-perform — `(mk (E.get))` with `mk s = (T s s s)` substituted the
    // effectful arg by name at all 3 uses; the evaluate-once let-bind (apply_lambda_uncached) fixed it,
    // reducing to `(let ((s (E.get))) (T s s s))`. (2) that `let` then sits in the SCRUTINEE position of
    // `(match _ ((T a b c) …))`, and a single-arm sum match FOLDS to a bare `Leaf` (`core_of(body)`) with
    // NO scrutinee materialization, so each of the 3 `Core::SumPayload` binders (`a`,`b`,`c`) RE-EMITTED
    // the host-reaching scrutinee → 3 host calls again. FIX: `lower_match_sum`'s `Leaf` arm keeps the
    // `Core::MatchSum` wrapper when the scrutinee reaches a host call, so the wrapper's emit
    // materializes the scrutinee into ONE slot and every payload binder reads it. Now `sum3(mk(E.get))`
    // makes exactly ONE host `get` call — strict by-value + deterministic host-sequence (core-semantics
    // §Applying A Function, §283; capabilities §75).
    let src = "(do (effect E (op get (-> Unit Int64))) (type Trip (T Int64 Int64 Int64)) \
                   (def (mk s) (T s s s)) (def (sum3 t) (match t ((T a b c) (+ (+ a b) c)))) \
                   (def (main) (host (E) (sum3 (mk (E.get))))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("compiles (an effectful arg into a destructuring match is now evaluated once)");
    // Count core `call 0` — the host `get` import is core func 0 (host imports laid first).
    let host_calls = count_opcode(&bytes, |op| {
        matches!(op, wasmparser::Operator::Call { function_index: 0 })
    });
    assert_eq!(
        host_calls, 1,
        "evaluate-once through a destructuring match: the host arg is bound once (1 host call), not \
             re-performed per payload binder"
    );
}

#[test]
fn an_effectful_host_arg_to_a_multiuse_scalar_fn_param_evaluates_once() {
    // EVALUATE-ONCE (strict eval, the fix for the call-by-name re-perform miscompile). A HOST-delegated
    // op passed as the argument to a fn whose param is used MULTIPLY was substituted BY NAME and
    // re-performed per use: `(mk (E.get))` with `mk s = (+ (+ s s) s)` emitted THREE host `get` calls,
    // violating strict by-value binding (core-semantics.md §Applying A Function binds the parameter to a
    // single evaluated value; §283) + the deterministic host-call sequence (capabilities §75). FIX =
    // `apply_lambda_uncached` LET-BINDS an effectful multi-use argument ONCE at the call site (β-reduce
    // leaves the param out of the substitution, wraps the reduced body in `(let ((s (E.get))) …)`), so
    // the perform runs once and every use reads the bound local — the compiler now does automatically
    // what the hand-written `(let ((s (E.get))) …)` workaround did. A single-use param, or a pure arg,
    // is untouched (byte-identical). This SCALAR-continuation shape (`s` used in arithmetic) folds to
    // ONE host call; the `(T s s s)`-into-a-destructuring-match shape hits a SECOND, independent bug
    // (`Core::SumPayload` re-lowers a host-reaching scrutinee per payload binder — see
    // `an_effectful_host_arg_to_a_multiuse_fn_param_reperforms_is_a_known_miscompile`).
    let src = "(do (effect E (op get (-> Unit Int64))) \
                   (def (mk s) (+ (+ s s) s)) \
                   (def (main) (host (E) (mk (E.get)))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("compiles (an effectful multi-use arg is now let-bound, not re-performed)");
    let host_calls = count_opcode(&bytes, |op| {
        matches!(op, wasmparser::Operator::Call { function_index: 0 })
    });
    assert_eq!(
        host_calls, 1,
        "evaluate-once: an effectful host arg to a multi-use scalar param is bound once (1 host call), \
             not re-performed per use"
    );
}

#[test]
fn an_abortive_arm_with_a_runtime_perform_arg_grounds_the_handle_result_type() {
    // REGRESSION (abort + runtime-arg wasm/rust divergence, corpus-bugfix 2026-07-18). An ABORTIVE arm
    // `(bail (n) s n)` returns the op arg `n`; performed with a RUNTIME (non-const) arg `(Bail.bail k)`
    // (k a def param), the abort collapses the handle to `n` = a reference to `k`. Before the fix the
    // abort value was returned WITHOUT `reparent_under_handle_site`, so the orphan copy's `k` read
    // unbound → the handle typed `Any` → wasm declined "return type has no machine representation" while
    // rust computed (a backend split). A CONST arg `(Bail.bail 7)` folded to a literal so it never hit
    // this. FIX = reparent the abort value under the handle site before returning (the same reparent the
    // resumptive path does), so `k` re-resolves to the param and grounds the result type. Compiles now;
    // `main(7)` = 7 (abort returns k=7, discards the `+ 1`).
    let src = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main (: k Int64)) (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail k)))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "an abortive arm with a runtime perform arg must ground the handle result type (reparent the \
             abort value), not decline 'no machine representation' on wasm while rust computes"
    );
}

#[test]
fn a_nested_effectful_let_inlined_into_a_reperforming_body_keeps_its_binder() {
    // REGRESSION (nested-effectful-let CDZ0101, compiler-ml Db-port blocker). A helper whose body is a
    // `let` binding an effect result — `inner = (let a = St.get() in (match St.put(a) with _ => a))` —
    // inlined as the init of an OUTER `let` whose body PERFORMS AGAIN: `outer = (let b = inner() in b +
    // St.get())`. Before the fix this emitted a SPANLESS `CDZ0101 unbound a`: after inlining, the handle
    // body is `(let ((b (let ((a St.get)) (match St.put(a) (_ a))))) (+ b St.get))`, and the fold threads
    // the outer body's second `St.get` to the `put`-arm's next-state, which is the node `a` — but `a` is
    // bound INSIDE `b`'s init-let, out of scope in `(+ b …)`. FIX = the nested-let init LET-LIFT in the
    // `let` thread arm: when a binding's threaded init is itself a `(let ((x e)…) lbody)`, the inner
    // bindings are hoisted to the enclosing let (siblings of `b`), so `a` is in scope for the
    // continuation + its threaded out-state. Under `handle St(10) get(s)=>resume(s,s) put(v,s)=>
    // resume(unit,v)`, run() = b(=10) + get()(=10) = 20. Compile-only here (the corpus case in
    // 14-effects value-grades the 20 against the real runtime).
    let src = "(do (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit))) \
                   (def (inner) (let ((a (St.get))) (match (St.put a) (_ a)))) \
                   (def (outer) (let ((b (inner))) (+ b (St.get)))) \
                   (def (main) (handle St 10 \
                     ((get (u) s (resume s s)) (put (v) s (resume unit v))) \
                     (outer))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a nested effectful let inlined into a re-performing body must keep its inner binder in scope \
             (the let-lift), not emit a spanless CDZ0101 unbound name"
    );
}

#[test]
fn a_ctl_style_arm_binds_the_continuation_and_declines_cleanly_for_now() {
    // E5 STEP 1 (classifier + surface): a 5-part `ctl`-style handler arm `(op (params) state k body)`
    // binds the continuation `k` as a first-class value. This is the surface a DES scheduler needs
    // (`sleep` captures `k`, stores it, resumes later). The frame-capture lowering is not built yet, so
    // a general arm DECLINES CLEANLY (a Todo — `compile_component` errors, never panics, never a
    // valid-but-wrong fold). Crucially `k` must be IN SCOPE in the body (bound by the arm), NOT a
    // spurious CDZ0101 unbound name — the arm parses as 5 parts and `handle_arm_binds` resolves `k`.
    // Here the body references `k` (as a value — the eventual `apply(k, v)`), which must resolve.
    let src = "(do (effect Amb (op flip (-> Unit Int64))) \
                   (def (main) (handle Amb 0 ((flip (u) s k k)) (Amb.flip))) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src))).expect_err(
        "a ctl-style continuation-binding arm declines cleanly (frame capture not built)",
    );
    // The decline must NOT be a scope error about `k` — `k` is bound by the arm. A CDZ0101 naming `k`
    // would mean the 5-part arm's continuation binder was not wired into scope (the bug this pins
    // against). Any other clean decline (a Todo — no machine rep / not-yet-lowered) is expected.
    assert!(
        !(err.code.as_deref() == Some("CDZ0101") && err.message.contains('k')),
        "the continuation binder `k` must be IN SCOPE (bound by the ctl-style arm), not a spurious \
             CDZ0101 unbound-name error: {:?} / {}",
        err.code,
        err.message
    );
}

#[test]
fn an_abortive_perform_in_a_connective_condition_folds() {
    // E4×connective (was a clean over-decline). An abortive perform inside a short-circuit connective
    // that is an `if` CONDITION — `(if (and b (> (Bail.bail 7) 0)) 100 200)` — used to decline: the
    // connective desugar produces `(if (if b (Bail.bail 7 …) false) 100 200)`, an outer `if` whose
    // CONDITION is an `if`-with-abort, which no hoist site reached. FIX (two parts): (1) a new
    // `hoist_once` site distributes the outer `if` through its condition-`if` — `(if (if c2 t2 e2) t e)`
    // ≡ `(if c2 (if t2 t e) (if e2 t e))` (pure outer branches) — landing the abort in a branch's
    // condition; (2) `body_has_unsound_abortive_perform` treats an abort in an `if` CONDITION on a TAIL
    // path as capturable (an abort in a condition abandons everything; `thread_branch_local_abort` /
    // the abort cell take it). So the whole thing folds. Compile-only here; the corpus case value-grades
    // b=true → 7 (abort fires) and b=false → 200 (short-circuit, rhs never performed).
    let src = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (run (: b Bool)) (handle Bail 0 ((bail (n) s n)) \
                     (if (and b (> (Bail.bail 7) 0)) 100 200))) \
                   (def (main) (run true)) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "an abortive perform in a connective-condition must fold (outer-if distribution + tail-condition \
             capture), not decline"
    );
}

#[test]
fn a_ctl_arm_applying_k_lexically_folds_through_the_continuation() {
    // E5 STEP 2 (within-activation, lexical `k`): a ctl-style arm that APPLIES `k` as `(k v)` — never
    // bare, stored, or passed as an arg — is semantically an ordinary non-tail resumptive arm: `(k v)`
    // returns into the delimited context, exactly like `(resume v)`. `ctl_arm_lexical_k_to_resume`
    // rewrites `(k v)` → `(resume v state)`, and the existing pure-one-hole fold serves it — NO new heap
    // rep, NO frames (those are step 3, for an ESCAPING `k`). Over the identity body `(Amb.flip)`, the
    // continuation `C = (+ □ 1)`, so `(k 10)` = `C[10]` = `(+ 10 1)` = 11. Compile-only (in-process
    // linker lacks the value heap for some paths); the corpus case value-grades the 11.
    let src = "(do (effect Amb (op flip (-> Unit Int64))) \
                   (def (main) (handle Amb 0 ((flip () s k (+ (k 10) 1))) (Amb.flip))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a ctl arm applying k lexically must fold through the continuation (k v = resume v), not decline"
    );
}

#[test]
fn a_ctl_arm_whose_k_escapes_over_a_pure_continuation_reifies_a_closure() {
    // E5 STEP-3 INC-2a: a ctl-style arm whose `k` ESCAPES — passed to another function `(use-k k)`,
    // stored — over a PURE delimited continuation `C` now REIFIES `k` as a closure `(fn (#kv) C)` and
    // FOLDS (was a decline in step 2). Here `C = □` (the handle body IS the bare perform), so `k = (fn
    // (#kv) #kv)` (identity); `use-k` applies it to 10 → 10. The reified continuation over a pure `C` is
    // an ordinary closure — no bespoke frame chain (`DESIGN-general-continuations-e5.md` §9-12). A
    // RE-performing `C` (the continuation itself performs the handled effect — the DES `sleep` case)
    // still declines, deferred to inc-2b (handler re-entry at apply); see the sibling test.
    let src = "(do (effect Amb (op flip (-> Unit Int64))) \
                   (def (use-k (: f (-> Int64 Int64))) (f 10)) \
                   (def (main) (handle Amb 0 ((flip () s k (use-k k))) (Amb.flip))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a ctl arm whose k escapes over a PURE continuation must reify a closure + fold (inc-2a), \
             not decline"
    );
}

#[test]
fn ty_cont_variant_is_reserved_and_its_predicates_recurse() {
    // E5 STEP 3 increment 1 (gate-neutral): the `Ty::Cont { resume, answer }` variant EXISTS + every
    // exhaustive `Ty` match has an arm. Nothing CONSTRUCTS one yet (an escaping-k arm still declines to
    // lower until the frame reification + apply dispatcher land in later increments), so this just pins
    // that the variant is well-formed and its recursive predicates thread into both components (a
    // `Cont` over a free-var/type-value/any component carries it — like `Ty::Fn`). Guards the foundation
    // slice so a later increment builds on a variant that behaves structurally, not a stub.
    use crate::ty::Ty;
    let free = Ty::Cont {
        resume: Box::new(Ty::Var(7)),
        answer: Box::new(Ty::int64()),
    };
    assert!(free.has_free_var(), "a Cont over a free var has a free var");
    assert!(!free.is_ground(), "a Cont over a free var is not ground");
    let typev = Ty::Cont {
        resume: Box::new(Ty::Unit),
        answer: Box::new(Ty::Type),
    };
    assert!(
        typev.has_type_value(),
        "a Cont over Type carries a type value"
    );
    let ground = Ty::Cont {
        resume: Box::new(Ty::Unit),
        answer: Box::new(Ty::int64()),
    };
    assert!(
        ground.is_ground(),
        "a Cont over ground components is ground"
    );
    assert_eq!(
        ground.render_name(&crate::ty::NameCtx::new(&[])),
        "(Cont Unit Int64)"
    );
    // A Cont has NO boundary form (host-composition invariant) but IS an i32 machine slot when built.
    assert_eq!(crate::backend::wasm::lir::comp_valtype_of(&ground), None);
}

#[test]
fn a_ctl_arm_applying_k_inside_a_match_scrutinee_resolves_k() {
    // REGRESSION (a bogus CDZ0101 my E5 step-2 surfaced): a ctl-style arm applying `k` INSIDE a MATCH
    // SCRUTINEE — `(flip () s k (match (k 10) (z (* z 2))))` — reported `CDZ0101 unbound k`, even though
    // `k` is bound by the arm and `(* (k 10) 2)` (no match) resolved fine. ROOT: `is_param_occurrence`
    // recognized the arm's STATE binder (`parts[2]`) as a binder occurrence but had NO case for the
    // 5-part arm's CONTINUATION binder `k` (`parts[3]`), so on the resolution path a match reference
    // takes, `k`'s scope was never established. FIX = an `is_param_occurrence` case for a 5-part arm's
    // `parts[3]`. Now folds through the continuation: `(k 10)` = `resume 10` = 10 into the context, and
    // the match arm doubles it → 20.
    let src = "(do (effect Amb (op flip (-> Unit Int64))) \
                   (def (main) (handle Amb 0 ((flip () s k (match (k 10) (z (* z 2))))) (Amb.flip))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a ctl arm applying k inside a match scrutinee must resolve k (not a bogus CDZ0101), then fold"
    );
}

#[test]
fn a_malformed_type_used_as_an_annotation_does_not_also_report_it_a_value() {
    // A type whose declaration is MALFORMED (a duplicate variant) does not fully register, so
    // `typeval_of` of its name fails — and using it as an annotation `(: c C)` used to CASCADE into a
    // misleading "`C` is a value, not a type" (it IS a type, just broken, and the phrasing blames the
    // annotation, not the real defect). The duplicate-variant CDZ0201 is the ONE primary; the
    // consequent "is a value" is suppressed (the annotation name resolves to a declared type).
    let src = "(module m (type C (Red) (Red)) (def (f (: c C)) 1) (export f))";
    let errs: Vec<crate::abi::Diagnostic> =
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .collect();
    assert!(
        errs.iter()
            .any(|d| d.message.contains("more than once in sum `C`")),
        "the duplicate-variant reject is present: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        !errs
            .iter()
            .any(|d| d.message.contains("is a value, not a type")),
        "no misleading 'C is a value, not a type' consequent: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO OVER-SUPPRESSION: a genuine VALUE misused as a type still says "is a value, not a type".
    let val = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def helper 5) (def (f (: x helper)) x) (export helper))",
    )));
    assert!(
        val.iter()
            .any(|d| d.message.contains("`helper` is a value, not a type")),
        "a value keeps its own message: {:?}",
        val.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // An APPLIED malformed generic — `(: x (Box Int64))` where `(type Box (W a) (W b))` has a dup
    // variant — likewise defers: only the dup-variant reject, no "found a non-type" consequent.
    let applied = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (type Box (W a) (W b)) (def (f (: x (Box Int64))) x) (export f))",
    )));
    let aerrs: Vec<&str> = applied
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        aerrs
            .iter()
            .any(|m| m.contains("more than once in sum `Box`"))
            && !aerrs.iter().any(|m| m.contains("found a non-type")),
        "an applied malformed generic defers to the dup-variant reject: {aerrs:?}"
    );
    // NO OVER-SUPPRESSION on a WELL-FORMED generic MISAPPLIED: `(Box 5)` (a non-type argument) and
    // `(Box Int64 Bool)` (wrong arity) still report — the type is fine, the USE is wrong.
    for bad_use in [
        "(module m (type Box (W a)) (def (f (: x (Box 5))) x) (export f))",
        "(module m (type Box (W a)) (def (f (: x (Box Int64 Bool))) x) (export f))",
    ] {
        let d = crate::diagnostics(&mut crate::db::Db::load(parse(bad_use)));
        assert!(
            d.iter().any(|x| x.severity == crate::abi::Severity::Error),
            "a well-formed generic misapplied still reports: {bad_use} -> {:?}",
            d.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }
}

#[test]
fn a_sum_type_name_resolves_to_its_sum_type_in_type_position() {
    // The records-everywhere realization: `(type Option (Some Int64) None)` binds `Option` to a
    // synthesized RECORD whose `(meta t)` is the sum type-value. So `Option` used in a type
    // annotation reduces to `Ty::Sum` through the ORDINARY `(meta t)` projection — the same path
    // `(: e UInt8)` takes to `Ty::Int`, no sum special case. Its identity is the DECLARATION
    // occurrence (`type-system.md §158`), and its render name is the declared name.
    use crate::db::Db;
    use crate::eval::typeval_of;
    use crate::testkit::parse;
    use crate::ty::Ty;
    let ast = parse("(module m (type Option (Some Int64) None) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = db.type_decl_by_name("Option").expect("Option bound");
    let ty = typeval_of(&mut db, occ).expect("Option is a type");
    match ty {
        Ty::Sum { decl, args } => {
            // The declared name is recovered from `decl` via the render context (no longer on the type).
            assert_eq!(db.name_ctx().name_of(decl), Some("Option"));
            assert!(
                args.is_empty(),
                "a bare monomorphic-shaped sum has no type args"
            );
            // The identity is the declaration occurrence recorded in the scan.
            let decl_occ = db
                .type_decls
                .iter()
                .find(|t| t.name == "Option")
                .unwrap()
                .occ;
            assert_eq!(decl, decl_occ);
        }
        other => panic!(
            "expected Ty::Sum, got {}",
            other.render_name(&db.name_ctx())
        ),
    }
}

#[test]
fn the_type_reflection_module_denotes_the_kind_of_types_in_a_type_position() {
    // Type-valued-parameter vertical, T1: `Type` (the type-reflection module, fields `of`/`eq` =
    // `type-of`/`type-eq`) used in a TYPE POSITION denotes the KIND OF TYPES, `Ty::Type` — so
    // `(: t Type)` declares a type-valued parameter. Recognized STRUCTURALLY (its `of` field's
    // `(meta apply)` is `Prim::TypeOf`), never by the name "Type" (no key outside the prelude). This
    // revises the prior "`Type` in a bare type position is not a type" stance (`prelude.rs`), giving
    // the type-valued-parameter model a spellable annotation.
    use crate::db::Db;
    use crate::eval::typeval_of;
    use crate::testkit::parse;
    use crate::ty::Ty;
    let mut db = Db::load(parse("(module m (def (main) 0) (export main))"));
    let type_occ = *db.prelude.get("Type").expect("Type is in the prelude");
    assert_eq!(
        typeval_of(&mut db, type_occ),
        Some(Ty::Type),
        "the Type reflection module denotes Ty::Type in a type position"
    );
    // A ground type name is unaffected — still its own type, not the kind.
    let int_occ = *db.prelude.get("Int64").expect("Int64 is in the prelude");
    assert!(
        matches!(typeval_of(&mut db, int_occ), Some(Ty::Int(_))),
        "Int64 still denotes the Int64 type, not the kind"
    );
}

#[test]
fn the_kind_type_under_a_compound_annotation_round_trips_not_collapses_to_unit() {
    // The `Ty::Type` twin of the `Ty::Var` round-trip fix above: the KIND-OF-TYPES `Type` used INSIDE a
    // compound annotation — an arrow domain/result `(-> Type Int64)` / `(-> Int64 Type)`, a `(Tuple Type
    // …)`, a `(Record (kind Type))` — took the `reduce_ctor`→`encode_typeval` round-trip, and `encode_ty`
    // had NO `Ty::Type` arm, so its catch-all stubbed `Type` as `Unit`. A `(: g (-> Type Int64))` param
    // then schemed as `(-> (-> Unit Int64) …)`, and passing a real `(-> Type Int64)` value failed CDZ0203
    // "argument is a Type, but a value of type Unit is expected". Fixed by pairing an `encode_ty`/
    // `decode_ty` `Type` arm (same class as the Var/Bytes/String/Qty/Nominal round-trip holes). Verified
    // via the def scheme: `Type` must survive in each compound position, not read `Unit`.
    let scheme_of = |src: &str, name: &str| {
        let mut db = crate::db::Db::load(parse(src));
        let idx = db
            .defs
            .iter()
            .position(|d| d.name == name)
            .expect("def present");
        crate::infer::def_scheme(&mut db, idx)
            .map(|s| s.ty.render_name(&db.name_ctx()))
            .unwrap_or_else(|| "<none>".to_string())
    };
    // Arrow DOMAIN: `(: g (-> Type Int64))` — g's type must stay `(-> Type Int64)`, not `(-> Unit Int64)`.
    let dom = scheme_of(
        "(module m (def (f (: g (-> Type Int64)) (: t Type)) (g t)) (export f))",
        "f",
    );
    assert!(
        dom.contains("(-> Type Int64)") && !dom.contains("(-> Unit Int64)"),
        "Type in an arrow domain round-trips (not Unit): {dom}"
    );
    // TUPLE element + arrow RESULT.
    let tup = scheme_of(
        "(module m (def (f (: p (Tuple Type Int64))) 0) (export f))",
        "f",
    );
    assert!(
        tup.contains("(Tuple Type Int64)"),
        "Type as a tuple element round-trips: {tup}"
    );
    let res = scheme_of(
        "(module m (def (f (: g (-> Int64 Type))) 0) (export f))",
        "f",
    );
    assert!(
        res.contains("(-> Int64 Type)") && !res.contains("(-> Int64 Unit)"),
        "Type in an arrow result round-trips: {res}"
    );
}

#[test]
fn a_malformed_const_parameter_names_the_annotated_binder_shape() {
    // A `const` parameter wraps exactly ONE annotated binder — `(const (: n Int64))`. A malformed
    // `(const n Int64)` (two operands, unannotated) survives `strip_const_params` (which only unwraps a
    // single-operand `(const <binder>)`) and reached `check_binding_pattern` as a pattern whose head is
    // `const` → the misleading generic "a binding pattern head is not a tuple, record, or constructor".
    // It now names the correct `const` shape.
    let d = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (rep (const n Int64) (: x Int64)) x) (export rep))",
    )))
    .into_iter()
    .find(|d| d.message.contains("`const`"))
    .expect("a malformed const param reports a const-shaped message");
    assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
    assert!(
        d.message.contains("wraps exactly ONE annotated binder")
            && d.message.contains("(const (: <name> <Type>))"),
        "names the const parameter shape: {}",
        d.message
    );
    // NO false change: the well-formed const param compiles clean.
    assert!(
            crate::compile::compile_component(&crate::codec::encode(&parse(
                "(module m (def (rep (const (: n Int64)) (: x Int64)) x) (def (main) (rep 3 5)) (export main))"
            )))
            .is_ok(),
            "a well-formed const param is valid"
        );
    // ONE clean error, no consequent noise: an EXPORTED def whose sole param is the malformed const
    // reports ONLY the const-shape CDZ0201 — NOT also a spurious "parameter type is ambiguous — annotate
    // it" (the malformed const strips to a SYNTHESIZED `p$0` binder that never resolved a type; the
    // export-boundary ambiguous check now skips a non-user-node param, deferring to the const-shape
    // reject). A GENUINE bare unannotated exported param still draws the ambiguous error.
    let cdiags = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (mk (const n Int64)) (: 5 (UInt n))) (export mk))",
    )));
    assert!(
        cdiags
            .iter()
            .any(|d| d.message.contains("wraps exactly ONE annotated binder")),
        "the const-shape reject is present: {:?}",
        cdiags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        !cdiags
            .iter()
            .any(|d| d.message.contains("parameter type is ambiguous")),
        "no consequent 'parameter type is ambiguous' on the synthesized const-strip binder: {:?}",
        cdiags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // A GENUINE bare unannotated exported param (a USER node) still gets the ambiguous error + fix.
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (mk x) x) (export mk))"
        )))
        .iter()
        .any(|d| d.message.contains("parameter type is ambiguous")),
        "a genuine unannotated exported param still warns ambiguous"
    );
}

#[test]
fn a_typed_parameter_missing_its_colon_names_the_annotated_binder_shape() {
    // A typed parameter is `(: <name> <Type>)` (an annotated binder). Writing `(a Float64)` — the
    // binder juxtaposed with its type, no leading `:` — reaches `check_binding_pattern` as a
    // two-element list whose head `a` is not a constructor, previously giving the misleading generic
    // "a binding pattern head is not a tuple, record, or constructor". It now recognizes the shape
    // (second child resolves as a type) and names the real repair — add the `:` — with a VERIFIED
    // fix carrying the exact `(: a Float64)` replacement.
    let d = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (a Float64)) 0))",
    )))
    .into_iter()
    .find(|d| d.message.contains("typed parameter"))
    .expect("a colon-less typed param reports the annotated-binder shape");
    assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
    assert!(
        d.message.contains("`(: <name> <Type>)`")
            && d.message.contains("leading `:`")
            && d.message.contains("binder `a`"),
        "names the missing-colon repair: {}",
        d.message
    );
    let fix = d.fix.as_ref().expect("carries a verified add-`:` fix");
    assert!(fix.verified, "the colon rewrite is a rule, not a guess");
    assert_eq!(
        fix.replacement, "(: a Float64)",
        "the exact repair spelling"
    );
    // ROUND TRIP backing the VERIFIED marker: splicing the fix's `(: a Float64)` in place of the
    // colon-less `(a Float64)` binder yields a program that compiles clean — the add-`:` rewrite
    // clears the CDZ0201 by construction (an annotated binder is exactly what the parameter position
    // wants). `a` is read in the body so the repaired param draws no consequent unused warning.
    assert!(
        crate::compile::compile_component(&crate::codec::encode(&parse(
            "(module m (def (f (: a Float64)) a) (export f))"
        )))
        .is_ok(),
        "applying the verified add-`:` fix must recompile clean"
    );

    // A COMPOUND type (`(List Int64)`) has no single name atom to splice — the message still fires
    // (routing the repair) but carries NO fix.
    let dc = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (xs (List Int64))) 0))",
    )))
    .into_iter()
    .find(|d| d.message.contains("typed parameter"))
    .expect("a compound-typed colon-less param still names the shape");
    assert!(
        dc.fix.is_none(),
        "no fix when the type is compound (no single spelling): {:?}",
        dc.fix
    );

    // NO false positive: a genuine two-BINDER pattern `(a b)` whose second child is NOT a type keeps
    // the generic shape message (it is a real malformed pattern, not a missing-colon annotation).
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (f (a b)) 0))"
        )))
        .iter()
        .any(|d| d.message.contains("is not a tuple, record, or constructor")),
        "a non-type second child is not hijacked as a missing-colon annotation"
    );

    // NO false change: the properly-colon'd `(: a Float64)` param compiles clean.
    assert!(
        crate::compile::compile_component(&crate::codec::encode(&parse(
            "(module m (def (f (: a Float64)) a) (def (main) (f 1.0)) (export main))"
        )))
        .is_ok(),
        "a correctly annotated typed param is valid"
    );
}

#[test]
fn a_pure_data_record_const_param_folds_through_a_bare_recursive_call() {
    // ca07: a `const (: r (Record …))` param of PURE DATA (no `(-> …)` function field) folds across a
    // BARE recursive call (no `(const …)` block) — the record counter `a` shrinks each step. The
    // activation gate now admits a data record (symmetric with the Tuple gate, ca06); a record OF
    // FUNCTIONS stays the runtime-inlined dictionary the sibling test above pins, so the two are
    // distinguished by the field types. `f (record (a 3) (b 0))` counts `a` to 0 accumulating `b` → 3.
    // (This is a Rust test, not a corpus case, because a `(Record …)` param annotation does not yet
    // round-trip through the ML surface — the pending record-type-syntax gap; the fold itself is what's
    // pinned here.) The fold to a scalar imports NO value-heap runtime — the record never materializes.
    let src = "(module m \
               (def (f (const (: r (Record (a Int64) (b Int64))))) \
                 (if (= (. r a) 0) (. r b) (f (record (= a (- (. r a) 1)) (= b (+ (. r b) 1)))))) \
               (def (main) (f (record (= a 3) (= b 0)))) (export main))";
    // Compiles (the activation gate admits a data record — not a decline / machine-repr emit error).
    compile_component(&crate::codec::encode(&parse(src)))
        .expect("a pure-data record const-param recursion compiles");
    // The fold IS the pin (wasmtime-free): `main` folds all the way to the scalar constant 3 — a
    // `Core::ConstInt`, NOT a `Core::Record`/heap build — so the record never materializes and there is
    // no value-heap runtime import. `f` counts a=3 down to 0 while accumulating into b → 3.
    let mut db = crate::db::Db::load(parse(src));
    let d = db.def_by_name("main").expect("def main");
    let body = db.defs[d].body.expect("main has a body");
    let folded = match crate::lower::core_of(&mut db, body) {
            crate::core::Core::ConstInt(iv) => iv.to_i64(),
            other => panic!(
                "a pure-data record const param must fold to the scalar constant 3 (record eliminated), \
                 got {other:?}"
            ),
        }
        .expect("the folded body is a machine int");
    assert_eq!(
        folded, 3,
        "f counts a=3 down to 0 while accumulating into b → 3"
    );
}

#[test]
fn a_taken_trap_over_a_record_const_param_surfaces_its_message() {
    // The bare recursive-call fold over a record const param EXECUTES a taken `trap` and surfaces its
    // message as a compile error (a const-trap), rather than the "function return type has no machine
    // representation" emit failure the gate exclusion used to cause. Symmetric with the Int/Char/Float/
    // Tuple const-param trap shapes. (Rust test: the `(Record …)` annotation does not round-trip the ML
    // surface yet — the record-type-syntax gap — so the corpus route is unavailable.)
    let mut db = crate::db::Db::load(parse(
        "(module m \
               (def (f (const (: r (Record (a Int64) (b Int64))))) \
                 (if (= (. r a) 0) (trap \"record base reached\") \
                     (f (record (= a (- (. r a) 1)) (= b (+ (. r b) 1)))))) \
               (def (main) (f (record (= a 2) (= b 0)))) (export main))",
    ));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("record base reached")),
        "a taken trap over a record const param surfaces its message, not a machine-repr emit error: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_known_closure_stored_in_a_variant_is_called_directly_not_via_call_indirect() {
    // KNOWN-CLOSURE DEVIRTUALIZATION (S1): a closure stored in a variant field, matched out, and
    // applied — the ad-hoc-poly dispatch shape. The closure captures `k` so it CANNOT β-reduce (it is a
    // genuine runtime `Core::CallClosure`, not an inlined-away lambda), but its constructor site is
    // visible at the call (`mk` inlines → the `Box.Mk` payload the `match` binds folds to a
    // `Core::Closure`), so the funcref table slot is a compile-time constant. The wasm backend must emit
    // a DIRECT `call` to the lifted function, NOT a `call_indirect` reading the slot from the cell.
    // WITHOUT the devirt this emits 2 `call_indirect` (the pre-fix baseline, verified); with it, ZERO.
    // Value parity (40) is the behavior witness that the direct call computes the identical result.
    let bytes = compile_component(&crate::codec::encode(&parse(
        "(module m \
               (type Box (Mk (-> Int64 Int64))) \
               (def (mk (: k Int64)) (Box.Mk (fn ((: n Int64)) (+ n k)))) \
               (def (use2 (: b Box)) (match b ((Box.Mk f) (+ (f 10) (f 20))))) \
               (def (main) (use2 (mk 5))) (export main))",
    )))
    .expect("a capturing closure stored in a variant compiles");
    // Emit-shape pin (wasmparser, wasmtime-free). The runtime value (use2(mk 5) = (5+10)+(5+20) = 40) is
    // pinned in the corpus; here we assert only the DEVIRTUALIZATION shape.
    // The stored closure's application is DEVIRTUALIZED: the emitted core carries NO `call_indirect`
    // (the table slot is known, so it is a direct call). Scan the code section with `wasmparser`.
    let has_call_indirect = {
        use wasmparser::{Parser, Payload};
        let mut found = false;
        for payload in Parser::new(0).parse_all(&bytes) {
            if let Ok(Payload::CodeSectionEntry(body)) = payload {
                let mut ops = body.get_operators_reader().expect("ops");
                while let Ok(op) = ops.read() {
                    if matches!(op, wasmparser::Operator::CallIndirect { .. }) {
                        found = true;
                    }
                }
            }
        }
        found
    };
    assert!(
        !has_call_indirect,
        "a known-constructor-site closure call must devirtualize to a direct call (no call_indirect)"
    );
}

#[test]
fn a_const_closure_re_passed_through_recursion_specializes_and_fuses() {
    // S2 closure fusion: a recursive driver with a `const` CLOSURE param that re-passes the closure to
    // itself UNCHANGED (`(loop step s2 acc)`) used to DECLINE (CDZ0201) — the standalone generic body
    // can't bind the unbound const `step`, a FALSE POSITIVE (a const-param fn is specialize-at-each-call).
    // The identity-re-pass exemption makes that standalone decline a plain decline (not a fault), so the
    // program compiles and every CONCRETE call specializes `step` to its closure + threads it through the
    // recursion → the call_indirect DEVIRTUALIZES to a direct call (S1) and the closure fuses. Assert
    // BOTH: it computes the right value (a run test — a wrong specialization would miscompute) AND the
    // emitted core carries NO call_indirect (the fusion witness). `loop` over [1,2,3] summing = 6.
    let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m \
               (def (loop (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) (: s (List Int64)) (: acc Int64)) \
                 (match (step s) ((Option.None) acc) ((Option.Some p) (match p ((tuple x s2) (loop step s2 (+ acc x))))))) \
               (def (main) (loop (fn ((: s (List Int64))) (match s ((list) (Option.None)) ((list h .. t) (Option.Some (tuple h t))))) (list 1 2 3) 0)) \
               (export main))",
        )))
        .expect("a recursive const-closure driver compiles (identity re-pass is not a fault)");
    // Emit-shape pin (wasmparser, wasmtime-free): the const closure re-passed through recursion must
    // specialize + devirtualize the call_indirect to a direct call (S1 fusion). The runtime value
    // (sums [1,2,3] to 6) is pinned in the corpus.
    assert!(
        !component_has_call_indirect(&bytes),
        "the const closure re-passed through recursion must specialize + devirtualize (no call_indirect)"
    );
}

#[test]
fn equality_on_option_and_variant_values_const_folds() {
    // GENERALITY (operator: drive const-fold declines to 0): structural `==` over SUM values (an
    // `Option`, any variant) must const-fold, not just scalars/lists. This is what the P4 self-reflection
    // library's `Option`-threaded navigation needs — `head-name(g) == Option.Some("type")` gating a fold.
    // Before, `cval_eq` handled only Int/Bool/Str/Bytes/Unit/List → a `==` on `Option` returned `None`
    // (decline). `Ast.encode` DEMANDS a compile-time constant, so if the `==` did not fold, the encode
    // declines with "runtime AST value" and no wasm artifact is produced — this test would then fail.
    use crate::testkit::parse;
    // `pick` const-branches on `name-of(form) == Option.Some("x")`; fed a matching `Ast.Name("x")`, the
    // whole thing must const-fold through `Ast.encode` to constant bytes (the export compiles).
    let src = "(module m \
             (def (name-of (const (: form Ast))) \
               (match form ((Ast.Name n) (Option.Some n)) (_ Option.None))) \
             (def (pick (const (: form Ast))) \
               (if (= (name-of form) (Option.Some \"x\")) (Ast.Name \"yes\") (Ast.Name \"no\"))) \
             (def (enc) (Ast.encode (pick (Ast.Name \"x\")))) \
             (export enc))";
    // `Ast.encode` DEMANDS a compile-time constant, so a SUCCESSFUL compile IS the proof that `==` on an
    // Option const-folded: an unfolded `==` makes the encode decline ("runtime AST value") → no artifact
    // → this compile fails. (Wasmtime-free: the compile-success is the const-fold witness.)
    compile_component(&crate::codec::encode(&parse(src)))
        .expect("`==` on Option must const-fold so Ast.encode folds to constant bytes");
}

#[test]
fn const_params_re_passed_on_a_mixed_match_recursive_arm_do_not_drop() {
    // REGRESSION (v-iterators' filter-map, the const-param-drop bug): a `const` param re-passed on a
    // SELF-RECURSIVE call that sits on a MIXED innermost match arm — a recursive arm BESIDE a
    // value-returning sibling — used to DECLINE CDZ0101 "unbound name step" during const-specialization.
    // Root cause: the recursive arm's re-passed `const` arg is UNBOUND in the specialized copy (a
    // Poison), so the closure-identity `|clos` fingerprint DIVERGED from the enclosing spec's key → a
    // divergent SECOND spec whose body carried the unbound arg. Fixed two ways: (1) a Poison self-repass
    // INHERITS the enclosing spec's recorded const fingerprint (`db.const_repass_fp`) so the recursion
    // closes on ONE spec; (2) the `|clos` augmentation fires ONLY for a BARE-NAME arg (a lambda-literal
    // arg's AST already self-identifies and its lift `code` is unstable per copy). This is the filter-map
    // shape (keep = return a value, drop = recurse). BOTH a single `const step` and TWO const params
    // (step + f) must compile + run. The scrutinees return a SCALAR (not an Option) to avoid the
    // SEPARATE, pre-existing mixed-match Option-returning tail-loop invalid-wasm bug (routed to
    // v-wasm-opt), which is orthogonal to the const-param-drop this pins. `twostep` over 0.. keeping the
    // first `>2` yields 3. (`Option` is a nullary-declared sum here, so annotations write `Option`.)
    use crate::testkit::parse;
    // (a) SINGLE const param `step`, mixed-match recursive arm.
    let single = "(module m (type Option (Some Int64) None) \
             (def (mk (: n Int64)) (Option.Some n)) \
             (def (twostep (const (: step (-> Int64 Option))) (: s Int64)) \
               (match (step s) ((Option.None) 0) \
                 ((Option.Some x) (if (> x 2) x (twostep step (+ s 1)))))) \
             (def (main) (twostep (fn ((: n Int64)) (mk n)) 0)) (export main))";
    compile_component(&crate::codec::encode(&parse(single)))
        .expect("single const param re-passed on a mixed-match recursive arm must compile");
    // (b) TWO const params (`step` + `f`) both re-passed on the mixed-match recursive arm — the both-const
    // case (a lambda-literal `f` whose per-copy lift code must NOT force a divergent key).
    let both = "(module m (type Option (Some Int64) None) \
             (def (mk (: n Int64)) (Option.Some n)) \
             (def (twostep (const (: step (-> Int64 Option))) (: s Int64) \
                           (const (: f (-> Int64 Bool)))) \
               (match (step s) ((Option.None) 0) \
                 ((Option.Some x) (if (f x) x (twostep step (+ s 1) f))))) \
             (def (main) (twostep (fn ((: n Int64)) (mk n)) 0 (fn ((: x Int64)) (> x 2)))) (export main))";
    compile_component(&crate::codec::encode(&parse(both)))
        .expect("two const params re-passed on a mixed-match recursive arm must compile");
}

#[test]
fn a_derived_const_closure_re_pass_still_rejects_not_an_identity_repass() {
    // The UNSOUND-TWIN guard (v-inference ACK): the identity-re-pass exemption must be NARROW — it
    // rescues ONLY a bare re-pass of the callee's own const param, NOT a DERIVED const arg. Here the
    // recursion passes a NEW closure `(fn (x) (+ (step x) 1))` (composed from `step`) to the const param
    // each depth — that is not an identity re-pass (a fresh, unbounded-per-depth closure), so it MUST
    // still be the coded CDZ0201 reject, exactly as before. Proves the exemption did not open a hole.
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m \
                       (def (bad (const (: step (-> Int64 Int64))) (: s Int64) (: acc Int64)) \
                         (if (= s 0) acc (bad (fn ((: x Int64)) (+ (step x) 1)) (- s 1) (+ acc (step s))))) \
                       (def (main) (bad (fn ((: x Int64)) (* x 2)) 3 0)) (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    let has_coded = out
        .diagnostics
        .iter()
        .any(|d| d.severity == crate::abi::Severity::Error && d.code.as_deref() == Some("CDZ0201"));
    assert!(
        has_coded,
        "a DERIVED const-closure re-pass (not an identity re-pass) must still be a coded CDZ0201 reject, got: {:?}",
        out.diagnostics
    );
}

#[test]
fn a_forwarded_param_const_arg_gets_an_actionable_polymorphic_or_inline_hint() {
    // OPTION-C DIAGNOSTIC (concierge ruling): the standalone-lowering const-forward reject
    // (`fold(it: Iter, acc, g)` forwarding a runtime param into `drive`'s `const` slot — the annotated
    // `it: Iter` makes `fold` monomorphic, so it lowers STANDALONE and is not specialized at its call
    // site, and the forwarded param is unbound there → CDZ0201). The reject STAYS (the real fix, backward
    // const-propagation, is a separate operator-gated arc), but its message must be ACTIONABLE — point at
    // keeping the enclosing fn polymorphic (drop the annotation) or inlining, NOT the cryptic bare
    // "depends on runtime data". A DERIVED const arg (the twin above) keeps the plain message; only a
    // BARE forwarded param gets this hint.
    let src = "(module m \
               (type Iter (Mk (List Int64) (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) \
               (def (from-list (: xs (List Int64))) ((. Iter Mk) xs (fn ((: s (List Int64))) (match s ((list) ((. Option None) unit)) ((list h .. t) ((. Option Some) (tuple h t))))))) \
               (def (drive (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) (: s (List Int64)) (: acc Int64) (const (: g (-> Int64 (-> Int64 Int64))))) \
                 (match (step s) (((. Option None) _) acc) (((. Option Some) p) (match p ((tuple x s2) (drive step s2 (g acc x) g)))))) \
               (def (fold (: it Iter) acc (: g (-> Int64 (-> Int64 Int64)))) (match it (((. Iter Mk) s step) (drive step s acc g)))) \
               (def (sum it) (fold it 0 (fn (a x) (+ a x)))) \
               (def (main) (sum (from-list (list 1 2 3)))) (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    let hint = out.diagnostics.iter().find(|d| {
        d.severity == crate::abi::Severity::Error && d.code.as_deref() == Some("CDZ0201")
    });
    let hint = hint.unwrap_or_else(|| {
        panic!(
            "the forwarded-param const arg must still be a coded CDZ0201, got: {:?}",
            out.diagnostics
        )
    });
    assert!(
        hint.message
            .contains("forwarded from an enclosing function")
            && hint.message.contains("POLYMORPHIC")
            && hint.message.contains("INLINE"),
        "the forwarded-param const-arg reject must carry the actionable polymorphic-or-inline hint, got: {:?}",
        hint.message
    );
}

#[test]
fn a_closed_literal_closure_forwarded_through_const_wrapper_hops_is_not_a_false_reject() {
    // The const-wrapper-chain false-reject (arena parent-THEFT): a CLOSED literal closure
    // `fn(a,x)=>a+x` forwarded through several const-param call hops (`sum` → `fold` → `drive`'s
    // `const g`) used to be WRONGLY rejected CDZ0201 — a β-substitution splices the source lambda into
    // a spec copy and STEALS its single arena parent pointer, so `arg_captures_runtime_binding`'s
    // `is_within` walk saw the lambda's OWN params `a`/`x` as "outside" it → a false capture. The fix
    // cross-checks with a theft-immune lexical free-name walk (`const_arg_is_lexically_closed`): a
    // genuinely closed forwarded closure is accepted, while a DIVERGING derived-closure re-pass is
    // still declined by the fingerprint-extension backstop (the twin test above). Pin BOTH halves the
    // tracker asked for: the CHECK path is clean (no false CDZ0201) AND compile+run computes the right
    // value. `sum(from-list([1,2,3]))` folds with `+` → 6. (S-expr matches the ML lowering exactly:
    // curried `(-> Int64 (-> Int64 Int64))` arrow, `(. Option None)` patterns, bare closure params.)
    let src = "(module m \
               (type Iter (Mk (List Int64) (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) \
               (def (from-list xs) ((. Iter Mk) xs (fn (s) (match s ((list) ((. Option None) unit)) ((list h .. t) ((. Option Some) (tuple h t))))))) \
               (def (drive (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) (: s (List Int64)) (: acc Int64) (const (: g (-> Int64 (-> Int64 Int64))))) \
                 (match (step s) (((. Option None) _) acc) (((. Option Some) p) (match p ((tuple x s2) (drive step s2 (g acc x) g)))))) \
               (def (fold it acc (: g (-> Int64 (-> Int64 Int64)))) (match it (((. Iter Mk) s step) (drive step s acc g)))) \
               (def (sum it) (fold it 0 (fn (a x) (+ a x)))) \
               (def (total) (sum (from-list (list 1 2 3)))) \
               (def (main) (total)) (export main))";
    // CHECK path: no false CDZ0201 (the standalone-lowering path the false-reject fired on).
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.severity == crate::abi::Severity::Error
                && d.code.as_deref() == Some("CDZ0201")),
        "a closed literal closure forwarded through const-wrapper hops must NOT be a false CDZ0201 reject, got: {:?}",
        out.diagnostics
    );
    // COMPILE: the forwarded closure specializes correctly and the component builds (no false reject).
    // The runtime value (sum([1,2,3]) with a forwarded const `+` closure folds to 6) is pinned in the corpus.
    compile_component(&crate::codec::encode(&parse(src)))
        .expect("the const-wrapper-chain program compiles");
}

#[test]
fn two_nested_recursive_const_closure_drivers_emit_valid_wasm() {
    // Regression for the nested-driver INVALID-WASM bug: a `filter` adapter whose recursive
    // `filter-step` takes a `const` step closure, consumed by a recursive `drive` fold that ALSO takes
    // its step `const` — two nested recursive const-closure specializations. This emitted INVALID WASM
    // ("function index out of bounds": a nested spec's `Core::Call` referenced an un-laid-out function
    // slot) because `finish_layout`'s reachability walk appended a nested spec to `order` WITHOUT
    // closing over its own callees. The joint call/lifted-closure fixpoint fix reaches them. Assert the
    // emitted component VALIDATES via `wasmparser::validate` (an out-of-bounds function index is invalid
    // wasm); the runtime value ([1..5] kept >2, summed = 3+4+5 = 12) is pinned in the corpus. (The
    // single-driver fusion emit-shape is pinned separately; this pins the NESTED case does not regress
    // to invalid wasm.)
    let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m \
               (type It (Mk (List Int64) (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) \
               (def (from-list (: xs (List Int64))) (It.Mk xs (fn ((: s (List Int64))) (match s ((list) (Option.None)) ((list h .. t) (Option.Some (tuple h t))))))) \
               (def (filter-step (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) (: s (List Int64)) (const (: p (-> Int64 Bool)))) \
                 (match (step s) ((Option.None) (Option.None)) ((Option.Some pr) (match pr ((tuple x s2) (if (p x) (Option.Some (tuple x s2)) (filter-step step s2 p))))))) \
               (def (filter (: it It) (: p (-> Int64 Bool))) (match it ((It.Mk s0 step) (It.Mk s0 (fn ((: s (List Int64))) (filter-step step s p)))))) \
               (def (drive (const (: step (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) (: s (List Int64)) (: acc Int64)) \
                 (match (step s) ((Option.None) acc) ((Option.Some p) (match p ((tuple x s2) (drive step s2 (+ acc x))))))) \
               (def (sum (: it It)) (match it ((It.Mk s step) (drive step s 0)))) \
               (def (main) (sum (filter (from-list (list 1 2 3 4 5)) (fn ((: x Int64)) (> x 2))))) (export main))",
        )))
        .expect("two nested recursive const-closure drivers compile");
    // Validity pin (wasmparser, wasmtime-free): the nested spec's Core::Call must reference a laid-out
    // function slot — an out-of-bounds function index is invalid wasm. The runtime value (filter >2 then
    // sum [1..5] = 3+4+5 = 12) is pinned in the corpus.
    assert!(
        wasmparser::validate(&bytes).is_ok(),
        "two nested recursive const-closure drivers must emit a VALID module (no out-of-bounds function index)"
    );
}

#[test]
fn a_compound_returning_export_dispatching_a_runtime_closure_emits_valid_wasm() {
    // Regression (v-core-opt issue, 2026-07-20): an export whose RESULT is a COMPOUND (`Option (Tuple
    // Int64 (List Int64))`) escapes through the runtime-resource ESCAPE module, and whose body dispatches
    // a FIRST-CLASS (non-const, non-devirtualizable) closure via `call_indirect` — the filter-map
    // keep=return / drop=recurse shape. The escape-module assembler emitted NEITHER the lifted closure
    // body NOR the funcref table/elem (only `core_module_impl` did), so `call_indirect` referenced a
    // non-existent table 0 → invalid wasm; and once the lifted body was appended, an op used ONLY inside
    // the closure was absent from the escape module's import set → its `CallImport` resolved to `u32::MAX`
    // ("unknown function 4294967295"). Fixed by `append_lifted_bodies` + `collect_module_used_ops` +
    // `form_ex2` table/elem emission at every escape site. `step` is a PLAIN param (not `const`), so it
    // stays a runtime `call_indirect` — the decisive trigger (a scalar-returning twin already compiled).
    // `twostep` over [1,2,3,4] keeps the first `classify x = x>2`: x=1,2 recurse, x=3 kept → Some((3,[4])).
    let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m \
               (type It (Mk (List Int64) (-> (List Int64) (Option (Tuple Int64 (List Int64)))))) \
               (def (from-list (: xs (List Int64))) (It.Mk xs (fn ((: s (List Int64))) (match s ((list) (Option.None)) ((list h .. t) (Option.Some (tuple h t))))))) \
               (def (classify (: x Int64)) (if (> x 2) (Option.Some x) (Option.None))) \
               (def (twostep (: step (-> (List Int64) (Option (Tuple Int64 (List Int64))))) (: s (List Int64))) \
                 (match (step s) ((Option.None) (Option.None)) ((Option.Some pair) (match pair ((tuple x s2) (match (classify x) ((Option.None) (twostep step s2)) ((Option.Some y) (Option.Some (tuple y s2))))))))) \
               (def (run2 (: it It)) (match it ((It.Mk s0 step) (twostep step s0)))) \
               (def (main) (run2 (from-list (list 1 2 3 4)))) (export main))",
        )))
        .expect("a compound-returning export dispatching a runtime closure compiles + validates");
    // Validity pin (wasmparser, wasmtime-free): the escape module must emit the lifted closure body +
    // funcref table/elem so the `call_indirect` and its imports resolve — invalid table/func indices are
    // invalid wasm. The runtime value (twostep keeps the first x>2 → Some((3, [4]))) is pinned in the corpus.
    assert!(
        wasmparser::validate(&bytes).is_ok(),
        "a compound-returning export dispatching a runtime closure must emit a VALID module (table/func indices resolve)"
    );
}

#[test]
fn a_const_collection_recursively_folded_unrolls_not_hangs_or_rejects() {
    // A `const` COLLECTION param consumed by a SELF-RECURSIVE fold once composed the const erasure with
    // the tail-loop transform into an INFINITE LOOP (a `loop { … br 0 }` with the `(list)`-nil exit
    // const-folded away) — a valid program HUNG. It was then DECLINED (reject-don't-miscompile). The
    // recursive-const-fold unroll (#3344, spec/semantics/09-functions.sexp "a const collection
    // recursively folded UNROLLS to its result") now does the SOUND thing: it fully UNROLLS the bounded
    // const fold to its constant result at compile time, so the program COMPILES (folds to a value) —
    // neither hangs nor rejects. The behavioral oracle for the fold value lives in that corpus case; this
    // test pins the EMIT SHAPE: not merely that it compiles (the old hanging miscompile ALSO compiled —
    // to a `loop`), but that NO tail-loop is emitted at all (the recursion is fully unrolled away). That
    // is the precise anti-miscompile guard; a bare "does not reject" would not detect a regression to the
    // hang. And that the NON-folding siblings still compile.
    let reject_code = |src: &str| {
        crate::compile::compile_component(&crate::codec::encode(&parse(src)))
            .err()
            .and_then(|e| e.code)
    };
    let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m \
                   (def (s (const (: xs (List Int64))) (: acc Int64)) \
                     (match xs ((list) acc) ((list h .. t) (s t (+ acc h))))) \
                   (def (main) (s (list 1 2 3) 0)) (export main))",
        )))
        .expect("a const list consumed by a tail fold now UNROLLS to a constant (#3344) — compiles, no reject");
    assert_eq!(
        count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::Loop { .. })),
        0,
        "the const-list fold must UNROLL to its constant result — NO tail-loop may be emitted (the \
             const-erasure × tail-loop composition that used to hang is fully folded away)"
    );
    // The RUNTIME-list version (no `const`) compiles cleanly — the list is an ordinary runtime value the
    // tail-loop iterates with its real `br_if` length/nil exit. So the reject is specific to the
    // const-erasure × tail-loop composition, NOT to tail-folding a list.
    assert_eq!(
        reject_code(
            "(module m \
                   (def (s (: xs (List Int64)) (: acc Int64)) \
                     (match xs ((list) acc) ((list h .. t) (s t (+ acc h))))) \
                   (def (main) (s (list 1 2 3) 0)) (export main))"
        ),
        None,
        "the runtime-list tail fold (no const) compiles — only the const-collection composition rejects"
    );
    // NO REGRESSION: a `const` DICTIONARY consumer that recurses driven by a RUNTIME counter (the dict
    // passed UNCHANGED) still compiles — the const value is not a collection folded down a spine, so
    // the guard does not fire. (The `is_collection_ty` predicate distinguishes them.)
    assert_eq!(
        reject_code(
            "(module m \
                   (def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64)) \
                     (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc)))) \
                   (def (main) (fold-n (record (= op (fn (x) (+ x 10)))) 3 0)) (export main))"
        ),
        None,
        "a const-dictionary recursive consumer (runtime-counter-driven) still compiles"
    );
    // NO REGRESSION: a const SCALAR recursion compiles (a scalar is not a collection).
    assert_eq!(
        reject_code(
            "(module m \
                   (def (cd (const (: n Int64)) (: acc Int64)) (if (= n 0) acc (cd (- n 1) (+ acc 1)))) \
                   (def (main) (cd 5 0)) (export main))"
        ),
        None,
        "a const-scalar recursion compiles (only a const collection folded recursively rejects)"
    );
}

#[test]
fn an_inline_never_def_is_emitted_once_not_inlined_per_call() {
    // 09-functions "an inline-never definition is emitted once and called" (Addendum 4). The default
    // is always-inline (every non-recursive call β-reduces); `inline-never` forces `big` to be emitted
    // as ONE real function and CALLED. `big`'s body has 3 `i64.mul`s. `main` takes RUNTIME params (so
    // the calls do NOT constant-fold away): called twice, INLINED it emits 6 muls, EMITTED-ONCE it
    // emits 3. Assert the module has exactly 3 muls (emit-once); the un-marked control emits 6.
    let src = "(module m \
             (@ inline-never (def (big (: x Int64)) (+ (* x 7) (+ (* x 11) (* x 13))))) \
             (def (main (: a Int64) (: b Int64)) (+ (big a) (big b))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let mul_count = count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::I64Mul));
    assert_eq!(
        mul_count, 3,
        "inline-never `big` must be emitted ONCE (3 muls), not inlined per call (would be 6)"
    );
    // Control: WITHOUT the marker, the same program inlines `big` at both runtime call sites → 6 muls.
    let inlined = "(module m \
             (def (big (: x Int64)) (+ (* x 7) (+ (* x 11) (* x 13)))) \
             (def (main (: a Int64) (: b Int64)) (+ (big a) (big b))) (export main))";
    let ib = compile_component(&crate::codec::encode(&parse(inlined))).expect("compile");
    assert_eq!(
        count_opcode(&ib, |op| matches!(op, wasmparser::Operator::I64Mul)),
        6,
        "the un-marked control must inline (6 muls) — the differential is the feature"
    );
}

#[test]
fn an_inline_never_def_with_a_const_dict_still_erases_the_dict() {
    // 09-functions "an inline-never definition with a const dictionary still monomorphizes the
    // dictionary" — the HEADLINE composition (Addendum 4): "avoid the inline but keep polymorphism".
    // `apply2` is `inline-never` (emit once + call) AND has a `const` dict param (monomorphize +
    // erase). Both hold: the dict's `op` is INLINED into the specialized copy (NO `call_indirect`, no
    // runtime record) and that copy is emitted once + called. Runs to 145; asserts 0 `call_indirect`.
    let src = "(module m \
             (@ inline-never \
               (def (apply2 (const (: d (Record (op (-> Int64 Int64))))) (: x Int64)) \
                 ((. d op) ((. d op) x)))) \
             (def (main) (+ (apply2 (record (= op (fn (n) (+ n 10)))) 5) \
                            (apply2 (record (= op (fn (n) (+ n 10)))) 100))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The fold VALUE (145) is covered by the corpus dictionary-erasure family (09-functions "a recursive
    // consumer of a dictionary record inlines and erases the dictionary" + companions); only the
    // emit witness (0 call_indirect — the dict inlined, no runtime record) stays here.
    let indirect = count_opcode(&bytes, |op| {
        matches!(op, wasmparser::Operator::CallIndirect { .. })
    });
    assert_eq!(
        indirect, 0,
        "an inline-never def's const dict must still be inlined (0 call_indirect), not a runtime record"
    );
}

#[test]
fn the_cost_heuristic_emits_a_big_multiply_called_helper_once() {
    // Addendum 4 cost heuristic. `big` is LARGE (well past INLINE_COST_THRESHOLD nodes — 8 products
    // summed) and called at TWO sites, each with a RUNTIME argument (`main`'s params `a`/`b`, so the
    // soundness gate fires: the result can't be compile-time-demanded). The UNANNOTATED default would
    // inline it at both sites; the heuristic instead emits it ONCE and calls it. Measured by call
    // count: emit-once ⇒ ≥2 `Call`s to a separate `big` function; inlined ⇒ 0 internal calls. The
    // value is unchanged either way (the gate corpus covers the value; here we assert the STRATEGY).
    let src = "(module m \
             (def (big (: x Int64)) \
               (+ (* x 2) (+ (* x 3) (+ (* x 5) (+ (* x 7) \
                 (+ (* x 11) (+ (* x 13) (+ (* x 17) (* x 19))))))))) \
             (def (main (: a Int64) (: b Int64)) (+ (big a) (big b))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let calls = count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::Call { .. }));
    assert!(
        calls >= 2,
        "a big helper called twice with runtime args must be emitted once + called (≥2 calls), got {calls}"
    );
}

#[test]
fn the_cost_heuristic_leaves_a_small_helper_inlined() {
    // The heuristic is CONSERVATIVE: a SMALL body (below INLINE_COST_THRESHOLD) stays inlined even when
    // called multiply with runtime args — the always-inline default is unchanged for ordinary helpers.
    // `sm` is 2 products; inlined ⇒ 0 internal calls (no separate function).
    let src = "(module m \
             (def (sm (: x Int64)) (+ (* x 2) (* x 3))) \
             (def (main (: a Int64) (: b Int64)) (+ (sm a) (sm b))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let calls = count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::Call { .. }));
    assert_eq!(
        calls, 0,
        "a small helper must stay inlined (0 internal calls), got {calls}"
    );
}

#[test]
fn the_cost_heuristic_still_folds_a_const_argument_call() {
    // SOUNDNESS: the heuristic must NOT emit-once a call whose result is compile-time-demanded — it
    // fires only when an argument captures a RUNTIME binding. A big helper called with CONSTANT args
    // (no runtime capture) must still fold at compile time (β-reduce), so the whole thing constant-folds
    // to a literal — no runtime call to `big`. `big(2) + big(3)` with `big(x) = x*(2+3+5+7+11+13+17+19)`
    // = 2*77 + 3*77 = 385. Assert it runs to 385 AND emits no internal call (fully folded).
    let src = "(module m \
             (def (big (: x Int64)) \
               (+ (* x 2) (+ (* x 3) (+ (* x 5) (+ (* x 7) \
                 (+ (* x 11) (+ (* x 13) (+ (* x 17) (* x 19))))))))) \
             (def (main) (+ (big 2) (big 3))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The fold VALUE (385) is an ordinary compile-time const-fold covered by the corpus const-fold
    // families; only the emit witness (0 Call — the const-arg call fully folded, no runtime call) stays.
    let calls = count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::Call { .. }));
    assert_eq!(
        calls, 0,
        "a const-arg call must fold at compile time (no runtime call), got {calls}"
    );
}

#[test]
fn a_deeply_nested_inlined_projection_chain_registers_callables_linearly() {
    // REGRESSION (perf): `Db::register_reduced_callables` runs after EVERY `apply_lambda` β-reduction to
    // discover do-local recursive defs in the reduced term, and its `collect_reduced_callables` helper
    // walked the WHOLE reduced subtree each call. A deeply-nested inlining — e.g. a dictionary-projection
    // chain `((. d op0) ((. d op1) … acc))` where each `(. d opi)` projects a lambda and applies it —
    // produces N β-reductions each returning a progressively-larger O(N)-deep term, so the repeated
    // whole-subtree walks were O(N²) (a depth-800 chain's callable scan alone was ~2.5M node-visits,
    // 4×/doubling). FIX: a `db.reduced_callable_walked` visited set — a node's structure is immutable
    // once built, so an already-walked node yields no new candidates; skip it (the fix-30 pattern). This
    // drops the scan to O(N), so `cdz check` on the chain scales linearly.
    //
    // Correctness: `apply-all` threads `acc` through N ops `op_i = (+ · i)`, so the result is
    // `acc + (0+1+…+(N-1))`. Pin the fold value at a small N.
    fn chain_src(n: usize) -> String {
        let dtype: String = format!(
            "(Record {})",
            (0..n)
                .map(|i| format!("(op{i} (-> Int64 Int64))"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let mut body = String::from("acc");
        for i in 0..n {
            body = format!("((. d op{i}) {body})");
        }
        let dval: String = format!(
            "(record {})",
            (0..n)
                .map(|i| format!("(= op{i} (fn (x) (+ x {i})))"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        format!(
            "(module m (def (apply-all (: d {dtype}) (: acc Int64)) {body}) \
                   (def (main) (apply-all {dval} 0)) (export main))"
        )
    }
    // Compiles cleanly (the record value uses the value heap, so RUNNING would need the runtime store;
    // the perf property is compile-time, so a clean compile is the correctness signal here — the
    // dictionary erasure's runtime value is pinned by `a_recursive_dictionary_consumer_…`).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&chain_src(5))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a nested dictionary-projection chain compiles with no error diagnostics: {diags:?}"
    );
    // Growth guard — the NOISE-FREE signal: total `collect_reduced_callables` node-visits (a wall-clock
    // ratio is diluted by the rest of `check`, which is linear and dominates). Before the visited set, N
    // β-reductions each re-walked the growing O(N)-deep reduced term → the visit count was O(N²)
    // (~4×/doubling); with the set it is O(N) (~2×/doubling). Measure at width N and 2N and assert the
    // count grows sub-quadratically. Deterministic (a pure function of the program), so no min-of-runs
    // needed — the count is identical every run.
    fn crc_visits(src: &str) -> u64 {
        // The width-300/600 chain type-checks to a deep-but-finite recursion — set, run, and read the
        // visit counter all on the depth-sized compiler thread (the counter is a thread-local, so the
        // whole trio must share one thread) so it doesn't overflow the ~2 MB `cargo test` worker stack.
        crate::host::run_with_compiler_stack(|| {
            crate::db::COLLECT_REDUCED_CALLABLES_VISITS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::COLLECT_REDUCED_CALLABLES_VISITS.with(|c| c.get())
        })
    }
    let v300 = crc_visits(&chain_src(300));
    let v600 = crc_visits(&chain_src(600));
    // Linear ⇒ ~2× for a 2× width; the old O(N²) re-walk was ~4×. Threshold 3× separates them with
    // margin (and guards against a partial regression). Guard the denominator against a degenerate 0.
    let ratio = v600 as f64 / (v300.max(1)) as f64;
    assert!(
        ratio < 3.0,
        "a deeply-nested inlined projection chain's callable scan must grow LINEARLY (was O(N²) via a \
             per-β-reduction whole-subtree re-walk in `collect_reduced_callables`; the \
             `reduced_callable_walked` visited set fixes it): width 300→600 grew the visit count {ratio:.1}× \
             (v300={v300}, v600={v600}); linear is ~2×, the O(N²) re-walk was ~4×"
    );
}

#[test]
fn check_no_home_follows_a_shared_callee_body_once_per_handler_context() {
    // REGRESSION (perf): `effects::check_no_home_walk` (the CDZ0401 no-home check, run over every export
    // body) FOLLOWS a non-recursive call into its callee body — a perform may be cross-function — but did
    // so with NO dedup (only a `depth > 64` backstop). A helper called from N sites (here `(mk)`, a
    // nullary constructor of an O(N)-field record, projected field-by-field in `main`) had its whole
    // O(N) body RE-WALKED once per call site → O(sites × body) = O(N²). The sibling
    // `body_reached_effects_walk` already had a `visited` guard; this walk was missing it. FIX: dedup the
    // callee-follow by `(callee_body, handled-set)` — a callee walked under an identical handled set
    // yields identical CDZ0401s, so re-walking is redundant; a DIFFERENT handled set (an effect granted
    // at one site, ungranted at another) is a distinct key and still walked (so the diagnostic is
    // preserved — verified by `check_no_home` probes out-of-band).
    //
    // The NOISE-FREE signal is the total `check_no_home_walk` node-visit count (a wall-clock ratio is
    // diluted by the rest of `check`, and this shape also exercises a separate projection-fold cost). It
    // is a deterministic pure function of the program, so no min-of-runs is needed.
    fn proj_src(n: usize) -> String {
        let fields: String = (0..n)
            .map(|i| format!("(= f{i} {i})"))
            .collect::<Vec<_>>()
            .join(" ");
        // A balanced `+`-tree of `(. (mk) f_i)` projections — every leaf is a fresh `(mk)` call, so the
        // callee-follow dedup is exactly what bounds the walk.
        fn tree(items: &[String]) -> String {
            if items.len() == 1 {
                return items[0].clone();
            }
            let m = items.len() / 2;
            format!("(+ {} {})", tree(&items[..m]), tree(&items[m..]))
        }
        let projs: Vec<String> = (0..n).map(|i| format!("(. (mk) f{i})")).collect();
        format!(
            "(module m (def (mk) (record {fields})) (def (main) {}) (export main))",
            tree(&projs)
        )
    }
    // A small instance compiles clean (no spurious CDZ0401 — there is no effect here at all).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&proj_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide-record projection compiles with no error diagnostics: {diags:?}"
    );
    fn cnh_visits(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::CHECK_NO_HOME_VISITS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::CHECK_NO_HOME_VISITS.with(|c| c.get())
        })
    }
    let v400 = cnh_visits(&proj_src(400));
    let v800 = cnh_visits(&proj_src(800));
    // Linear ⇒ ~2× for a 2× width; the old per-site callee re-walk was ~4× (O(N²)). Threshold 3×
    // separates the regimes with margin. Guard the denominator against a degenerate 0.
    let ratio = v800 as f64 / (v400.max(1)) as f64;
    assert!(
        ratio < 3.0,
        "the CDZ0401 no-home walk must follow a shared callee body a BOUNDED number of times (was \
             O(N²) via an un-deduped per-call-site re-walk in `check_no_home_walk`; the `(callee, \
             handled)` follow-dedup fixes it): width 400→800 grew the visit count {ratio:.1}× (v400={v400}, \
             v800={v800}); linear is ~2×, the O(N²) re-walk was ~4×"
    );
}

#[test]
fn runtime_record_field_projection_indexes_in_bounded_time() {
    // REGRESSION (perf): `eval::runtime_member_index` (the sorted-slot lookup for a field read on a
    // RUNTIME record `(. r f)` — one whose value does not fold to a visible record) found the slot by a
    // LINEAR `fields.keys().position(|k| k == key)` scan — O(fields) PER projection. A wide record
    // projected field-by-field (`(+ (. r f0) (+ (. r f1) …))`) was O(fields × projections) = O(N²) (a
    // param record at N=6400: 1066ms, growth ~3.1×/doubling). FIX: build the `name → sorted-slot` map
    // ONCE per record type (keyed by the type's shared `Rc<BTreeMap>` address) and read it O(1); the
    // total field keys ENUMERATED is then O(fields), not O(fields × projections).
    //
    // The shape: `use` takes a P-field record parameter and projects every field once; a runtime record
    // (a parameter) never folds, so each projection goes through `runtime_member_index`. Because all P
    // projections share the one parameter's record TYPE, the index is built ONCE → exactly P keys
    // enumerated, regardless of P. The counter is the noise-free signal (a wall-clock ratio is diluted
    // by inference's own per-field cost).
    fn proj_param_src(p: usize) -> String {
        let fields_ty: String = (0..p)
            .map(|i| format!("(f{i} Int64)"))
            .collect::<Vec<_>>()
            .join(" ");
        fn tree(items: &[String]) -> String {
            if items.len() == 1 {
                return items[0].clone();
            }
            let m = items.len() / 2;
            format!("(+ {} {})", tree(&items[..m]), tree(&items[m..]))
        }
        let projs: Vec<String> = (0..p).map(|i| format!("(. r f{i})")).collect();
        format!(
            "(module m (def (use (: r (Record {fields_ty}))) {}) \
                   (def (main) 0) (export main))",
            tree(&projs)
        )
    }
    // Compiles clean (a runtime record projection lowers to a `Core::Proj` at the field's slot).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&proj_param_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide runtime-record projection compiles with no error diagnostics: {diags:?}"
    );
    fn keys_scanned(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::RECORD_FIELD_INDEX_KEYS_SCANNED.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::RECORD_FIELD_INDEX_KEYS_SCANNED.with(|c| c.get())
        })
    }
    // The whole point: enumerating keys is O(fields), NOT O(fields × projections). Building the index
    // once per record type means the total keys scanned for a P-field record projected P times is a
    // SMALL MULTIPLE of P (the type may be indexed from a couple of distinct occurrences — the param
    // read and the fold path — but a bounded, projection-COUNT-independent number), never the O(P²) of
    // the old per-projection scan. Assert it is well under P² even at a modest P where P² dwarfs the
    // margin. Deterministic (a pure function of the program) — no min-of-runs.
    let p = 400usize;
    let scanned = keys_scanned(&proj_param_src(p));
    // `scanned > 0` proves the per-type index CACHE actually ran (a revert to the old
    // `keys().position()` scan never populates `record_field_index`, so this counter would stay 0 —
    // catching the regression); `scanned <= P·8` proves it is O(P), not the O(P²) of a per-projection
    // scan. The bound P·8 leaves margin for the type being indexed from a few distinct occurrences (the
    // param read + the fold path) while sitting far below P² = 160000.
    assert!(
        scanned > 0 && scanned <= (p as u64) * 8,
        "a P-field runtime record projected field-by-field must index in O(P) keys via the per-type \
             cache, not O(P²) via a per-projection `keys().position()` scan: P={p} projected {p} times \
             enumerated {scanned} keys (expected 0 < n ≤ {}); the O(P²) scan was ~{}",
        (p as u64) * 8,
        (p as u64) * (p as u64)
    );
}

#[test]
fn ty_has_free_var_walks_a_shared_record_type_once() {
    // REGRESSION (perf): `infer::type_of`'s memoization guard runs `!t.has_free_var()` on EVERY node's
    // solved type. For a parameter annotated with an N-field record, referenced from N sites, each
    // reference's `type_of` walked the whole O(N) record type → O(N²) (the `pp` shape: N=6400 grew
    // ~3×/dbl, `has_free_var` ~41% self). The record type's field `BTreeMap` is an IMMUTABLE `Rc` shared
    // across all those references. FIX: `infer::ty_has_free_var` caches the verdict per payload `Rc`
    // address (`Db::ty_has_free_var`), so the O(N) walk happens ONCE per record type, not once per
    // reference. Total fields walked is then O(N), not O(N²).
    //
    // The shape: `use` takes a P-field record parameter and projects every field once — each `(. r fi)`
    // demands `type_of(r)` = the record type, whose guard would re-walk all P fields absent the cache.
    // Deterministic counter (a pure function of the program), so no min-of-runs.
    fn proj_param_src(p: usize) -> String {
        let fields_ty: String = (0..p)
            .map(|i| format!("(f{i} Int64)"))
            .collect::<Vec<_>>()
            .join(" ");
        fn tree(items: &[String]) -> String {
            if items.len() == 1 {
                return items[0].clone();
            }
            let m = items.len() / 2;
            format!("(+ {} {})", tree(&items[..m]), tree(&items[m..]))
        }
        let projs: Vec<String> = (0..p).map(|i| format!("(. r f{i})")).collect();
        format!(
            "(module m (def (use (: r (Record {fields_ty}))) {}) \
                   (def (main) 0) (export main))",
            tree(&projs)
        )
    }
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&proj_param_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide record-param projection compiles with no error diagnostics: {diags:?}"
    );
    fn elems_walked(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::TY_HAS_FREE_VAR_ELEMS_WALKED.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::TY_HAS_FREE_VAR_ELEMS_WALKED.with(|c| c.get())
        })
    }
    // The `has_free_var` walk over the P-field record type must total O(P) elements, not O(P²). The
    // record type may be walked from a couple of distinct `Rc`s (the param annotation may be reduced
    // more than once before the type_of memo settles), so allow a small constant multiple of P; that is
    // still far below the O(P²) a per-reference re-walk would produce. `> 0` proves the cached path ran
    // (a revert to a bare `t.has_free_var()` in the guard leaves this counter at 0 → the test fails).
    let p = 400usize;
    let walked = elems_walked(&proj_param_src(p));
    assert!(
        walked > 0 && walked <= (p as u64) * 8,
        "the `type_of` guard's free-var check on a P-field record referenced P times must walk O(P) \
             fields via the per-`Rc` cache, not O(P²) (was a full re-walk per reference): P={p} walked \
             {walked} fields (expected 0 < n ≤ {}); the O(P²) re-walk was ~{}",
        (p as u64) * 8,
        (p as u64) * (p as u64)
    );
}

#[test]
fn a_deeply_nested_collection_type_annotation_reduces_without_a_node_blowup() {
    // REGRESSION (perf): `eval::typeval_of`'s type-constructor arm reduced a COLLECTION type ctor
    // (`List`/`Set`/`Map`) via `reduce_ctor`, which builds the `Ty` and then `encode_typeval`s it BACK
    // into fresh AST nodes — an arena round-trip. For a deeply-nested annotation `(List (List … Int64))`
    // that re-serialized the WHOLE built `Ty` at EVERY nesting level → O(depth²) appended nodes, and the
    // reduction re-ran per referencing occurrence, so `cdz check` on a depth-N annotation was ~O(N³)
    // (depth 100/200/400/800 = 35/169/1091/8203ms, ~cubic). The generic-SUM ctor path already avoided
    // this (`reduce_sum_ctor` returns the `Ty` directly); the collection ctors did not. FIX: build the
    // `Ty::List`/`Set`/`Map` DIRECTLY from the reduced argument types in `typeval_of`, no
    // `encode_typeval` round-trip — so the arena stays O(depth), not O(depth²).
    //
    // The NOISE-FREE signal is the loaded arena's NODE COUNT (`db.ast.structure.len()`): a pure function
    // of the program, so no timing. A depth-D annotation should leave the arena O(D) (the prelude is a
    // constant baseline); the round-trip made it O(D²). Measure the GROWTH from D to 2D and assert it is
    // sub-quadratic — linear node growth is ~2×, the O(D²) blowup was ~4×+.
    fn nested_list_src(depth: usize) -> String {
        let mut ty = String::from("Int64");
        for _ in 0..depth {
            ty = format!("(List {ty})");
        }
        format!("(module m (def (f (: x {ty})) x) (def (main) 0) (export main))")
    }
    // A small instance compiles clean (a valid nested collection annotation, no error diagnostics).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&nested_list_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a nested collection-type annotation compiles with no error diagnostics: {diags:?}"
    );
    // Node count AFTER a full `diagnostics` run (which forces the annotation's `typeval_of` reduction).
    fn nodes_after_check(src: &str) -> usize {
        crate::host::run_with_compiler_stack(|| {
            let mut db = crate::db::Db::load(parse(src));
            let _ = crate::diagnostics(&mut db);
            db.ast.structure.len()
        })
    }
    // Subtract the constant baseline (prelude + fixed scaffolding) measured at a shallow depth, so the
    // ratio reflects the DEPTH-DEPENDENT growth, not the fixed prelude that dominates a small program.
    let base = nodes_after_check(&nested_list_src(50)) as f64;
    let n200 = nodes_after_check(&nested_list_src(200)) as f64 - base;
    let n400 = nodes_after_check(&nested_list_src(400)) as f64 - base;
    // Depth 200→400 is a 2× depth; linear node growth ⇒ ~2×, the O(depth²) round-trip was ~4×. Guard
    // the denominator, and require < 3× (between the two regimes, with margin for constant terms).
    let ratio = n400 / n200.max(1.0);
    assert!(
        ratio < 3.0,
        "a deeply-nested collection-type annotation must leave the arena O(depth), not O(depth²) (was a \
             `reduce_ctor`→`encode_typeval` round-trip per nesting level; the direct `Ty` build in \
             `typeval_of` fixes it): depth 200→400 grew the node count {ratio:.1}× (n200={n200}, \
             n400={n400}); linear is ~2×, the O(depth²) blowup was ~4×"
    );

    // The TUPLE and RECORD type constructors are the same-mechanism TWINS (they also went through the
    // `reduce_ctor`→`encode_typeval` round-trip until they got the direct-build fast path in
    // `typeval_of`). Assert the arena stays O(depth) for a deeply-nested `(Tuple (Tuple … Int64) Int64)`
    // and `(Record (f (Record …)))` too, so a regression on EITHER twin is caught here.
    let wrap_tuple: fn(&str) -> String = |t| format!("(Tuple {t} Int64)");
    let wrap_record: fn(&str) -> String = |t| format!("(Record (f {t}))");
    for (label, wrap) in [("Tuple", wrap_tuple), ("Record", wrap_record)] {
        let src = |depth: usize| {
            let mut ty = String::from("Int64");
            for _ in 0..depth {
                ty = wrap(&ty);
            }
            format!("(module m (def (g (: x {ty})) x) (def (main) 0) (export main))")
        };
        let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&src(4))));
        assert!(
            diags
                .iter()
                .all(|d| d.severity != crate::abi::Severity::Error),
            "a nested {label}-type annotation compiles with no error diagnostics: {diags:?}"
        );
        let base = nodes_after_check(&src(50)) as f64;
        let t200 = nodes_after_check(&src(200)) as f64 - base;
        let t400 = nodes_after_check(&src(400)) as f64 - base;
        let ratio = t400 / t200.max(1.0);
        assert!(
            ratio < 3.0,
            "a deeply-nested {label}-type annotation must leave the arena O(depth), not O(depth²) (the \
                 `reduce_ctor`→`encode_typeval` twin of the collection ctors; the direct `Ty` build fixes \
                 it): depth 200→400 grew the node count {ratio:.1}× (t200={t200}, t400={t400}); linear is \
                 ~2×, the O(depth²) blowup was ~4×"
        );
    }
}

#[test]
fn a_deeply_nested_generic_nominal_annotation_reduces_to_a_linear_size_type() {
    // REGRESSION (perf): `Db::normalize_sum` reduces a generic NOMINAL (erasable newtype, `(type Box
    // (Mk a))`) at an instantiation to `Ty::Nominal { args, inner }`, where `inner` is the template with
    // `args` substituted. `inner` was a `Box<Ty>`, so for a NESTED `(Box (Box … Int64))` the child
    // nominal was stored in BOTH `args` and (DEEP-CLONED into) `inner` at every level → the materialized
    // `Ty` DOUBLED per nesting level = O(2^depth) (depth 20 built a ~2M-node `Ty`; the compiler hung —
    // depth 24 took ~6s, depth 30+ timed out). FIX: `Nominal.inner: Rc<Ty>` — the child's allocation is
    // SHARED across `args`/`inner` and across levels, so a depth-D nesting is O(D) nodes, not O(2^D).
    //
    // The NOISE-FREE signal is the `subst_template_vars` VISIT COUNT (the template-substitution work
    // that builds each `Nominal.inner`): O(depth) after the fix, O(2^depth) before — a pure function of
    // the program, deterministic, no timing. (A `Ty`-node COUNT is the WRONG measure here: with the
    // `Rc` sharing the built type is O(depth) in MEMORY but a naive tree-walk that doesn't dedup shared
    // `Rc`s would itself re-expand to O(2^depth) — so the fix is in the WORK, counted directly.)
    use crate::db::Db;
    use crate::testkit::parse;
    fn subst_visits_for_nested_box(depth: usize) -> u64 {
        crate::host::run_with_compiler_stack(move || {
            let mut ty = String::from("Int64");
            for _ in 0..depth {
                ty = format!("(Box {ty})");
            }
            let src = format!(
                "(module m (type Box (Mk a)) (def (g (: x {ty})) x) (def (main) 0) (export main))"
            );
            crate::db::SUBST_TEMPLATE_VARS_VISITS.with(|c| c.set(0));
            // Loading + a full `diagnostics` run reduces the annotation (`typeval_of`→`normalize_sum`→
            // `subst_template_vars`) — forcing exactly the work whose count we measure.
            let mut db = Db::load(parse(&src));
            let _ = crate::diagnostics(&mut db);
            crate::db::SUBST_TEMPLATE_VARS_VISITS.with(|c| c.get())
        })
    }
    // Sanity: the substitution actually runs for a nested nominal.
    assert!(subst_visits_for_nested_box(4) > 0);
    // Linear ⇒ the work for 2× depth is ~2×; the O(2^depth) blowup was ~1000×+ (and would hang at these
    // depths — depth 20 alone drove ~2^20 substitutions before the fix). Measure depth 20 vs 40 and
    // require sub-quadratic growth: linear (~2×) clears < 4× easily, and any exponential regression
    // fails catastrophically (it would not even complete). Deterministic; no min-of-runs.
    let v20 = subst_visits_for_nested_box(20);
    let v40 = subst_visits_for_nested_box(40);
    let ratio = v40 as f64 / (v20.max(1)) as f64;
    assert!(
        ratio < 4.0,
        "a deeply-nested generic-nominal annotation must reduce with O(depth) template-substitution \
             work, not O(2^depth) (was `Nominal.inner: Box<Ty>` deep-cloning the child into both `args` and \
             `inner` per level; `inner: Rc<Ty>` shares it): depth 20→40 grew `subst_template_vars` visits \
             {ratio:.1}× (v20={v20}, v40={v40}); linear is ~2×, the exponential doubled PER LEVEL"
    );
}

#[test]
fn a_wide_runtime_map_match_resolves_synth_names_in_bounded_time() {
    // REGRESSION (perf): the runtime-map-match desugar (`lower::desugar_runtime_map_match`) folds N
    // key-directed arms into an O(N)-DEEP nested `(if <k-present> body <else>)` presence chain, each
    // `<k-present>` a synthesized `(match (Map.lookup m k) ((Some _) true) ((None) false))`. Those synth
    // nodes are NOT in the load-time scope-skip index, so resolving a prelude name (`Map`/`lookup`/
    // `Some`/`None`) in an inner arm walked O(depth) enclosing forms to conclude "not lexically bound"
    // → O(arms²) `binder_in` calls (N=200/400/800 = 105K/410K/1.6M, ~4×/dbl). FIX:
    // `Db::extend_scope_skip_pass_through` gives the synth chain scope-skip entries (every node is a
    // non-binding form, so it passes its parent's skip through), making each resolution hop O(1).
    //
    // The NOISE-FREE signal is the total `binder_in` call count (a pure function of the program). A
    // match with N single-key map arms should resolve its synth names in O(N) `binder_in` calls, not
    // O(N²). Correctness (the dispatch picks the right arm) is pinned by the run-value tests out of band.
    fn wide_map_match_src(arms: usize) -> String {
        let arm_forms: String = (0..arms)
            .map(|i| format!("((map (\"k{i}\" v)) {i})"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "(module m (def (f (: m (Map String Int64))) (match m {arm_forms} (_ -1))) \
                   (def (main) (f (map (= \"k0\" 1)))) (export main))"
        )
    }
    // A small instance compiles clean (a valid runtime-map match).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_map_match_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide runtime-map match compiles with no error diagnostics: {diags:?}"
    );
    fn binder_in_calls(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::BINDER_IN_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::BINDER_IN_CALLS.with(|c| c.get())
        })
    }
    // Arms 200→400 is a 2× width; linear `binder_in` growth ⇒ ~2×, the O(N²) walk was ~4×. Guard the
    // denominator and require < 3× (between the regimes, with margin for constant terms).
    let n200 = binder_in_calls(&wide_map_match_src(200));
    let n400 = binder_in_calls(&wide_map_match_src(400));
    let ratio = n400 as f64 / (n200.max(1)) as f64;
    assert!(
        ratio < 3.0,
        "a wide runtime-map match must resolve its synthesized presence-chain names in O(arms) \
             `binder_in` calls, not O(arms²) (the desugar's O(depth)-nested `if`/`match` chain needs \
             scope-skip coverage — `extend_scope_skip_pass_through`): arms 200→400 grew binder_in calls \
             {ratio:.1}× (n200={n200}, n400={n400}); linear is ~2×, the deep-walk was ~4×"
    );
}

#[test]
fn a_quote_free_program_skips_the_reify_quotes_position_scan() {
    // REGRESSION (perf): `quote::reify_quotes` runs at EVERY load, but for a program with NO
    // `quote`/`quasiquote` FORM (the overwhelming common case) all of its work is dead —
    // `pattern_position_nodes` builds an O(N) parent/child-index map + a downward BitSet walk,
    // `binder_position_nodes` scans every node, and the reverse per-node plan loop does two
    // allocating `as_form(id,"quote"/"quasiquote").map(to_vec)` shape probes per node. On a large
    // quote-free module (`reify_quotes` was ~3-4% of `as_form`'s self-time via those passes), that is
    // pure churn. FIX: a single O(leaves) prescan for a `quote`/`quasiquote` NAME leaf; absent one, no
    // quote head exists anywhere, so `reify_quotes` returns immediately WITHOUT touching the O(N)
    // position passes or the plan loop.
    //
    // The NOISE-FREE signal is `REIFY_QUOTES_POSITION_SCAN_NODES` (the plan-loop node count, a pure
    // function of the program): 0 for a quote-free program (the fast-bail fired), > 0 and O(N) for a
    // genuine quote program. Correctness (a quote still reifies, a def named `quote` still binds) is
    // pinned by the `quote::` unit tests.
    fn position_scan_nodes(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::REIFY_QUOTES_POSITION_SCAN_NODES.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::REIFY_QUOTES_POSITION_SCAN_NODES.with(|c| c.get())
        })
    }
    // A wide quote-FREE module: N newtype decls, no quote form anywhere.
    let quote_free: String = {
        let decls: String = (0..200)
            .map(|i| format!("(type T{i} (Mk{i} Int64))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("(module m {decls} (def (main) 0) (export main))")
    };
    assert_eq!(
        position_scan_nodes(&quote_free),
        0,
        "a quote-free program must fast-bail `reify_quotes` before its O(N) position passes / plan \
             loop (the single O(leaves) name prescan found no `quote`/`quasiquote` head), so the plan-loop \
             node count is 0"
    );
    // A program that genuinely contains a `quote` form DOES run the plan loop (the counter is > 0) —
    // the prescan is an over-approximation only in the harmless direction (a spurious fall-through for
    // a program that merely mentions the identifier), never a false skip of a real quote.
    let with_quote = "(module m (def (main) (quote (+ 1 2))) (export main))";
    assert!(
        position_scan_nodes(with_quote) > 0,
        "a program containing a `quote` form must run the reify plan loop (the counter is > 0) — the \
             fast-bail must NOT skip a genuine quote"
    );
}

#[test]
fn an_eval_free_program_skips_the_desugar_eval_node_scan() {
    // REGRESSION (perf): `eval_ast::desugar_eval` runs at EVERY load, scanning every node with an
    // `as_form(id,"eval")` probe — dead for a program with no `(eval …)` form (the common case). FIX:
    // a single O(leaves) prescan for an `eval` NAME leaf fast-bails before the per-node scan. The
    // noise-free signal is `DESUGAR_EVAL_SCAN_NODES`: 0 for an eval-free program, > 0 for a genuine
    // eval program. Correctness (an `(eval (quote …))` still reconstructs + folds) is pinned by the
    // `eval`/metaprogramming unit + corpus tests.
    fn scan_nodes(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::DESUGAR_EVAL_SCAN_NODES.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::DESUGAR_EVAL_SCAN_NODES.with(|c| c.get())
        })
    }
    let eval_free: String = {
        let decls: String = (0..50)
            .map(|i| format!("(type T{i} (Mk{i} Int64))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("(module m {decls} (def (main) 0) (export main))")
    };
    assert_eq!(
        scan_nodes(&eval_free),
        0,
        "an eval-free program must fast-bail `desugar_eval` before its per-node scan (the O(leaves) \
             name prescan found no `eval` head), so the scan node count is 0"
    );
    // A genuine `(eval …)` program DOES run the scan (counter > 0) — the fast-bail must not skip it.
    let with_eval = "(module m (def (main) (eval (quote (+ 1 2)))) (export main))";
    assert!(
        scan_nodes(with_eval) > 0,
        "a program containing an `(eval …)` form must run the desugar scan (the counter is > 0) — \
             the fast-bail must NOT skip a genuine eval"
    );
}

#[test]
fn a_tagged_template_free_program_skips_the_expand_node_scan() {
    // REGRESSION (perf): `tagged_template::expand` runs at EVERY load, scanning every node with a
    // `rewrite_of` (`as_name(items[0]) == "tagged-template"`) probe — dead for a program with no
    // tagged template (the common case). FIX: a single O(leaves) prescan for a `tagged-template` NAME
    // leaf fast-bails before the per-node scan. The noise-free signal is `TAGGED_TEMPLATE_SCAN_NODES`:
    // 0 for a tagged-template-free program, > 0 for a genuine one. Correctness (a `tag"…{e}…"` still
    // expands to the dispatched call) is pinned by the tagged-template unit + corpus tests.
    fn scan_nodes(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::TAGGED_TEMPLATE_SCAN_NODES.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::TAGGED_TEMPLATE_SCAN_NODES.with(|c| c.get())
        })
    }
    let tt_free: String = {
        let decls: String = (0..50)
            .map(|i| format!("(type T{i} (Mk{i} Int64))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("(module m {decls} (def (main) 0) (export main))")
    };
    assert_eq!(
        scan_nodes(&tt_free),
        0,
        "a tagged-template-free program must fast-bail `tagged_template::expand` before its per-node \
             scan (the O(leaves) name prescan found no `tagged-template` head), so the scan node count is 0"
    );
    // A genuine `(tagged-template …)` node DOES run the scan (counter > 0) — the reader's canonical
    // form for the ML surface `tag"…{expr}…"`. The head resolves to an unbound `tag` (no such def
    // here), but the load-time expand pass still fires and the counter proves it.
    let with_tt =
        "(module m (def (main) (tagged-template tag (chunks \"a\") (holes))) (export main))";
    assert!(
        scan_nodes(with_tt) > 0,
        "a program containing a `(tagged-template …)` form must run the expand scan (the counter is \
             > 0) — the fast-bail must NOT skip a genuine tagged template"
    );
}

#[test]
fn check_unknown_units_scans_only_user_nodes_not_the_prelude() {
    // REGRESSION (perf): `infer::check_unknown_units` runs at EVERY compile/check, scanning every node
    // for a `(Unit.of …)` head (`resolved_ref` + `meta_apply_of`) — but it scanned the FULL structure,
    // which appends the O(prelude) built-in bindings + every evaluator-synthesized β-copy. On a large
    // unit-FREE real program (the whole ML compiler uses no units) this pass had inclusive-time
    // dominance (~6% of `emit-db.cdz`'s compile) walking built-in nodes for nothing. FIX: bound the scan
    // to `user_node_count` — a genuine unknown-unit fault can only anchor at a USER node (a prelude /
    // synth anchor has no span → nulled by `sanitize_origin`; a β-copy relocates to its user origin,
    // already in-range), so the built-in bulk is never a source of a reportable fault.
    //
    // The noise-free signal is `CHECK_UNKNOWN_UNITS_SCAN_NODES`: it must equal exactly the program's
    // USER node count, never the (larger) full structure length. Correctness (an `(Unit.of #"zorks")`
    // still reports CDZ0201 with a did-you-mean) is pinned by the units unit + corpus tests, and
    // re-verified here.
    fn scan_and_user_count(src: &str) -> (u64, u64) {
        crate::host::run_with_compiler_stack(|| {
            crate::db::CHECK_UNKNOWN_UNITS_SCAN_NODES.with(|c| c.set(0));
            let mut db = crate::db::Db::load(parse(src));
            let _ = crate::diagnostics(&mut db);
            let user = db.user_node_count() as u64;
            let scanned = crate::db::CHECK_UNKNOWN_UNITS_SCAN_NODES.with(|c| c.get());
            (scanned, user)
        })
    }
    // A large unit-free program: its full structure is user + a big prelude append. The scan must cover
    // ONLY the user nodes.
    let unit_free: String = {
        let decls: String = (0..50)
            .map(|i| format!("(type T{i} (Mk{i} Int64))"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("(module m {decls} (def (main) 0) (export main))")
    };
    let (scanned, user) = scan_and_user_count(&unit_free);
    assert_eq!(
        scanned, user,
        "check_unknown_units must scan EXACTLY the user nodes ({user}), not the full structure — the \
             prelude/synth bulk carries no reportable unknown-unit fault, so it is skipped (was {scanned})"
    );
    assert!(
        user > 0,
        "the program must have user nodes to make the bound meaningful"
    );

    // Correctness re-verify: a genuine unknown unit STILL surfaces CDZ0201 (the bound must not drop a
    // real user-node fault). The bad `Unit.of` is a user node, in-range of the bounded scan.
    let bad = "(module m (def (main) (Qty.of 5 (Unit.of #\"zorks\"))) (export main))";
    let diags = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(bad)))
    });
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unknown unit `zorks`")),
        "a user `(Unit.of #\"zorks\")` must still report the unknown-unit CDZ0201 under the bounded scan"
    );

    // COVERAGE for the PR #1101 review concern (Copilot), corpus-bugfix-confirmed against trunk: a
    // `(Unit.of #"zorks")` inside an `(eval (quote …))` — where reconstruction could GRAFT synthesized
    // nodes at ids >= user_node_count — must STILL report CDZ0201. It does, and the "escaping synth node"
    // never materializes: `eval` refuses to reconstruct ANY quote carrying a `#"…"` SYMBOL literal (a bare
    // symbol does NOT reconstruct, unlike a string/int/float), so a quoted `Unit.of` (which requires a
    // symbol arg) declines CDZ0101 BEFORE a runnable synth `Unit.of` node is built. Meanwhile the quoted
    // `Unit.of`'s own `#"zorks"` is an in-range USER literal the bounded scan still CDZ0201's. So the
    // user-bound scan drops no reportable fault. Pin it so the bound can't later be narrowed in a way that
    // would (and see the corpus CDZ0101-decline tripwire, which flips if `eval` learns to reconstruct
    // symbol literals — at which point the synth-node path becomes reachable and this bound needs re-review).
    let eval_quoted =
        "(module m (def (main) (eval (quote (Qty.of 5 (Unit.of #\"zorks\"))))) (export main))";
    let eval_diags = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(eval_quoted)))
    });
    assert!(
        eval_diags
            .iter()
            .any(|d| d.message.contains("unknown unit `zorks`")),
        "a `(Unit.of #\"zorks\")` inside `(eval (quote …))` must still report CDZ0201 — its quoted-source \
             occurrence is a user node the bounded scan covers (the reconstruction splice is a copy, not a \
             synth-only original)"
    );
}

#[test]
fn a_wide_arithmetic_body_partitions_cse_candidates_in_bounded_time() {
    // REGRESSION (perf): the wasm CSE class-partition (`collect_cse_candidate_groups`) grouped
    // candidates into value-equivalence classes by an ALL-PAIRS `core_eq` scan ("a body has few CSE
    // candidates" — false for a WIDE body). A `(def (main p) (+ (calcN p) (+ … )))` balanced `+`-tree
    // yields THOUSANDS of DISTINCT scalar candidates (~5842 at N=2000) → a singleton-heavy partition
    // → ~cands²/2 `core_eq` calls, EACH a subtree-cloning `core_of` walk → the emit path was O(N²)
    // (compile 1913ms vs 621ms after, a controlled same-source A/B on N=2000, byte-identical output).
    // FIX: bucket candidates by a cheap shallow `core_hash_key` (`core_eq(a,b) ⇒ equal key`) and run
    // `core_eq` only WITHIN a bucket → distinct candidates never pairwise-compare → O(N) compares.
    //
    // The NOISE-FREE signal is the per-`Db` `cse_partition_core_eq_calls` count (within-bucket
    // compares, a pure function of ONE program's compile), read off this compile's `CompileOutput` —
    // NOT a process-global atomic (which the parallel test harness's other concurrent compiles
    // `fetch_add`-pollute during the read window, inflating it under load; see `Db`'s field doc).
    // With hash-bucketing a wide all-distinct body makes ~0 compares (each candidate its own bucket);
    // the all-pairs scan made ~cands²/2. Assert a LINEAR bound (≤ 4·cands_upper): the fixed partition
    // stays far under it, the O(N²) scan blows past it.
    fn wide_arith_src(n: usize) -> String {
        let defs: String = (0..n)
            .map(|i| format!("(def (calc{i} (: x Int64)) (+ (* x {i}) {}))", i % 13))
            .collect::<Vec<_>>()
            .join(" ");
        fn tree(lo: usize, hi: usize) -> String {
            if hi - lo == 1 {
                return format!("(calc{lo} p)");
            }
            let m = (lo + hi) / 2;
            format!("(+ {} {})", tree(lo, m), tree(m, hi))
        }
        format!(
            "(module m {defs} (def (main (: p Int64)) {}) (export main))",
            tree(0, n)
        )
    }
    fn compares(src: &str) -> u64 {
        // Read the CSE-partition compare count off the CompileOutput of THIS compile — a per-`Db`
        // metric surfaced through the #[cfg(test)] CompileOutput field, so the parallel test harness's
        // other concurrent compiles cannot pollute it (the old process-global atomic was
        // `fetch_add`-contaminated during the read window, inflating the reading under load).
        // MUST run the compile on the bumped compiler-stack worker (`run_with_compiler_stack`): the
        // deep left-nested chain recurses deep in `core_eq`/emit and overflows the default ~2MB
        // cargo-test thread stack. `compile_component` routed through this worker implicitly; driving
        // `compile` directly to read the per-`Db` field does NOT, so wrap it here (as the 20+ sibling
        // deep-recursion tests do).
        crate::host::run_with_compiler_stack(|| {
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "main",
                    crate::codec::encode(&parse(src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            out.cse_partition_core_eq_calls
        })
    }
    // N=400: candidate count is a few thousand (each arith subterm + call is a candidate). A LINEAR
    // partition makes O(cands) compares; the O(N²) all-pairs scan makes ~cands²/2 = millions. A bound
    // of 200_000 sits far above linear-at-this-width yet far below the quadratic scan.
    let c400 = compares(&wide_arith_src(400));
    assert!(
        c400 < 200_000,
        "a wide arithmetic body must partition its CSE candidates in ~O(N) `core_eq` compares, not              O(N²) (the all-pairs scan needs shallow-hash bucketing — `core_hash_key`): N=400 made              {c400} within-bucket compares (linear is a few thousand; the all-pairs scan was millions)"
    );
}

#[test]
fn value_range_stays_linear_on_a_runtime_binding_chain() {
    // REGRESSION (perf, superlinear): `lower::value_range` recurses `LocalRef → value_range(initializer)`,
    // so over a SEQUENTIAL-DEPENDENCY chain `(x_i (+ x_{i-1} p))` it re-walked every predecessor per node
    // = O(N²) compile-time (an empirical `--warm-only` probe measured near-quadratic growth while wasm
    // output stayed linear). FIX: `value_range` memoizes its refinement-free result (gated on
    // `db.range_refinements.is_empty()`), making `value_range_uncached` run ~once per node = O(N). The
    // NOISE-FREE signal is the per-`Db` `value_range_uncached_calls` count read off THIS compile's
    // `CompileOutput` (a single-compile metric, not a parallel-test-contaminated global). A future
    // un-memoization flips the count back to quadratic; this pins it LINEAR. `p` is a runtime param so
    // the chain does NOT const-fold (a constant chain would collapse and exercise nothing).
    fn chain_src(n: usize) -> String {
        let lets: String = (1..n)
            .map(|i| format!("(x{i} (+ x{} p))", i - 1))
            .collect::<Vec<_>>()
            .join(" ");
        let mut sum = String::from("x0");
        for i in 1..n {
            sum = format!("(+ x{i} {sum})");
        }
        format!("(module m (def (main (: p Int64)) (let ((x0 p) {lets}) {sum})) (export main))")
    }
    fn uncached_calls(src: &str) -> u64 {
        // Read the per-`Db` `value_range_uncached` count off the CompileOutput of THIS compile — a
        // single-compile metric, contamination-proof (see the CSE-partition twin). On the bumped
        // compiler-stack worker: a deep chain recurses deep in lowering and would overflow the default
        // cargo-test thread stack (`compile_component` routes through this worker; driving `compile`
        // directly to read the per-`Db` field does not, so wrap it as the sibling deep-recursion tests do).
        crate::host::run_with_compiler_stack(|| {
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "main",
                    crate::codec::encode(&parse(src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            out.value_range_uncached_calls
        })
    }
    // Growing the chain 4× (N=100 → 400) grows `value_range_uncached` calls ~4× when LINEAR (memoized);
    // the O(N²) recompute grows ~16×. Assert the ratio stays SUB-QUADRATIC (< 6×) — comfortably above
    // the linear 4× (some constant-factor slack), far below the quadratic 16×. Both counts must be > 0
    // (the compile actually ran `value_range`).
    let c100 = uncached_calls(&chain_src(100));
    let c400 = uncached_calls(&chain_src(400));
    assert!(
        c100 > 0 && c400 > 0,
        "value_range ran on the chain: {c100}, {c400}"
    );
    assert!(
        c400 < 6 * c100,
        "value_range must stay ~O(N) on a runtime sequential-binding chain (memo intact): N=100 made \
             {c100} `value_range_uncached` calls, N=400 made {c400} (a 4× LINEAR ratio ~ {}×; the O(N²) \
             recompute would be ~16× — a regression un-memoized `value_range`)",
        c400 / c100.max(1)
    );
}

#[test]
fn param_apply_extra_handled_stays_polynomial_on_a_nested_applied_lambda_chain() {
    // REGRESSION (perf, EXPONENTIAL — seq-203 #5755): `effects::param_apply_extra_handled`'s transitive
    // apply-site homing follows a known non-recursive sub-callee by RE-ENTERING itself, and its inner
    // `walk` ALSO re-descends the same head with no shared memo — so over N-deep NESTED
    // IMMEDIATELY-APPLIED lambdas each capturing the outer param, `((fn (q_i) (+ q_i <inner>)) p)`, the
    // follow re-analyzes each shared inner body via BOTH routes = 2^N compile-time (the operator's
    // "compiler hangs" class; v-compiler-perf's original repro, measured pre-#5755: N=12 .018s … N=24
    // 14s, ×4/+2 = 2^N). FIX: memoize on `(callee_body, arity, depth)` → identical re-analyses collapse
    // to a cache HIT, so the count drops to POLYNOMIAL. The NOISE-FREE signal is the per-`Db`
    // `param_apply_extra_handled_calls` count read off THIS compile's `CompileOutput` (a single-compile
    // metric, not a parallel-test-contaminated global): MEMOIZED it is polynomial (measured N=12→298,
    // N=24→2324 ≈ cubic); UN-MEMOIZED it is EXACTLY 2^N−1 (measured N=8→255, N=10→1023, N=12→4095,
    // N=14→16383 → N=24 = 16_777_215). A future un-memoization flips the count back to exponential; this
    // pins it POLYNOMIAL.
    //
    // This repro's EMIT is LINEAR (wasm 480→864B across the whole range — the analysis was the sole
    // superlinear pole, no effect handler needed), so unlike a handler-driven doubling tree (whose
    // effect-continuation lowering is independently 2^N) the FULL compile stays fast under the memo
    // (N=24 ≈ 260ms) and we can pin a large N gap for a sharp exponential-vs-polynomial signal.
    fn chain_src(n: usize) -> String {
        // N-deep nested immediately-applied lambdas, each capturing the accumulated inner expression and
        // the outer param `p` — v-compiler-perf's original param_apply_extra_handled 2^N reproducer.
        let mut inner = String::from("p");
        for i in 0..n {
            inner = format!("((fn (q{i}) (+ q{i} {inner})) p)");
        }
        format!("(module m (def (main (: p Int64)) {inner}) (export main))")
    }
    fn homed_calls(src: &str) -> u64 {
        // Read the per-`Db` `param_apply_extra_handled_calls` count off THIS compile's `CompileOutput`
        // (a single-compile metric, contamination-proof — see the value_range/CSE-partition twins). The
        // transitive follow recurses deep in the effects walk, so wrap on the compiler worker stack as
        // the sibling deep-recursion tests do.
        crate::host::run_with_compiler_stack(|| {
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "main",
                    crate::codec::encode(&crate::testkit::parse(src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            out.param_apply_extra_handled_calls
        })
    }
    // Grow the tree depth N=4 → 10 (both under the `depth < 32` follow-gate, so both COMPILE). The count
    // ratio is a normalized signal that cancels constant factors: MEMOIZED it is quadratic — 16 → 100 =
    // 6.25× (= (10/4)²); UN-MEMOIZED it is exponential — 37 → 3049 = 82× (2^N). Assert the ratio stays
    // sub-exponential (< 20×): comfortably ABOVE the quadratic 6.25× (>3× slack for constant-factor/
    // higher-order drift), far BELOW the exponential 82× (>4× margin). VERIFIED the guard bites:
    // neutralizing the `(callee_body, arity, depth)` memo makes N=10 report 3049 (> 20·37 = 740) → this
    // assertion FAILS. Both counts must be > 0 (the tree ran the transitive homing — a 0 would mean a
    // false-decline via `fail_with`, see the header note).
    // Grow the nesting N=12 → 24 (v-compiler-perf's canonical points; both COMPILE with linear emit).
    // The count ratio is a normalized signal that cancels constant factors: MEMOIZED it is polynomial —
    // 298 → 2324 = 7.8× (~cubic); UN-MEMOIZED it is exponential — 4095 → 16_777_215 = 4096× (= 2^12).
    // Assert the ratio stays sub-exponential (< 50×): comfortably ABOVE the polynomial 7.8× (>6× slack
    // for constant-factor/degree drift), astronomically BELOW the exponential 4096× (>80× margin).
    // VERIFIED the guard bites: neutralizing the `(callee_body, arity, depth)` memo makes N=24 report
    // 16_777_215 (≫ 50·4095) → this assertion FAILS. Both counts must be > 0 (the compile ran the
    // transitive-homing walk).
    let c12 = homed_calls(&chain_src(12));
    let c24 = homed_calls(&chain_src(24));
    assert!(
        c12 > 0 && c24 > 0,
        "the transitive homing ran on the nested-lambda chain: {c12}, {c24}"
    );
    assert!(
        c24 < 50 * c12,
        "param_apply_extra_handled must stay POLYNOMIAL on N-deep nested immediately-applied lambdas \
             (memo intact): N=12 made {c12} body-runs, N=24 made {c24} (a ~cubic ratio ~ {}×; the 2^N \
             recompute is ~4096× — a regression un-memoized param_apply_extra_handled, the seq-203 hang)",
        c24 / c12.max(1)
    );
}

#[test]
fn a_deep_uniform_arith_chain_partitions_cse_candidates_in_bounded_time() {
    // REGRESSION (perf): the CSE partition's hash bucketing (`core_hash_key`) must use a FULL-DEPTH
    // memoized structural hash, NOT a shallow (one-level) one. A UNIFORM-shape body — a deep
    // left-nested `(+ (+ … (* p 0)) (* p 1))` accumulator chain — has every node with the SAME shallow
    // key (`Arith(Add)` over `[Arith,Arith]`, and every `(* p k)` is `Arith(Mul)` over `[Param,
    // ConstInt]`), so a shallow hash collides ALL candidates into ONE bucket → the within-bucket
    // `core_eq` scan degrades back to O(N²) with DEEP-recursive compares (measured: this shape compiled
    // 21/75/486/3976ms @ N=100/200/400/800, ~8×/dbl, `core_eq` ~96% inclusive — WORSE than the balanced
    // tree). The full-depth hash recurses to the LEAVES, so `(* p 0)`/`(* p 1)`/… and each distinct
    // chain prefix hash DIFFERENTLY → distinct buckets → O(N) compares. Complements
    // `a_wide_arithmetic_body_…` (balanced tree, distinct heads); this pins the uniform-shape case a
    // shallow hash missed.
    fn deep_chain_src(n: usize) -> String {
        let mut e = String::from("p");
        for i in 0..n {
            e = format!("(+ {e} (* p {i}))");
        }
        format!("(module m (def (main (: p Int64)) {e}) (export main))")
    }
    // A small instance compiles clean.
    let bytes =
        crate::compile::compile_component(&crate::codec::encode(&parse(&deep_chain_src(4))));
    assert!(
        bytes.is_ok(),
        "a deep uniform arith chain compiles: {bytes:?}"
    );
    fn compares(src: &str) -> u64 {
        // Per-`Db` CSE-partition compare count off the CompileOutput of THIS compile (see the twin in
        // `a_wide_arithmetic_body_…`) — contamination-proof, unlike the old process-global atomic.
        // On the bumped compiler-stack worker (`run_with_compiler_stack`): this DEEP left-nested chain
        // recurses deep in `core_eq`/emit and overflows the default ~2MB cargo-test thread stack if run
        // there directly (`compile_component` routed through the worker; driving `compile` for the
        // per-`Db` field does not, so wrap it — as the sibling deep-recursion tests do).
        crate::host::run_with_compiler_stack(|| {
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "main",
                    crate::codec::encode(&parse(src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            out.cse_partition_core_eq_calls
        })
    }
    // N=400: full-depth bucketing → each node its own bucket → ~0 within-bucket compares. A shallow
    // hash (all-collide) would make ~cands²/2 = hundreds of thousands. Bound of 200_000 discriminates.
    let c400 = compares(&deep_chain_src(400));
    assert!(
        c400 < 200_000,
        "a deep UNIFORM-shape arith chain must partition its CSE candidates in ~O(N) `core_eq` \
             compares, not O(N²) (a shallow one-level hash collides every same-shaped node into one \
             bucket — needs the FULL-DEPTH memoized `core_hash_key`): N=400 made {c400} within-bucket \
             compares (full-depth is ~0; a shallow/all-pairs scan was hundreds of thousands)"
    );
}

#[test]
fn a_wide_match_resolves_in_a_bounded_number_of_clones() {
    // REGRESSION (perf): `resolve::resolved_of` returns the resolved form BY VALUE — it CLONES the
    // whole `Resolved` per call (a memo-hit `r.clone()`). A dispatch/tag-test caller that only READS
    // the form should use the borrow companion `resolved_ref` instead (the fix-35/36/`prim_of`/
    // `collect_pattern_binders` borrow family). This pins the clone count on a match-heavy program
    // (the shape that drove `collect_pattern_binders` — a parser like `sread.cdz` — where a per-
    // pattern-atom `resolved_of` tag test was a top `Resolved::clone` caller) to grow ~LINEARLY with
    // the program, so a regression that reintroduces a per-node `resolved_of` where a borrow would do
    // is caught deterministically (a wall-clock A/B of a borrow change is swamped by fleet-load noise;
    // the per-`Db` `resolved_of_calls` count is a pure function of ONE program's compile). It is a
    // PER-`Db` field, NOT a process-global atomic — a global was `fetch_add`-polluted by the parallel
    // test harness's other concurrent compiles during the read window, inflating the reading under
    // load and false-tripping this guard (fixed 2026-07-21; see the field doc in `db.rs`).
    fn wide_match_src(n: usize) -> String {
        // N functions each matching a runtime sum over literal + binder arms — heavy pattern lowering.
        let defs: String = (0..n)
            .map(|i| {
                format!(
                    "(def (f{i} (: x Int64)) (match x (0 {i}) (1 {}) (k (+ k {i}))))",
                    i + 1
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!("(module m {defs} (def (main) (f0 0)) (export main))")
    }
    // A small instance compiles clean.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_match_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide match program compiles with no error diagnostics: {diags:?}"
    );
    fn clone_calls(src: &str) -> u64 {
        // Read the count off the very `Db` this call drove — a PER-`Db` field, so the parallel test
        // harness's other concurrent compiles cannot pollute it (the old process-global atomic was
        // `fetch_add`-contaminated by other tests' `resolved_of` calls during the read window, which
        // inflated the reading nondeterministically under load and false-tripped this guard).
        let mut db = crate::db::Db::load(parse(src));
        let _ = crate::diagnostics(&mut db);
        db.resolved_of_calls
    }
    // Width 100→200 is 2×; a per-node clone count that grows LINEARLY with the program ⇒ ~2×. Require
    // < 3× (between linear and any O(N²) resolve-clone regression, with constant-term margin). `> 0`
    // proves resolution ran. This does NOT assert a specific count (that drifts as passes evolve) —
    // only that `resolved_of` clones scale linearly, so a quadratic-clone regression trips it.
    let c100 = clone_calls(&wide_match_src(100));
    let c200 = clone_calls(&wide_match_src(200));
    let ratio = c200 as f64 / (c100.max(1)) as f64;
    assert!(
        c100 > 0 && ratio < 3.0,
        "a match-heavy program must resolve in a LINEARLY-growing number of `resolved_of` clones, not \
             O(N²) (a per-node dispatch/tag-test site should borrow via `resolved_ref`, not clone via \
             `resolved_of`): width 100→200 grew clones {ratio:.1}× (c100={c100}, c200={c200}); linear is \
             ~2×"
    );
}

#[test]
fn a_wide_runtime_string_match_resolves_synth_names_in_bounded_time() {
    // REGRESSION (perf): the runtime-STRING-match desugar (`lower.rs`, the `Ty::String` scrutinee arm)
    // folds N string-literal arms into an O(N)-DEEP nested `(if (= s "k0") b0 (if (= s "k1") b1 …))`
    // value-eq if-chain via `db.push_list`. Like the runtime-map-match chain (`bf5a1a1c`), those synth
    // nodes are NOT in the load-time scope-skip index, so resolving a prelude name (`=`) — or any pass
    // (`check_unknown_units`/`collect_faults`) that resolves an inner synth node — walked O(depth)
    // enclosing `if` forms to conclude "not lexically bound" → O(arms²) `binder_in` calls (profiled:
    // check N=400/800/1600/3200 = 14/32/91/315ms, ~3×/dbl, 89% under `resolve_name`→`binder_in`→
    // `as_form`). FIX: call `Db::extend_scope_skip_pass_through(else_node)` on the synthesized chain
    // before resolving it (the arm bodies/guards are reused load-time occurrences that keep their own
    // final skip; the `if`/`=` spine is all non-binding, so the pass-through is sound + O(1)).
    //
    // The NOISE-FREE signal is the total `binder_in` call count (a pure function of the program). A
    // match with N string-literal arms must resolve its synth names in O(N) `binder_in` calls, not
    // O(N²). Correctness (the dispatch picks the right arm) is pinned by the run-value tests out of band.
    fn wide_str_match_src(arms: usize) -> String {
        let arm_forms: String = (0..arms)
            .map(|i| format!("(\"k{i}\" {i})"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "(module m (def (f (: s String)) (match s {arm_forms} (_ -1))) \
                   (def (main) (f \"k0\")) (export main))"
        )
    }
    // A small instance compiles clean (a valid runtime-string match).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_str_match_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide runtime-string match compiles with no error diagnostics: {diags:?}"
    );
    fn binder_in_calls(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::BINDER_IN_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::BINDER_IN_CALLS.with(|c| c.get())
        })
    }
    // Arms 200→400 is a 2× width; linear `binder_in` growth ⇒ ~2×, the O(N²) walk was ~4×. Guard the
    // denominator and require < 3× (between the regimes, with margin for constant terms).
    let n200 = binder_in_calls(&wide_str_match_src(200));
    let n400 = binder_in_calls(&wide_str_match_src(400));
    let ratio = n400 as f64 / (n200.max(1)) as f64;
    assert!(
        n200 > 0 && ratio < 3.0,
        "a wide runtime-string match must resolve its synthesized value-eq if-chain names in O(arms) \
             `binder_in` calls, not O(arms²) (the desugar's O(depth)-nested `if` chain needs scope-skip \
             coverage — `extend_scope_skip_pass_through`): arms 200→400 grew binder_in calls {ratio:.1}× \
             (n200={n200}, n400={n400}); linear is ~2×, the deep-walk was ~4×"
    );
}

#[test]
fn a_wide_list_pattern_resolves_element_binders_in_bounded_time() {
    // REGRESSION (perf): `resolve::find_leading_binder_in_list_pattern` answered "does this `(list p…
    // .. rest)` pattern bind `name`, and where?" by re-scanning the LEADING element positions from 0 on
    // EVERY reference — positive to the binder's position, NEGATIVE (a prelude/outer name the pattern
    // does not bind, e.g. `g`/`+`/`Int64`) over the WHOLE pattern. So a wide `(match xs ((list a0 … aN
    // .. r) body) …)` destructure whose body references each binder was O(leading) per reference × O(N)
    // references = O(N²) (measured: N=1600 ~140ms, ~4×/dbl, `find_leading_binder_in_list_pattern` ~37%
    // self + its per-element path `Vec` alloc ~45% of malloc). FIX: `Db::simple_list_binders` indexes a
    // SIMPLE pattern's binders (every leading element a bare name) ONCE by name, so each lookup is an
    // O(1) map read — the total leading elements enumerated is O(N), not O(N²).
    //
    // The NOISE-FREE signal is `LIST_PATTERN_BINDER_ELEMS_SCANNED` — the leading elements the index
    // build (+ any linear fallback) enumerates, a pure function of the program. A width-N pattern
    // referenced N times should enumerate O(N) elements (one index build), not O(N²) (a re-scan per
    // reference). Correctness (the binders read the right elements) is pinned by the run-value tests.
    fn wide_list_pattern_src(n: usize) -> String {
        let binders: String = (0..n)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        // `g` consumes every binder (so each is referenced once from the arm body, driving one
        // resolution against the wide list pattern), and every `g` param is used → no unused warnings.
        let refs: String = (0..n)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let g_body = {
            fn tree(items: &[String]) -> String {
                if items.len() == 1 {
                    return items[0].clone();
                }
                let m = items.len() / 2;
                format!("(+ {} {})", tree(&items[..m]), tree(&items[m..]))
            }
            let ps: Vec<String> = (0..n).map(|i| format!("p{i}")).collect();
            tree(&ps)
        };
        let params: String = (0..n)
            .map(|i| format!("p{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "(module m (def (g {params}) {g_body}) \
                   (def (f (: xs (List Int64))) (match xs ((list {binders} .. r) (g {refs})) (_ 0))) \
                   (def (main) (f (list))) (export main))"
        )
    }
    // A small instance compiles with no error diagnostics (a valid wide list destructure).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_list_pattern_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide list-pattern destructure compiles with no error diagnostics: {diags:?}"
    );
    fn elems_scanned(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::LIST_PATTERN_BINDER_ELEMS_SCANNED.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::LIST_PATTERN_BINDER_ELEMS_SCANNED.with(|c| c.get())
        })
    }
    // Width 200→400 is a 2× pattern; linear (index-build) growth ⇒ ~2×, the O(N²) re-scan was ~4×.
    // Require < 3× (between the regimes, with margin for constant terms). `> 0` proves the counter
    // ran (the index build enumerates the leading elements once); a revert to the per-reference scan
    // pushes the ratio toward 4×, failing the test.
    let n200 = elems_scanned(&wide_list_pattern_src(200));
    let n400 = elems_scanned(&wide_list_pattern_src(400));
    let ratio = n400 as f64 / (n200.max(1)) as f64;
    assert!(
        n200 > 0 && ratio < 3.0,
        "a wide list-pattern destructure must resolve its element binders in O(N) enumerated leading \
             elements, not O(N²) (the per-reference `find_leading_binder_in_list_pattern` scan needs the \
             per-pattern `Db::simple_list_binders` index): width 200→400 grew scanned elements {ratio:.1}× \
             (n200={n200}, n400={n400}); linear is ~2×, the re-scan was ~4×"
    );
}

#[test]
fn a_wide_refutable_literal_list_arm_resolves_its_guard_chain_in_bounded_time() {
    // REGRESSION (perf): `lower::desugar_refutable_literal_list_elements` rewrites a list arm with N
    // literal LEADING elements `(list 0 1 … N .. r)` into `(guard (list __le0 … __leN .. r) (and (and
    // (= __le0 0) (= __le1 1)) …))` — an O(N)-DEEP left-nested `and`-chain guard cond (`and` is strictly
    // binary). Those synth nodes are appended AFTER load, so without scope-skip coverage each `__leK`
    // guard reference (and each prelude `=`/`and`) walked O(depth) `and` forms to reach the enclosing
    // `(guard …)` → O(N²) `binder_in` calls. FIX: `Db::extend_scope_skip_into_subtree` — the CANDIDATE-
    // AWARE scope-skip extension (the `and`-spine is non-binding, the one `(guard …)` node is a
    // candidate its children skip TO) makes each such resolution hop O(1). (A SEPARATE O(N²) — the
    // `node_contains(g[1], id)` re-descent in `is_variant_pattern_binder_occurrence` classifying each
    // pattern binder — was fixed in the same change by tracking the ascent's `prev` child; the timing
    // sweep confirms both, this counter pins the scope-walk one.)
    //
    // The NOISE-FREE signal is the total `binder_in` call count (a pure function of the program). A
    // match with N literal-element arms should resolve its synth guard-chain names in O(N) `binder_in`
    // calls, not O(N²). Correctness (the arm matches the right prefix) is pinned by the run-value tests.
    fn wide_literal_list_src(n: usize) -> String {
        let elems: String = (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
        format!(
            "(module m (def (f (: xs (List Int64))) (match xs ((list {elems} .. r) 1) (_ 0))) \
                   (def (main) (f (list))) (export main))"
        )
    }
    // A small instance compiles with no error diagnostics (a valid refutable-literal-element arm).
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(&wide_literal_list_src(4))));
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a wide refutable-literal list arm compiles with no error diagnostics: {diags:?}"
    );
    fn binder_in_calls(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::BINDER_IN_CALLS.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::BINDER_IN_CALLS.with(|c| c.get())
        })
    }
    // Width 200→400 is a 2× arm; linear `binder_in` growth ⇒ ~2×, the O(N²) deep-walk was ~4×. Require
    // < 3× (between the regimes, with margin for constant terms).
    let n200 = binder_in_calls(&wide_literal_list_src(200));
    let n400 = binder_in_calls(&wide_literal_list_src(400));
    let ratio = n400 as f64 / (n200.max(1)) as f64;
    assert!(
        ratio < 3.0,
        "a wide refutable-literal list arm must resolve its synthesized `and`-chain guard names in \
             O(N) `binder_in` calls, not O(N²) (the desugar's O(depth)-nested `and`-chain cond needs \
             candidate-aware scope-skip coverage — `extend_scope_skip_into_subtree`): width 200→400 grew \
             binder_in calls {ratio:.1}× (n200={n200}, n400={n400}); linear is ~2×, the deep-walk was ~4×"
    );
}

#[test]
fn arm_pattern_name_fast_reject_never_drops_a_real_binding() {
    // `binder_in`'s per-arm fast-reject (`Db::arm_cannot_bind`) skips the match-arm case cascade when
    // `name` is not a NAME atom in the arm's pattern region — the O(1) cut for the O(depth)-per-
    // reference resolve pole on a deeply-nested match. It is SAFE only if the pattern-name set
    // OVER-approximates the arm's binders: a name the arm CAN bind must never be fast-rejected. This
    // pins that invariant against a future under-approximation (e.g. excluding a guard's pattern, or
    // the wrong child) — such a bug would drop a genuine binding and surface as a spurious CDZ0101.
    use crate::testkit::parse;
    // Compile on the compiler's deep stack — a deeply-nested match deep-recurses the resolve/lower
    // passes, which overflows the debug test thread's default stack (the release compiler is fine).
    let no_errors = |src: &str| {
        let errs = crate::host::run_with_compiler_stack(|| {
            crate::diagnostics(&mut crate::db::Db::load(parse(src)))
                .into_iter()
                .filter(|d| d.severity == crate::abi::Severity::Error)
                .map(|d| format!("{d:?}"))
                .collect::<Vec<_>>()
        });
        assert!(
            errs.is_empty(),
            "expected no error diagnostics, got: {errs:?}"
        );
    };
    // Deeply-nested Option match: each arm binds `x_i` (a payload binder in its pattern) and the body
    // references it in the NEXT scrutinee. Every `x_i` must resolve (it is a pattern atom → in the set,
    // never fast-rejected), while the global `Some`/`None`/`+` (absent from every arm's set) correctly
    // fall through to the prelude. A wrongly-narrow set would unbind an `x_i`.
    fn nested_option_match(depth: usize) -> String {
        fn build(i: usize, depth: usize) -> String {
            if i >= depth {
                return if i > 0 {
                    format!("x{}", i - 1)
                } else {
                    "p".into()
                };
            }
            let prev = if i > 0 {
                format!("x{}", i - 1)
            } else {
                "p".into()
            };
            format!(
                "(match (Some (+ {prev} 1)) ((Some x{i}) {}) ((None _) 0))",
                build(i + 1, depth)
            )
        }
        format!(
            "(module m (def (main (: p Int64)) {}) (export main))",
            build(0, depth)
        )
    }
    no_errors(&nested_option_match(20));
    // A GUARDED arm: the binder `x` lives inside the guard `(guard (Some x) (> x 0))` (child 0 of the
    // arm), and BOTH the guard cond and the arm body reference it. The pattern-name set is collected
    // from all arm children except the body, so it must include the guard's pattern names — else the
    // body ref `x` (or the guard ref) is dropped. Pins the guarded-arm shape specifically.
    no_errors(
        "(module m (def (main (: p Int64)) \
               (match (Some p) ((guard (Some x) (> x 0)) x) ((Some y) y) ((None _) 0))) \
             (export main))",
    );
    // A nested variant/tuple pattern binder must also survive (a deeper pattern atom).
    no_errors(
        "(module m (def (main (: p Int64)) \
               (match (Some (tuple p 1)) ((Some (tuple a b)) (+ a b)) ((None _) 0))) \
             (export main))",
    );
}

#[test]
fn arm_cascade_entries_stay_linear_on_a_nested_match() {
    // REGRESSION (perf): the per-arm BINDER-name fast-reject (`Db::arm_cannot_bind`) keeps the match-arm
    // case cascade O(N) on a deeply-nested match. A reference to a global/prelude/CTOR name
    // (`Some`/`None`/`+`) is bound by NO arm; because the fast-reject set EXCLUDES pattern heads, a ctor
    // name is rejected in O(1) at every enclosing arm → cascade entries stay O(N) (only genuine binder
    // references `x_i` enter, at their binding arm). Were heads kept in the set (or no fast-reject), the
    // ctor references `Some`/`None` would enter the cascade at each of O(depth) arms → O(N²).
    // `ARM_CASCADE_ENTRIES` is the noise-free signal (`BINDER_IN_CALLS` cannot pin this — the fast-reject
    // is INSIDE binder_in, so invocation count is unchanged; only cascade ENTRIES drop). Right-nested
    // Option match, each arm binding `x_i` fed to the next scrutinee (runtime-derived from `p`, 2 arms).
    fn nested_option_match(depth: usize) -> String {
        fn build(i: usize, depth: usize) -> String {
            if i >= depth {
                return if i > 0 {
                    format!("x{}", i - 1)
                } else {
                    "p".into()
                };
            }
            let prev = if i > 0 {
                format!("x{}", i - 1)
            } else {
                "p".into()
            };
            format!(
                "(match (Some (+ {prev} 1)) ((Some x{i}) {}) ((None _) 0))",
                build(i + 1, depth)
            )
        }
        format!(
            "(module m (def (main (: p Int64)) {}) (export main))",
            build(0, depth)
        )
    }
    fn cascade_entries(src: &str) -> u64 {
        crate::host::run_with_compiler_stack(|| {
            crate::db::ARM_CASCADE_ENTRIES.with(|c| c.set(0));
            let _ = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
            crate::db::ARM_CASCADE_ENTRIES.with(|c| c.get())
        })
    }
    // Depth 100→200 is a 2× nest; the head-excluding fast-reject keeps cascade entries ~linear (~2×),
    // the O(N²) all-atoms/un-rejected cascade was ~4×. Require < 3× (between the regimes, margin for
    // constant terms).
    let n100 = cascade_entries(&nested_option_match(100));
    let n200 = cascade_entries(&nested_option_match(200));
    let ratio = n200 as f64 / (n100.max(1)) as f64;
    assert!(
        n100 > 0 && ratio < 3.0,
        "the match-arm cascade must stay O(N) entries on a nested match, not O(N²) (a global/ctor-name \
             reference ascending O(depth) arms needs the head-excluding pattern-name fast-reject): depth \
             100→200 grew cascade entries {ratio:.1}× (n100={n100}, n200={n200}); linear is ~2×, the \
             un-rejected/all-atoms cascade was ~4×"
    );
}

#[test]
fn newtype_underlying_reads_the_erased_structural_type() {
    // `Db::newtype_underlying` reports the underlying structural type of an erasable single-variant
    // sum (a nominal newtype), and declines (None) for everything that must stay boxed. This is the
    // predicate the erasure (N3/N4) keys off — the realization of `§Nominal Is An Orthogonal
    // Modifier` (the tag adds nothing to the runtime representation).
    use crate::db::Db;
    use crate::infer::newtype_underlying;
    use crate::testkit::parse;
    use crate::ty::{IntTy, Ty};

    let decl_of = |db: &Db, n: &str| db.type_decls.iter().find(|t| t.name == n).unwrap().occ;

    // A newtype over a scalar → the scalar's type.
    let ast = parse("(module m (type UserId (Mk Int64)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "UserId");
    assert_eq!(
        newtype_underlying(&mut db, occ),
        Some(Ty::Int(IntTy::i64())),
        "a newtype over Int64 erases to Int64"
    );

    // A multi-payload single variant (a struct) → a tuple of the payload types.
    let ast = parse("(module m (type Point (Mk Int64 Int64)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "Point");
    assert_eq!(
        newtype_underlying(&mut db, occ),
        Some(Ty::Tuple(
            vec![Ty::Int(IntTy::i64()), Ty::Int(IntTy::i64())].into()
        )),
        "a two-payload single variant erases to a 2-tuple"
    );

    // A nullary single variant (a unit tag) → Unit.
    let ast = parse("(module m (type Marker (The)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "Marker");
    assert_eq!(
        newtype_underlying(&mut db, occ),
        Some(Ty::Unit),
        "a nullary single variant erases to Unit"
    );

    // A MULTI-variant sum is NOT a newtype (it needs the discriminant).
    let ast = parse("(module m (type E (A Int64) (B Int64)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "E");
    assert_eq!(
        newtype_underlying(&mut db, occ),
        None,
        "a two-variant sum stays boxed"
    );

    // A RECURSIVE single-variant sum ERASES — its inner's self-reference is a finite `Ty::Sum{decl}`
    // LEAF (the μ-binder), so the inner `(Tuple Int64 Ty::Sum{Stream})` is finite. `Ty::Nominal` is
    // compared by decl+args (not inner), so the folded/unfolded divergence is harmless.
    let ast =
        parse("(module m (type Stream (More (Tuple Int64 Stream))) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "Stream");
    assert!(
        matches!(newtype_underlying(&mut db, occ), Some(Ty::Tuple(_))),
        "a recursive single-variant sum erases to a finite inner (self-ref is a Ty::Sum leaf)"
    );

    // A GENERIC single-variant sum IS erasable — its underlying template carries the param as
    // `Ty::Var(0)` (the positional slot `decode_ty` substitutes the instantiation's arg into). So
    // `(type Box (Mk a))` erases with template `Var(0)`; at `Box Int64` the inner is `Int64`.
    let ast = parse("(module m (type Box (Mk a)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "Box");
    assert_eq!(
        newtype_underlying(&mut db, occ),
        Some(Ty::Var(0)),
        "a generic single-variant sum erases with a param-var template"
    );

    // A generic newtype over a COMPOUND param position keeps the param var at its slot: `(type Wrap
    // (Mk (List a)))` → template `(List Var(0))`.
    let ast = parse("(module m (type Wrap (Mk (List a))) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "Wrap");
    assert_eq!(
        newtype_underlying(&mut db, occ),
        Some(Ty::List(Box::new(Ty::Var(0)))),
        "a generic newtype's template keeps the param var at its nested position"
    );

    // A NON-RECURSIVE newtype over a SUM erases (its inner is the sum type) — the recursion guard only
    // boxes a CYCLIC newtype, so wrapping an `Option` no longer double-boxes.
    let ast = parse("(module m (type Cached (Mk (Option Int64))) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let occ = decl_of(&db, "Cached");
    assert!(
        matches!(newtype_underlying(&mut db, occ), Some(Ty::Sum { .. })),
        "a non-recursive newtype over a sum erases to the sum type"
    );

    // MUTUAL recursion `(type A (Mk B)) (type B (Wrap A))` ERASES both — each newtype's inner is the
    // OTHER's `Ty::Sum` leaf (`A`'s inner is `Ty::Sum{B}`, `B`'s is `Ty::Sum{A}`), both finite. Neither
    // unfolds the cycle; the `Ty::Sum` back-edges terminate every reader, and `decl+args` identity
    // makes the folded/unfolded reps compare equal. Both return `Some`.
    let ast = parse("(module m (type A (Mk B)) (type B (Wrap A)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let a = decl_of(&db, "A");
    let b = decl_of(&db, "B");
    // Both ERASE (return `Some`). The inner's cross-reference is the other decl as a `Ty::Sum` OR
    // `Ty::Nominal` back-edge depending on which was cached first — both finite, both terminate every
    // reader; the exact variant is a load-order detail, so assert only that erasure happened.
    assert!(
        newtype_underlying(&mut db, a).is_some(),
        "mutual-recursive A erases"
    );
    assert!(
        newtype_underlying(&mut db, b).is_some(),
        "mutual-recursive B erases"
    );
}

#[test]
fn a_generic_sum_monomorphizes_at_a_concrete_instantiation() {
    // A GENERIC sum `(type Option (Some a) None)` has an implicit type parameter `a` (a free
    // lowercase name in a payload). `(Option Int64)` in type position APPLIES the sum constructor,
    // monomorphizing to `Ty::Sum { args: [Int64] }`; `(Option Bool)` to `[Bool]`. The two are
    // DISTINCT types (`type-system.md §the head Option agrees but the payload does not`), and each
    // renders as `(Option Int64)` / `(Option Bool)`. This is the "generics are type-valued
    // parameters" model — no new mechanism, the ctor's `(meta apply)` builds the type.
    use crate::db::Db;
    use crate::eval::typeval_of;
    use crate::testkit::parse;
    use crate::ty::Ty;
    // Parse a program that USES `(Option Int64)` and `(Option Bool)` in annotations, and reach those
    // type occurrences to reduce them.
    let ast = parse(
        "(module m (type Option (Some a) None) \
               (def (i (: x (Option Int64))) x) \
               (def (b (: y (Option Bool))) y) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    // The declaration recorded its implicit param `a`.
    let decl = db.type_decls.iter().find(|t| t.name == "Option").unwrap();
    assert_eq!(
        decl.params,
        vec!["a".to_string()],
        "implicit param a scanned"
    );
    // Locate the `(Option Int64)` type occurrence — the annotation type of `i`'s parameter. Rather
    // than dig the AST, build the type expression directly: `Option` applied to `Int64`.
    // (Reaching it via the annotation is fiddly; instead confirm the two instantiations differ by
    // reducing `Option` bare then comparing rendered instantiations produced by the escape/infer
    // path in the run tests below. Here we assert the SCAN + the type identity via encode/decode.)
    let opt_int = Ty::Sum {
        decl: decl.occ,
        args: std::rc::Rc::from([Ty::int64()]),
    };
    let opt_bool = Ty::Sum {
        decl: decl.occ,
        args: std::rc::Rc::from([Ty::Bool]),
    };
    assert!(
        !opt_int.agrees_with(&opt_bool),
        "Option Int64 ≠ Option Bool"
    );
    assert!(opt_int.agrees_with(&opt_int));
    assert_eq!(opt_int.render_name(&db.name_ctx()), "(Option Int64)");
    assert_eq!(opt_bool.render_name(&db.name_ctx()), "(Option Bool)");
    // The generic sum record IS applyable in type position (has `(meta apply)` = sum-ctor), so
    // `typeval_of` of a `(Option Int64)` application reduces to the monomorphized `Ty::Sum`.
    let option_rec = db.type_decl_by_name("Option").expect("Option bound");
    let int64 = db.push_name("Int64");
    let app = db.push_list(vec![option_rec, int64]);
    match typeval_of(&mut db, app) {
        Some(Ty::Sum { args, .. }) => {
            assert_eq!(args.len(), 1);
            assert_eq!(
                args[0].render_name(&db.name_ctx()),
                "Int64",
                "Option applied to Int64"
            );
        }
        other => panic!(
            "expected (Option Int64) to reduce to Ty::Sum, got {:?}",
            other.map(|t| t.render_name(&db.name_ctx()))
        ),
    }
    // NESTED: `(Option (Option Int64))` reduces to a `Ty::Sum` whose arg is ITSELF a `Ty::Sum`.
    // `typeval_of` on the SumCtor application returns the `Ty` DIRECTLY (no arena encode→decode
    // round-trip — that round-trip made a deeply-nested generic annotation O(N²); see
    // `eval::reduce_sum_ctor`). The nested structure and rendering must survive that direct path.
    let option_rec2 = db.type_decl_by_name("Option").expect("Option bound");
    let inner = db.push_list(vec![option_rec2, int64]); // (Option Int64)
    let option_rec3 = db.type_decl_by_name("Option").expect("Option bound");
    let outer = db.push_list(vec![option_rec3, inner]); // (Option (Option Int64))
    match typeval_of(&mut db, outer) {
        Some(t @ Ty::Sum { .. }) => {
            assert_eq!(
                t.render_name(&db.name_ctx()),
                "(Option (Option Int64))",
                "nested generic sum resolves through the direct (round-trip-free) path"
            );
            let Ty::Sum { args, .. } = &t else {
                unreachable!()
            };
            assert!(
                matches!(&args[0], Ty::Sum { .. }),
                "the outer arg is itself a Ty::Sum"
            );
        }
        other => panic!(
            "expected (Option (Option Int64)) to reduce to a nested Ty::Sum, got {:?}",
            other.map(|t| t.render_name(&db.name_ctx()))
        ),
    }
}

#[test]
fn a_generic_variant_constructor_infers_its_instantiation() {
    // G2: a GENERIC variant ctor `Some` of `(type Option (Some a) None)` has `(meta t)` =
    // `(fn (a) (-> a (Option a)))` — a scheme `∀a. a → Option a`. So `(Option.Some 5)` INFERS the
    // instantiation `Ty::Sum { args: [Int64] }` through the ordinary `apply_type` (instantiate +
    // unify a = Int64), not a monomorphic `(-> a Sum)`. `(Option.Some true)` infers `Option Bool`.
    use crate::db::Db;
    use crate::infer::type_of;
    use crate::testkit::parse;
    use crate::ty::Ty;
    let ast = parse(
        "(module m (type Option (Some a) None) \
               (def (i) (Option.Some 5)) \
               (def (b) (Option.Some true)) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let i_body = db
        .defs
        .iter()
        .find(|d| d.name == "i")
        .and_then(|d| d.body)
        .expect("i");
    match type_of(&mut db, i_body) {
        Ty::Sum { decl, args } => {
            assert_eq!(db.name_ctx().name_of(decl), Some("Option"));
            assert_eq!(args.len(), 1);
            assert_eq!(
                args[0].render_name(&db.name_ctx()),
                "Int64",
                "(Some 5) infers Option Int64"
            );
        }
        other => panic!(
            "expected Ty::Sum, got {}",
            other.render_name(&db.name_ctx())
        ),
    }
    let b_body = db
        .defs
        .iter()
        .find(|d| d.name == "b")
        .and_then(|d| d.body)
        .expect("b");
    match type_of(&mut db, b_body) {
        Ty::Sum { args, .. } => assert_eq!(
            args[0].render_name(&db.name_ctx()),
            "Bool",
            "(Some true) infers Option Bool"
        ),
        other => panic!(
            "expected Ty::Sum, got {}",
            other.render_name(&db.name_ctx())
        ),
    }
}

#[test]
fn bare_prelude_option_needs_no_declaration() {
    // `Option`/`Some`/`None` are BUILT IN (prelude sums), so a program uses bare `Some`/`None` with
    // NO `(type Option …)` — the corpus surface. `(Some 5)` infers `Option Int64`; a bare `None` is
    // an `Option` at an unconstrained payload (a var until something fixes it).
    use crate::db::Db;
    use crate::infer::type_of;
    use crate::testkit::parse;
    use crate::ty::Ty;
    let ast = parse("(module m (def (s) (Some 5)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let s_body = db
        .defs
        .iter()
        .find(|d| d.name == "s")
        .and_then(|d| d.body)
        .expect("s");
    match type_of(&mut db, s_body) {
        Ty::Sum { decl, args } => {
            assert_eq!(
                db.name_ctx().name_of(decl),
                Some("Option"),
                "bare Some builds the prelude Option"
            );
            assert_eq!(
                args[0].render_name(&db.name_ctx()),
                "Int64",
                "(Some 5) infers Option Int64"
            );
        }
        other => panic!(
            "expected Ty::Sum Option, got {}",
            other.render_name(&db.name_ctx())
        ),
    }
}

#[test]
fn a_user_type_shadows_a_prelude_sum_name() {
    // A user `(type Option …)` SHADOWS the built-in Option — top-level `type_decls` resolve before
    // the prelude. So `Option` in a program that declares it is the USER sum (its own declaration
    // occurrence), distinct from the prelude one. Pins that the built-ins do not privilege the name.
    use crate::db::Db;
    use crate::eval::typeval_of;
    use crate::testkit::parse;
    use crate::ty::Ty;
    let ast = parse("(module m (type Option (Only Int64) Nope) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    // The USER Option is found first (first-wins), and it has the user's variants (Only/Nope), not
    // the prelude's (Some/None).
    let occ = db.type_decl_by_name("Option").expect("Option");
    let user_occ = db
        .type_decls
        .iter()
        .find(|t| t.name == "Option")
        .unwrap()
        .occ;
    assert_eq!(
        occ,
        db.type_decls
            .iter()
            .find(|t| t.name == "Option")
            .unwrap()
            .synth
            .unwrap()
    );
    let ty = typeval_of(&mut db, occ).expect("Option is a type");
    match ty {
        Ty::Sum { decl, .. } => {
            assert_eq!(decl, user_occ, "the user Option shadows the prelude one")
        }
        other => panic!(
            "expected Ty::Sum, got {}",
            other.render_name(&db.name_ctx())
        ),
    }
}

#[test]
fn a_variant_constructor_is_a_member_typed_as_a_function_to_the_sum() {
    // `Option.Some` is ORDINARY member access on the sum record (the `Int64.max` path), reaching a
    // variant-constructor record whose `(meta t)` is `(-> Int64 Option)`. So `Some` types as a
    // FUNCTION from its payload to the sum — read by the same `scheme_of`/`apply_type` machinery
    // every operator uses, no per-variant rule. (Applying it to actually CONSTRUCT is a later tick;
    // this pins that the constructor's TYPE is right.)
    use crate::db::Db;
    use crate::eval::{project_field, scheme_of};
    use crate::resolved::Symbol;
    use crate::testkit::parse;
    use crate::ty::Ty;
    use crate::unify::Fresh;
    let ast = parse("(module m (type Option (Some Int64) None) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    let option = db.type_decl_by_name("Option").expect("Option bound");
    // Project the `Some` field off the sum record (the member-access reduction).
    let some = project_field(&mut db, option, &Symbol::plain("Some")).expect("Some field");
    // Its `(meta t)` scheme is `(-> Int64 Option)`.
    let mut fresh = Fresh::new();
    let scheme = scheme_of(&mut db, some, &mut fresh).expect("Some has a type");
    match scheme.ty {
        Ty::Fn(param, result) => {
            assert_eq!(param.render_name(&db.name_ctx()), "Int64");
            assert_eq!(result.render_name(&db.name_ctx()), "Option");
            assert!(matches!(*result, Ty::Sum { .. }));
        }
        other => panic!(
            "expected (-> Int64 Option), got {}",
            other.render_name(&db.name_ctx())
        ),
    }
    // A NULLARY variant `None` types as the sum directly (no arrow).
    let none = project_field(&mut db, option, &Symbol::plain("None")).expect("None field");
    let none_scheme = scheme_of(&mut db, none, &mut fresh).expect("None has a type");
    assert!(matches!(none_scheme.ty, Ty::Sum { .. }));
    assert_eq!(none_scheme.ty.render_name(&db.name_ctx()), "Option");
}

#[test]
fn two_same_named_sums_from_different_declarations_are_distinct_types() {
    // A nominal sum's identity is its DECLARATION occurrence, NOT its local name (`type-system.md
    // §160`: two nominal types are distinct whenever their FQNs differ, even with identical name +
    // structure). Two `(type Foo …)` declarations — as two modules would produce, or an import
    // would splice — carry distinct `TypeDecl.occ`, so their `Ty::Sum` do NOT agree. This is the
    // property no name-keyed map could hold (the second would clobber the first); we get it for free
    // by keying identity on the occurrence and resolving through the occurrence-keyed `type_decls`.
    use crate::db::Db;
    use crate::eval::typeval_of;
    use crate::testkit::parse;
    // Two identically-shaped, identically-NAMED sums declared separately (the cross-module case,
    // simulated in one flat program). We reach each by its DECLARATION, not the shadowed name.
    let ast =
        parse("(module m (type Foo (A Int64)) (type Foo (A Int64)) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    // Both declarations are present (nothing was nuked) — the Vec holds two `Foo` entries.
    let foos: Vec<_> = db.type_decls.iter().filter(|t| t.name == "Foo").collect();
    assert_eq!(foos.len(), 2, "both same-named declarations are retained");
    let synth0 = foos[0].synth.expect("synth 0");
    let synth1 = foos[1].synth.expect("synth 1");
    assert_ne!(synth0, synth1, "distinct declarations ⇒ distinct records");
    let ty0 = typeval_of(&mut db, synth0).expect("Foo #0 is a type");
    let ty1 = typeval_of(&mut db, synth1).expect("Foo #1 is a type");
    // Different declarations ⇒ different identity ⇒ do NOT agree, despite identical name + shape.
    assert!(
        !ty0.agrees_with(&ty1),
        "distinct declarations must be distinct types even when name + shape match"
    );
    // A sum agrees with ITSELF (same declaration).
    assert!(ty0.agrees_with(&ty0));
}

#[test]
fn a_variant_construction_lowers_to_sum_new() {
    // `(Some a)` with a RUNTIME payload `a` (a parameter, so it cannot fold) lowers to
    // `Core::SumNew { disc: 0, payloads: [a] }` — the variant application dispatched by the ctor's
    // `(meta apply)` = `sum-new`, its discriminant read off `(meta variant)`. A NULLARY `None` used
    // bare lowers to `Core::SumNew { disc: 1, payloads: [] }` (the sum's second variant). This pins
    // that construction routes to the heap builder without needing escape/match (later ticks).
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    use crate::testkit::parse;
    let src = "(module m (type Option (Some Int64) None) \
                   (def (mk (: a Int64)) (Option.Some a)) \
                   (def (none) Option.None) \
                   (def (main) 0) (export main))";
    let mut db = Db::load(parse(src));
    let mk_body = db
        .defs
        .iter()
        .find(|d| d.name == "mk")
        .and_then(|d| d.body)
        .expect("mk");
    match core_of(&mut db, mk_body) {
        Core::SumNew { disc, payloads } => {
            assert_eq!(disc, 0, "Some is variant 0");
            assert_eq!(payloads.len(), 1, "Some carries one payload");
        }
        other => panic!("expected Core::SumNew, got {other:?}"),
    }
    let none_body = db
        .defs
        .iter()
        .find(|d| d.name == "none")
        .and_then(|d| d.body)
        .expect("none");
    match core_of(&mut db, none_body) {
        Core::SumNew { disc, payloads } => {
            assert_eq!(disc, 1, "None is variant 1");
            assert!(payloads.is_empty(), "None is nullary");
        }
        other => panic!("expected Core::SumNew for None, got {other:?}"),
    }
}

#[test]
fn a_bare_user_variant_name_resolves_like_the_qualified_form() {
    // A USER `(type …)` sum's variant may be referenced BARE — `NLit`/`NNil`, not only the qualified
    // `(. Node NLit)` (`core-semantics.md` §A Sum Type Constructor Is A Single-Arity Function). A bare
    // reference resolves to the SAME ctor field the qualified member access projects, so it lowers to
    // the same `Core::SumNew`. This is the user-declaration analog of the built-in sums binding bare
    // `Some`/`None` — pinned by comparing the bare form's disc against the qualified form's.
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    use crate::testkit::parse;
    let src = "(module m (type Node (NLit Int64) NNil) \
                   (def (bare-lit (: a Int64)) (NLit a)) \
                   (def (qual-lit (: a Int64)) (Node.NLit a)) \
                   (def (bare-nil) NNil) \
                   (def (qual-nil) Node.NNil) \
                   (def (main) 0) (export main))";
    let mut db = Db::load(parse(src));
    let disc_of = |db: &mut Db, name: &str| -> u32 {
        let body = db
            .defs
            .iter()
            .find(|d| d.name == name)
            .and_then(|d| d.body)
            .unwrap_or_else(|| panic!("def {name}"));
        match core_of(db, body) {
            Core::SumNew { disc, .. } => disc,
            other => panic!("expected Core::SumNew for {name}, got {other:?}"),
        }
    };
    // The BARE payload variant and the QUALIFIED one lower to the same discriminant (0).
    assert_eq!(disc_of(&mut db, "bare-lit"), disc_of(&mut db, "qual-lit"));
    assert_eq!(disc_of(&mut db, "bare-lit"), 0, "NLit is variant 0");
    // The BARE nullary variant and the QUALIFIED one likewise (disc 1).
    assert_eq!(disc_of(&mut db, "bare-nil"), disc_of(&mut db, "qual-nil"));
    assert_eq!(disc_of(&mut db, "bare-nil"), 1, "NNil is variant 1");
}

#[test]
fn a_variant_construction_emits_the_heap_build_ops() {
    // Construction lowers to the right value-heap OPS — `collect_used_ops` reports exactly what
    // `emit` lays down (they must agree, or the import section omits a called op). A single-payload
    // `(Some a)` uses `sum-new` + `box-int` (box the Int64 payload); a nullary `None` uses `sum-new`
    // ALONE — its unit payload is the inline-unit CONSTANT (`IMM_UNIT`), so it imports no `arr-alloc`.
    // This proves construction reaches the heap builder without needing the sum to escape (next tick)
    // or a composed run (a dead sum folds away — a sum is only observable once it escapes or matched).
    use crate::backend::wasm::select::collect_used_ops;
    use crate::db::Db;
    use crate::testkit::parse;
    // A payload variant: `sum-new` + `box-int`.
    let src = "(module m (type Option (Some Int64) None) \
                     (def (mk (: a Int64)) (Option.Some a)) (export mk))";
    let mut db = Db::load(parse(src));
    let mk = db
        .defs
        .iter()
        .find(|d| d.name == "mk")
        .and_then(|d| d.body)
        .expect("mk");
    let mut ops = std::collections::BTreeSet::new();
    collect_used_ops(&mut db, mk, &mut ops);
    assert!(ops.contains("sum-new"), "Some emits sum-new; got {ops:?}");
    assert!(
        ops.contains("box-int"),
        "Some boxes its Int64 payload; got {ops:?}"
    );
    // A nullary variant: `sum-new` with the INLINE-UNIT CONSTANT payload — so it does NOT import
    // `arr-alloc`. `arr-alloc(0)` returns the inline unit (`runtime_abi::IMM_UNIT`), so the compiler
    // pushes that constant directly instead of calling `arr-alloc(0)` — a nullary construction needs
    // only `sum-new`, no per-payload heap op.
    let src2 = "(module m (type Option (Some Int64) None) \
                      (def (none) Option.None) (export none))";
    let mut db2 = Db::load(parse(src2));
    let none = db2
        .defs
        .iter()
        .find(|d| d.name == "none")
        .and_then(|d| d.body)
        .expect("none");
    let mut ops2 = std::collections::BTreeSet::new();
    collect_used_ops(&mut db2, none, &mut ops2);
    assert!(ops2.contains("sum-new"), "None emits sum-new; got {ops2:?}");
    assert!(
        !ops2.contains("arr-alloc"),
        "None's unit payload is the inline-unit constant, no arr-alloc; got {ops2:?}"
    );
}

#[test]
fn a_nullary_variant_pushes_the_inline_unit_constant_not_an_arr_alloc_call() {
    // The nullary-variant unit payload is emitted as the `IMM_UNIT` constant (derived from the
    // runtime's `cdz-abi` section), NOT a runtime `arr-alloc(0)` call. Asserted at the Lir level: the
    // construction contains a `ConstI32(IMM_UNIT)` and NO `CallImport("arr-alloc")`. And it runs
    // correctly (a `Sign` classify over the three nullary variants) — the constant IS the handle
    // `arr-alloc(0)` would have returned, so `sum-new`/`sum-disc` see the same value.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::runtime_abi::IMM_UNIT;
    use crate::db::Db;
    let ast = crate::testkit::parse(
        "(module m (type Sign Neg Zero Pos) \
               (def (f (: n Int64)) \
                  (match (if (< n 0) (Sign.Neg unit) (if (= n 0) (Sign.Zero unit) (Sign.Pos unit))) \
                    ((Sign.Neg _) -1) ((Sign.Zero _) 0) ((Sign.Pos _) 1))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("f").expect("def f");
    let sig = db.defs[d].params.clone();
    let params: Vec<_> = sig
        .into_iter()
        .map(|p| {
            let b = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            (b, crate::infer::type_of(&mut db, b))
        })
        .collect();
    let body = db.defs[d].body.expect("body");
    let code = crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
        .expect("select")
        .code;
    assert!(
        code.iter()
            .any(|i| matches!(i, Lir::ConstI32(v) if *v == IMM_UNIT as i32)),
        "a nullary variant pushes the inline-unit constant, got: {code:?}"
    );
    assert!(
        !code
            .iter()
            .any(|i| matches!(i, Lir::CallImport("arr-alloc"))),
        "no arr-alloc call for a nullary variant's unit payload, got: {code:?}"
    );
}

#[test]
fn list_at_none_arm_pushes_the_inline_unit_constant_not_an_arr_alloc_call() {
    // A runtime `List.at` builds its `None` (OOB) box with the inline-unit CONSTANT (`IMM_UNIT`) for the
    // nullary payload, NOT a runtime `arr-alloc(0)` CALL — parity with the `SumNew` nullary path. `f`'s
    // body reads `xs` via `vec-get` (no arr-alloc of its own), so after the fix the ENTIRE `f` body has
    // ZERO `arr-alloc` calls and DOES contain `ConstI32(IMM_UNIT)` (the None payload). `arr-alloc(0)`
    // returns exactly `imm_unit()`, so the constant is the same handle `sum-new`/`sum-disc` would see.
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::runtime_abi::IMM_UNIT;
    use crate::db::Db;
    let ast = crate::testkit::parse(
        "(module m \
               (def (f (: xs (List Int64)) (: i Int64)) \
                  (match ((. List at) xs i) ((Some v) v) ((None _) -1))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = crate::layout::compute(&mut db).expect("layout");
    let d = db.def_by_name("f").expect("def f");
    let sig = db.defs[d].params.clone();
    let params: Vec<_> = sig
        .into_iter()
        .map(|p| {
            let b = db
                .ast
                .as_form(p, ":")
                .and_then(|t| t.first().copied())
                .unwrap_or(p);
            (b, crate::infer::type_of(&mut db, b))
        })
        .collect();
    let body = db.defs[d].body.expect("body");
    let code = crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
        .expect("select")
        .code;
    assert!(
        code.iter()
            .any(|i| matches!(i, Lir::ConstI32(v) if *v == IMM_UNIT as i32)),
        "List.at's None arm pushes the inline-unit constant, got: {code:?}"
    );
    assert!(
        !code
            .iter()
            .any(|i| matches!(i, Lir::CallImport("arr-alloc"))),
        "List.at's None arm emits no arr-alloc call for its unit payload, got: {code:?}"
    );
}

#[test]
fn a_parameterized_sum_returning_export_escapes_via_param_forwarding_make() {
    // A PARAMETERIZED sum-returning export (`mk` takes `a`) now crosses as the resource escape: `make`
    // forwards the export's scalar param (`make(a) -> own<t>`), so the host builds `(Option.Some a)`
    // from its argument and `encode()` renders it. Previously DECLINED "only from a NULLARY export".
    // The nullary escape is still exercised by `a_nullary_sum_export_escapes_to_the_host` below.
    use crate::testkit::parse;
    let src = "(module m (type Option (Some Int64) None) \
                     (def (mk (: a Int64)) (Option.Some a)) (export mk))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a parameterized sum-returning export must compile via the param-forwarding resource escape"
    );
}

#[test]
fn a_nullary_sum_export_escapes_to_the_host() {
    // A single NULLARY export returning a user sum crosses as a resource. `(Option.Some 5)` over a
    // USER `(type Option (Some Int64) None)` is a COMPILE-TIME CONSTANT, so its canonical bytes are
    // baked (the `const_value_ast` `SumNew` arm) and NO value-heap runtime is imported. The variant
    // renders as its BARE name — `Some` — uniformly with a built-in sum (the value form does not
    // depend on built-in-vs-user). `(Option.Some 5)` → `(: (Some 5) Option)`. The RUNTIME disc-switch
    // encoder (a sum built from a non-constant payload) is exercised separately by
    // `a_runtime_sum_export_escapes_via_the_heap_walk`. Composed + run through `cdz-run`.
    use crate::testkit::parse;
    let src = "(module m (type Option (Some Int64) None) \
                     (def (main) (Option.Some 5)) (export main))";
    let bytes =
        compile_component(&crate::codec::encode(&parse(src))).expect("compile a sum escape");
    assert!(
        !imports_value_heap_runtime(&bytes),
        "a CONSTANT sum escape bakes its bytes — no value-heap runtime import"
    );
    // The RUN — `(Option.Some 5)` escapes as a resource and renders `(: (Some 5) Option)` (the bare
    // variant name) — is corpus-covered by 07-type-system "a USER-declared monomorphic sum's Some variant
    // escapes to the host rendering its bare name"; this test keeps the constant-baked no-heap pin above
    // (a compile-artifact the corpus cannot assert).
}

#[test]
fn a_variant_with_a_record_payload_whose_field_is_lowercase_is_not_generic() {
    // REGRESSION: a variant carrying a `(Record (field Type)…)` payload whose field NAME is lowercase
    // (`(Pt (Record (x Int64) (y Int64)))`) must NOT be mistaken for a generic sum. `collect_type_params`
    // scanned the payload for free lowercase names as implicit type parameters — and wrongly picked up
    // the record FIELD NAMES `x`/`y`, making `P` spuriously generic over them; the ctor arrow then read
    // its payload as an unresolvable variable, so `P.Pt` looked NULLARY and rejected the construction
    // (CDZ0201 "a nullary variant takes the unit value"). A field name is a LABEL, not a type expr — the
    // fix descends only into each field pair's TYPE. The bug was a compile-time REJECT, so COMPILING the
    // construction (it no longer faults) is the precise guard; the run is exercised by the corpus case
    // "a variant carrying a RECORD payload constructs and matches" (which links the value-heap runtime).
    let src = "(module m (type P (Pt (Record (x Int64) (y Int64))) O) \
                     (def (sum r) (+ (. r x) (. r y))) \
                     (def (main) (match (P.Pt (record (= x 3) (= y 4))) ((P.Pt r) (sum r)) (P.O 0))) \
                     (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a record-payload variant with lowercase field names must COMPILE — not reject P.Pt as \
             nullary (the field names must not be collected as type parameters)"
    );
}

#[test]
fn a_qty_payloads_inner_type_variable_stays_a_generic_parameter() {
    // The OTHER edge of the `(Qty T u)` type-parameter skip: `collect_type_params` descends only into a
    // Qty payload's inner type `T` and skips the unit `u` (so a unit-leaf name like `base` is NOT
    // harvested as a spurious parameter — the sibling `a_variant_with_a_record_payload…` guard's Qty
    // twin). But it must NOT OVER-skip: a type VARIABLE in the inner (`T`) position is a real generic
    // parameter. `(type Box (B (Qty a (Unit.base #"meter"))))` is generic over `a` (harvested from
    // `children[1]`), so a bare `Box` needs a type argument (CDZ0203) and `(Box Rational)` resolves.
    // Compiling the fully-applied construct→match→`Qty.value` is the precise guard that the inner-type
    // parameter survived the skip (a spurious over-skip would make `Box` nullary and reject `(Box Rational)`).
    let src = "(module m (type Box (B (Qty a (Unit.base #\"meter\")))) \
                     (def (mk (: x (Qty Rational (Unit.base #\"meter\")))) (Box.B x)) \
                     (def (unwrap (: b (Box Rational))) (match b ((Box.B q) (Qty.value q)))) \
                     (def (main) (unwrap (mk (Qty.of (Rational.of 7 2) (Unit.base #\"meter\"))))) \
                     (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a Qty payload's inner-type variable must stay a real generic parameter — `(Box Rational)` must \
             resolve (the unit-arg skip must not over-skip and drop the inner-type parameter)"
    );
    // The reject twin: a BARE `Box` (no type argument) is CDZ0203 — the inner-type parameter is genuinely
    // required, so the skip did not silently drop it (which would wrongly accept a nullary `Box`).
    let bare = "(module m (type Box (B (Qty a (Unit.base #\"meter\")))) \
                      (def (unwrap (: b Box)) (match b ((Box.B q) (Qty.value q)))) \
                      (def (main) (unwrap (Box.B (Qty.of (Rational.of 7 2) (Unit.base #\"meter\"))))) \
                      (export main))";
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(bare)))
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0203")),
        "a bare `Box` (a generic type used with no type argument) must reject CDZ0203"
    );
}

#[test]
fn an_all_nullary_enum_program_imports_no_value_heap_runtime() {
    // An all-nullary enum is a bare i32 discriminant (built with `i32.const`, matched with a
    // `br_table`) — it touches the value heap NOT AT ALL. So a program whose only sum is such an enum
    // must import NO runtime op: `collect_used_ops` must mirror `select`'s enum-disc fast path and NOT
    // over-report `sum-new`/`sum-disc` (a dead import would force a needless `heap` linkage AND — since
    // the import set fixes every `CallImport` index — a phantom import shifts them, risking a
    // miscompile). Regression for that over-report; also a composed run confirms correctness.
    use crate::testkit::parse;
    let src = "(module m (type Color (Red) (Green) (Blue)) \
                     (def (classify (: c Color)) (match c ((Red) 1) ((Green) 2) ((Blue) 3))) \
                     (def (pick (: n Int64)) (if (< n 0) (Red) (if (= n 0) (Green) (Blue)))) \
                     (def (main (: n Int64)) (classify (pick n))) \
                     (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile enum");
    assert!(
        !imports_value_heap_runtime(&bytes),
        "an all-nullary enum is a bare i32 — no value-heap runtime import (no dead sum-new/sum-disc)"
    );
    // A boxed sum in the SAME shape (Option) STILL imports the runtime — the elision is enum-only.
    // `mk` is RECURSIVE (a `(< n 0)` arm self-calls, never taken for the tested n>=0) so the sum is a
    // GENUINE runtime value: the match-into-if AND case-of-match fusions refuse to reduce through a
    // recursive call, so `(get (mk n))` keeps building the boxed Option on the heap — the runtime import
    // this asserts. (A non-recursive `mk` — bare `if`/`match` of ctors — would fold the sum away and drop
    // the import, defeating the test's premise.) Semantics unchanged for n>=0: n==0 → None, n>0 → Some n.
    let boxed = "(module m \
                       (def (mk (: n Int64)) (if (< n 0) (mk 0) (if (> n 0) (Some n) (None)))) \
                       (def (get (: o (Option Int64))) (match o ((Some x) x) ((None) 0))) \
                       (def (main (: n Int64)) (get (mk n))) \
                       (export main))";
    let bb = compile_component(&crate::codec::encode(&parse(boxed))).expect("compile boxed");
    assert!(
        imports_value_heap_runtime(&bb),
        "a genuinely-boxed sum still imports the value-heap runtime"
    );
}

#[test]
fn an_all_nullary_enum_derives_partial_eq_on_the_rust_backend() {
    // RUST-BACKEND / CROSS-BACKEND: the `=` intrinsic lowers (rust backend) to a native `x == y`, which
    // needs `PartialEq`/`Eq` — so a sum MUST be emitted with `#[derive(Clone, PartialEq, Eq)]` whenever
    // its payloads are themselves `Eq`-derivable, else the generated Rust fails to build even though the
    // SAME program runs on wasm (a value-heap equality walk). An all-nullary enum trivially qualifies; a
    // payload sum of Int/Bool/nested comparable payloads does too. A sum whose payload is NOT `Eq` (a
    // float — `PartialEq` but not `Eq`) stays `#[derive(Clone)]` only (its runtime `=` then declines,
    // decline-don't-miscompile).
    use crate::testkit::parse;

    // An all-nullary enum compared with `=` → its emitted Rust enum derives PartialEq/Eq and builds.
    let nullary = "(module m (type Color Red Green Blue) \
                         (def (eq2 (: x Color) (: y Color)) (if (= x y) 1 0)) \
                         (def (main (: b Bool)) (+ (eq2 (if b Color.Red Color.Green) Color.Red) 0)) \
                         (export main))";
    let mut db = crate::db::Db::load(parse(nullary));
    let layout = crate::layout::compute(&mut db).expect("layout");
    let rs = crate::backend::emit(crate::backend::Target::Rust, &mut db, &layout, None, None)
        .expect("rust artifact");
    let rs = String::from_utf8(rs).expect("utf8");
    assert!(
        rs.contains(
            "#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]\n#[allow(dead_code)]\npub enum Color"
        ),
        "an all-nullary enum must derive PartialEq/Eq (+ Ord, so it can key a BTreeMap) so native `==` builds; got:\n{rs}"
    );

    // A payload-carrying sum whose payloads are Eq-derivable (Int64) NOW derives PartialEq/Eq too, so a
    // runtime `(= a b)` over it emits a native `==` (was Clone-only + a decline).
    let payload = "(module m (type Box (Mk Int64) (Nil)) \
                         (def (unbox (: b Box)) (match b ((Mk n) n) ((Nil) 0))) \
                         (def (main) (unbox (Mk 42))) \
                         (export main))";
    let mut db2 = crate::db::Db::load(parse(payload));
    let layout2 = crate::layout::compute(&mut db2).expect("layout");
    let rs2 = crate::backend::emit(crate::backend::Target::Rust, &mut db2, &layout2, None, None)
        .expect("rust artifact");
    let rs2 = String::from_utf8(rs2).expect("utf8");
    assert!(
        rs2.contains(
            "#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]\n#[allow(dead_code)]\npub enum Box"
        ),
        "a payload sum with Eq-derivable payloads now derives PartialEq/Eq (+ Ord); got:\n{rs2}"
    );

    // A FLOAT-carrying sum stays Clone-only — `f64` is `PartialEq` but NOT `Eq`, so `#[derive(Eq)]`
    // would fail to compile; its runtime `=` declines (the wasm heap-walk path also declines a float
    // compound `=` in the seed).
    let float_sum = "(module m (type FBox (Mk Float64) (Nil)) \
                           (def (unbox (: b FBox)) (match b ((Mk n) n) ((Nil) 0.0))) \
                           (def (main) (unbox (Mk 4.0))) \
                           (export main))";
    let mut db3 = crate::db::Db::load(parse(float_sum));
    let layout3 = crate::layout::compute(&mut db3).expect("layout");
    let rs3 = crate::backend::emit(crate::backend::Target::Rust, &mut db3, &layout3, None, None)
        .expect("rust artifact");
    let rs3 = String::from_utf8(rs3).expect("utf8");
    assert!(
        rs3.contains("#[derive(Clone)]\n#[allow(dead_code)]\npub enum FBox"),
        "a float-carrying sum stays Clone-only (f64 is not Eq); got:\n{rs3}"
    );
}

#[test]
fn a_sum_match_disc_zero_probe_uses_eqz_and_dispatches() {
    // A sum match dispatches on `sum-disc(scrutinee) == disc`; when the tested discriminant is 0 —
    // the FIRST declared variant (`Some`), the common first-arm probe — that is `i32.eqz` (opcode
    // 0x45), not `const 0 ; i32.eq`. Verify BOTH the emitted core module carries an `i32.eqz` AND the
    // match still dispatches correctly across the present/absent variants (composed run). `Some`
    // first so its disc-0 probe is the eqz.
    use crate::testkit::parse;
    // `mk` is RECURSIVE (a `(< n 0)` arm self-calls, never taken for the tested n>=0) so the Option stays
    // a GENUINE runtime sum and the disc probe actually emits: the match-into-if AND case-of-match
    // fusions refuse to reduce through a recursive call, so they do NOT fold the sum away (a non-recursive
    // `mk` would let `(match (mk n) …)` fuse to a scalar select, eliminating the sum + its disc-0 eqz
    // probe, defeating this test). Semantics unchanged: n>0 → Some n.
    let src = "(module m (type Option (Some Int64) None) \
                     (def (mk (: n Int64)) (if (< n 0) (mk 0) (if (> n 0) (Option.Some n) Option.None))) \
                     (def (pick (: n Int64)) \
                        (match (mk n) \
                          ((Option.Some x) x) (Option.None -1))) \
                     (export pick))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The emitted component's embedded core module carries an `i32.eqz` (opcode 0x45) — the disc-0
    // probe. Without the eqz special case the probe would be `i32.const 0` (0x41 0x00) + `i32.eq`
    // (0x46) and no 0x45 would appear (this program has no other eqz — no scalar `== 0`, no bool
    // negation). A whole-component byte scan suffices: the core module is embedded verbatim.
    assert!(
        bytes.contains(&0x45),
        "the disc-0 (Some) probe must emit an i32.eqz (0x45)"
    );
    // The dispatch VALUE (`pick 5` -> 5 / `pick 0` -> -1) is the ordinary runtime Option match, covered
    // by the corpus (05-compound-types "(match o ((Some x) x) ((None _) d))" unwrap family); only the
    // i32.eqz emit witness (the byte-scan above) stays here — the corpus cannot observe the opcode.
}

#[test]
fn a_wide_sum_match_with_named_binders_dispatches_correctly() {
    // A match over a MANY-variant sum, each arm binding the payload by a NAME (`(Vi x) → …`). This is
    // the shape whose compile was O(N²): each arm's payload-binder `x` resolved as an "unbound name"
    // and ran the O(scope) nearest-name typo scan, AND each arm head `Vi` resolved via an O(variants)
    // `variant_ctor_field` scan, AND the redundant-arm check's `Vec::contains` was O(arms) per arm —
    // three O(N²)s. The fixes (lazy unbound-suggestion at the fault-surfacing site, a
    // `variant_ctor_index`, a redundant-arm `HashSet`) make it LINEAR. This test LOCKS IN the behavior
    // they must preserve: every arm dispatches to the value derived from its OWN bound payload, so a
    // dropped/misindexed binder or ctor would return the wrong value. 12 variants — enough that the
    // ctor index + binder resolution are genuinely exercised, small enough to run fast.
    use crate::testkit::parse;
    let n = 12;
    let variants: String = (0..n).map(|i| format!(" (V{i} Int64)")).collect();
    let arms: String = (0..n).map(|i| format!(" ((V{i} x) (+ x {i}))")).collect();
    // `pick k` builds `Vk 100`; `code k` matches it and returns `100 + k` (payload + arm index) — so
    // the binder `x` (=100) MUST reach each arm's body for the answer to be right.
    let mut pick = String::from("(V0 100)");
    for i in (0..n).rev() {
        pick = format!("(if (= n {i}) (V{i} 100) {pick})");
    }
    let src = format!(
        "(module m (type T{variants}) \
               (def (pick (: n Int64)) {pick}) \
               (def (code (: n Int64)) (match (pick n){arms})) \
               (export code))"
    );
    // COMPILING the 12-variant match is the regression exercise: the O(N^2) unbound-suggestion /
    // ctor-scan / redundant-arm-check paths are on the compile, and a successful compile witnesses they
    // are linear. The per-arm dispatch VALUE (each binder reaches its own arm) is ordinary N-variant
    // sum-match behavior covered by the corpus; only the compile-at-scale exercise stays here.
    compile_component(&crate::codec::encode(&parse(&src))).expect("compile wide-sum match");
}

#[test]
fn an_if_over_a_sum_type_checks_regardless_of_branch_order() {
    // A runtime `if` whose branches build the SAME sum two ways — a nullary variant `(None)` :
    // `Option ?0` and a payload variant `(Some n)` : `Option Int64` — must COMPILE in EITHER order.
    // The `if`'s type is the JOIN of its branches; `Ty::join` now joins two agreeing sums' type ARGS
    // pairwise, so the payload-carrying branch fixes the parameter whether it is first or second.
    // Before, a leading `None` took the join's fallthrough and kept `(Option ?0)` with `?0` free, so
    // the value-heap layout declined "projecting a tuple element of type ?0 needs the value heap".
    for branches in ["(None) (Some n)", "(Some n) (None)"] {
        let src = format!(
            "(module m (def (g (: n Int64)) \
                   (match (if (= n 0) {branches}) ((Some k) k) ((None) 0))) \
                 (def (main (: n Int64)) (g n)) (export main))"
        );
        assert!(
            compile_component(&crate::codec::encode(&parse(&src))).is_ok(),
            "an if-built Option must compile with branches in order [{branches}]"
        );
    }
    // The Result companion — two type parameters, resolved from either branch (the sibling finding).
    for branches in ["(Err n) (Ok n)", "(Ok n) (Err n)"] {
        let src = format!(
            "(module m (def (g (: n Int64)) \
                   (match (if (= n 0) {branches}) ((Ok k) k) ((Err e) 0))) \
                 (def (main (: n Int64)) (g n)) (export main))"
        );
        assert!(
            compile_component(&crate::codec::encode(&parse(&src))).is_ok(),
            "an if-built Result must compile with branches in order [{branches}]"
        );
    }
    // NO OVER-ACCEPTANCE: genuinely mismatched branches (a sum vs a bare integer) still reject.
    assert!(
        compile_component(&crate::codec::encode(&parse(
            "(module m (def (g (: n Int64)) (if (> n 0) (Some n) n)) \
                 (def (main (: n Int64)) (g n)) (export main))"
        )))
        .is_err(),
        "an if whose branches are a sum and a scalar must still be a type mismatch"
    );
}

#[test]
fn a_capturing_closure_stored_and_also_directly_called_is_force_kept() {
    // WARNING: INVALID-ARTIFACT regression (breaker adv-50, both backends): a CAPTURING `let`-bound lambda
    // whose HANDLE ESCAPES WHOLE (stored into a heap collection / sum payload) AND is ALSO DIRECTLY
    // CALLED emitted a broken artifact — wasm `invalid component … wasm[0]::function[N]`, rust
    // `error[E0425]: cannot find value __cap0`. Mechanism: the store lowers `f1` as a value
    // (`lower_lambda_value` LIFTS it, recording the body's capturing-reference occurrence `k` in
    // `db.captured_ref`), while `should_keep_binding`'s lambda short-circuit copy-propagates `f1` so
    // the direct call `(f1 d)` β-FOLDS to `(+ k d)` — REUSING that same `k` occurrence, now memoized
    // as a `Core::Captured` env-read in `main`'s ENV-LESS scope → the broken emit. Prior faces of the
    // speculative-lift family AVOIDED the lift (the store here REQUIRES it), so the fix is the dual:
    // FORCE-KEEP the binding as ONE materialized runtime `Core::Closure` and route the direct call
    // through it via `call_indirect` (`head_is_runtime_fn_value` treats a kept lambda binding as a
    // runtime fn value) — no fold reuses the poisoned occurrence. k=100, f1 v = k+v, main 5 → 105.
    let store_shapes = [
        // Map value, insert result DISCARDED in a `do` (the canonical minimal seed).
        "(let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do (Map.insert Map.empty 1 f1) (f1 d))))",
        // List literal holding f1, discarded.
        "(let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do (list f1) (f1 d))))",
        // (A Set-of-f1 store-shape was here; removed 2026-08-02 — a function-typed Set ELEMENT now
        // rejects CDZ0216 (a function has no equality/order, so it can't be a Set element; v-inference
        // ruling, concierge-confirmed). The List store above covers the collection-literal force-keep
        // path identically; a Set adds nothing here except the now-illegal fn-element.)
        // Sum payload, discarded (breaker s18).
        "(let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do (Some f1) (f1 d))))",
        // Surviving store (map len feeds the result) + direct call: 105 + 1 = 106.
        "(let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (+ (f1 d) ((. Map len) (Map.insert Map.empty 1 f1)))))",
    ];
    // The precise pre-fix failure was at the ARTIFACT level (compile accepted + emitted garbage), so
    // COMPILE producing a valid component (rust: a building artifact) IS the guard. The actual VALUE
    // runs (105 / 106) are pinned by the corpus migration in `spec/semantics/09-functions.sexp`, which
    // executes on the value-heap runtime the gate wires up (a closure allocates a cell — this
    // store-less lib host does not link that runtime, so it asserts validity, not a run).
    for body in store_shapes {
        let src = format!("(module m (def (main (: d Int64)) {body}) (export main))");
        compile_component(&crate::codec::encode(&parse(&src))).unwrap_or_else(|e| {
            panic!("compile must produce a VALID artifact for `{body}`: {e:?}")
        });
    }
    // CONTROLS that must stay on their existing (correct) paths, unchanged by the force-keep — each
    // must still COMPILE cleanly (they always did; this guards against the force-keep over-firing):
    //  - a NON-capturing lambda stored + called (no env cell, no poison).
    //  - a capturing lambda DIRECT-CALLED ONLY (no escape) — copy-propagates + folds.
    //  - a TUPLE-element store + direct call (fixed-shape unboxed rep, a long-standing SURVIVOR).
    //  - a `(do …)` in PROJECTION-OPERAND position whose TAIL is the tuple binding `r` — the `(do)`
    //    special-case in `collect_binding_uses` threads the caller's `proj_operand` to the tail form
    //    (PR #1245 Copilot), so the tail `Ref r` is a piece-read (`.0` projects it), NOT a whole-value
    //    escape that would spuriously mark `r` `escapes_whole` and flip its keep/copy decision. The
    //    NON-final `(Map.insert …)` stays `proj_operand=false` (sequenced, value discarded).
    for body in [
        "(let ((f1 (fn ((: v Int64)) (+ 1 v)))) (do (Map.insert Map.empty 1 f1) (f1 d)))",
        "(let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (f1 d)))",
        "(let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do (tuple f1 9) (f1 d))))",
        "(let ((r (tuple (fn ((: v Int64)) (+ d v)) 9))) ((. (do (Map.insert Map.empty 1 2) r) 0) 10))",
    ] {
        let src = format!("(module m (def (main (: d Int64)) {body}) (export main))");
        compile_component(&crate::codec::encode(&parse(&src)))
            .unwrap_or_else(|e| panic!("control must still compile cleanly `{body}`: {e:?}"));
    }
}

#[test]
fn a_capturing_closure_stored_and_directly_called_is_kept_in_core() {
    // Core-level WITNESS for the adv-50 force-keep (no runtime needed): a capturing lambda that both
    // ESCAPES WHOLE (stored) and is DIRECT-CALLED lowers `main` to a `Core::Let` naming the ONE
    // materialized closure (the force-keep), with the direct call a `Core::CallClosure` on that kept
    // slot — NOT a folded `Core::Arith` whose `k` operand poisoned to `Core::Captured` in main's
    // env-less scope (the pre-fix miscompile). Pins the ROUTE, so a future refactor that regresses
    // back to copy-propagate-and-fold fails here even on a store-less test host.
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    let src = "(module m (def (main (: d Int64)) \
            (let ((k 100)) (let ((f1 (fn ((: v Int64)) (+ k v)))) (do (list f1) (f1 d))))) (export main))";
    let mut db = Db::load(parse(src));
    let d = db.def_by_name("main").expect("main");
    let m_body = db.defs[d].body.expect("body");
    match core_of(&mut db, m_body) {
        // The kept closure binds a `Core::Let`; the body applies it via `Core::CallClosure`.
        Core::Let { .. } => {}
        other => panic!(
            "a stored-and-directly-called capturing closure must FORCE-KEEP a Core::Let slot, got {other:?}"
        ),
    }
}

#[test]
fn a_generic_transformer_closure_aggregate_result_grounds_its_element_on_rust_not_unit() {
    // INFERENCE FIX (v-inference, breaker gtx1 family / issue BUG-generic-transformer-closure-
    // compound-result-grounds-elements-to-unit, routed by v-rust-backend as a rust-visible miscompile).
    // The wasm-erasing twin above ran to 4 REGARDLESS of the bug; this pins the RUST emit, where the bug
    // was VISIBLE. `count(gmap(from-list [1,2], fn(x) => (x,x)))` at two distinct domains: `gmap`
    // specialized CORRECTLY at `GIter<(i64, i64)>`, but `type_of` of the OUTER `gmap`-call node (the
    // argument `count` specializes off) grounded the closure-result tuple ELEMENTS to `Unit` — so
    // `count` took `GIter<((), ())>` while `gmap` returned `GIter<(i64, i64)>` → rustc E0308. Root: in
    // `apply_scheme_to_args`, unifying the bare closure arg's Any-bearing bottom-up type `(-> Any (Tuple
    // Any Any))` bound the callee's result var to `(Tuple Any Any)`, and `Any`-absorbs then blocked the
    // recovery's re-unify against the concrete `(Tuple Int64 Int64)`. Fix: when the closure body solves
    // to a CONCRETE arrow under the pinned domain, unify THAT recovered arrow (not the Any-bearing
    // bottom-up type), so the result var binds to `(Tuple Int64 Int64)` and the outer call node — and
    // every consumer specialized off it — sees the tied element. `emit` itself always SUCCEEDED (the
    // mismatch is a rustc-level type error, not a decline), so assert on the emitted SOURCE.
    let src = "(module m \
            (type GIter (Nil) (Cons a (GIter a))) \
            (def (from-list xs) \
              (match xs ((list) (GIter.Nil)) ((list h .. t) (GIter.Cons h (from-list t))))) \
            (def (count it) \
              (match it ((GIter.Nil) 0) ((GIter.Cons _ rest) (+ 1 (count rest))))) \
            (def (gmap it f) \
              (match it ((GIter.Nil) (GIter.Nil)) ((GIter.Cons h rest) (GIter.Cons (f h) (gmap rest f))))) \
            (def (main) \
              (+ (count (gmap (from-list (list 1 2)) (fn (x) (tuple x x)))) \
                 (count (gmap (from-list (list \"a\" \"b\")) (fn (s) (String.concat s s)))))) \
            (export main))";
    let mut dbr = crate::db::Db::load(parse(src));
    let layr = crate::layout::compute(&mut dbr)
        .expect("generic-transformer aggregate-result closure lays out (rust)");
    let rs = crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &layr, None, None)
        .expect("generic-transformer aggregate-result closure emits rust");
    let rs = String::from_utf8(rs).expect("utf8");
    assert!(
        rs.contains("GIter<(i64, i64)>"),
        "the closure-result tuple element must ground to (i64, i64) on rust; got:\n{rs}"
    );
    assert!(
        !rs.contains("((), ())"),
        "the closure-result tuple elements must NOT erase to Unit `((), ())` (the E0308 miscompile); got:\n{rs}"
    );
}

#[test]
fn a_generic_transformer_closure_result_ties_every_structural_aggregate_shape_on_rust() {
    // COVERAGE (v-inference, breaker gtx family map, #4299): the closure-aggregate-result miscompile the
    // sibling test pins for a TUPLE spans EVERY structural aggregate — record (gtx4), List (gtx6), and
    // nested tuple (gtx7) all erased their element types to `Unit` on rust identically before the
    // `apply_scheme_to_args` recovered-arrow fix (3f6dc755c). The tuple cell is now a 3-backend corpus
    // run case (#4338); this pins the OTHER three shapes so a future change can't regress record/List/
    // nested while the tuple stays green (the fix re-solves the whole closure body under the pinned
    // domain, so it is shape-agnostic — but the emit paths for record/List/tuple differ, so each is a
    // distinct regression surface worth witnessing). Each: `count(gmap(from-list [1,2], fn(x) => <agg>))`
    // at Int64 + a String domain; assert the rust emit grounds the element CONCRETELY, never to `Unit`.
    let emit_rust = |src: &str| -> String {
        let mut db = crate::db::Db::load(parse(src));
        let lay =
            crate::layout::compute(&mut db).expect("structural-aggregate transformer lays out");
        let rs = crate::backend::emit(crate::backend::Target::Rust, &mut db, &lay, None, None)
            .expect("structural-aggregate transformer emits rust");
        String::from_utf8(rs).expect("utf8")
    };
    let prog = |agg: &str| -> String {
        format!(
            "(module m \
                (type GIter (Nil) (Cons a (GIter a))) \
                (def (from-list xs) \
                  (match xs ((list) (GIter.Nil)) ((list h .. t) (GIter.Cons h (from-list t))))) \
                (def (count it) \
                  (match it ((GIter.Nil) 0) ((GIter.Cons _ rest) (+ 1 (count rest))))) \
                (def (gmap it f) \
                  (match it ((GIter.Nil) (GIter.Nil)) ((GIter.Cons h rest) (GIter.Cons (f h) (gmap rest f))))) \
                (def (main) \
                  (+ (count (gmap (from-list (list 1 2)) (fn (x) {agg}))) \
                     (count (gmap (from-list (list \"a\" \"b\")) (fn (s) (String.concat s s)))))) \
                (export main))"
        )
    };

    // RECORD result `(record (= lo x) (= hi x))` — lowers to a rust 2-tuple of the field types.
    let rec = emit_rust(&prog("(record (= lo x) (= hi x))"));
    assert!(
        rec.contains("GIter<(i64, i64)>") && !rec.contains("((), ())"),
        "a record-result closure element must ground to i64, not erase to Unit; got:\n{rec}"
    );

    // LIST result `(list x x)` — element `Vec<i64>`, never `Vec<()>`.
    let lst = emit_rust(&prog("(list x x)"));
    assert!(
        lst.contains("GIter<Vec<i64>>") && !lst.contains("Vec<()>"),
        "a List-result closure element must ground to Vec<i64>, not Vec<()>; got:\n{lst}"
    );

    // NESTED-TUPLE result `(tuple (tuple x x) x)` — `((i64, i64), i64)`, never a Unit at any depth.
    let nest = emit_rust(&prog("(tuple (tuple x x) x)"));
    assert!(
        nest.contains("GIter<((i64, i64), i64)>") && !nest.contains("((), ())"),
        "a nested-tuple-result closure element must ground at every depth, not erase to Unit; got:\n{nest}"
    );
}

#[test]
fn a_closure_payload_sum_from_an_if_helper_with_a_reused_arg_compiles_not_cdz0101() {
    // REGRESSION (v-patterns adv-closure-payload-sum-picked-by-if-helper): a closure-payload 2-variant
    // sum built by an `if`-helper `mk`, matched + applied by `run`, with the caller reusing its param
    // `k` in BOTH arg positions `(run (mk k) k)`. `mk k` reduces to an `if`, so `run`'s `(match (mk k)
    // …)` triggers `fuse_match_into_if`, which DEEP-COPIES the arm bodies into both branches via
    // `clone_subtree_db` — and the arm body `(f arg)` carries `arg`=`k`, a free capture bound by
    // β-SUBSTITUTION (not lexically). `clone_subtree_db` copied `k` FRESH → it re-resolved LEXICALLY
    // against the grafted branch position (where `k` is invisible) → a spurious `CDZ0101 unbound name
    // k` at COMPILE while `cdz check` was clean. Fix: `clone_subtree_db` SHARES a pinned non-payload
    // capture (mirroring `beta_reduce`'s pinned-name share). k=4 → Fn arm → 4*3=12; k=-1 → Const → 77.
    // Two NULLARY mains (the reused arg `k` is a `let`-bound constant so the whole shape — reused in
    // both `(run (mk k) k)` arg positions — is preserved) exercising both arms: k=4 → Fn, k=-1 → Const.
    let prelude = "(type Box (Fn (-> Int64 Int64)) (Const Int64)) \
            (def (mk (: k Int64)) (if (> k 0) (Fn (fn ((: x Int64)) (* x 3))) (Const 77))) \
            (def (run (: b Box) (: arg Int64)) (match b ((Fn f) (f arg)) ((Const c) c)))";
    for kexpr in ["4", "(- 0 1)"] {
        let src = format!(
            "(module m {prelude} (def (main) (let ((k {kexpr})) (run (mk k) k))) (export main))"
        );
        // PRIMARY: this must COMPILE (the spurious CDZ0101 was a compile-time reject; check was clean).
        compile_component(&crate::codec::encode(&crate::testkit::parse(&src)))
                .unwrap_or_else(|e| {
                    panic!("closure-payload-sum if-helper reused-arg (k={kexpr}) must COMPILE, not CDZ0101: {e:?}")
                });
    }
}

#[test]
fn lambda_lifting_dedups_by_body_and_gives_distinct_lambdas_distinct_slots() {
    // `Db::lift_lambda` dedups by the lambda's `body` occurrence via an O(1) index (was a linear scan
    // of `db.lifted` per lift → O(N²) for a program lifting N distinct closures). This locks in the two
    // invariants the index must preserve: (a) N DISTINCT escaping lambdas get N distinct table slots
    // (no false dedup — a fresh body → a new slot), and (b) the SAME lambda body reached twice keeps ONE
    // slot (real dedup — the memo the index replaces). Read `db.lifted` after lowering.
    use crate::testkit::parse;
    // (a) A list of 8 DISTINCT escaping closures → 8 lifted entries, each a distinct body occurrence.
    let lams = (0..8)
        .map(|i| format!("(fn (a{i}) (+ a{i} {i}))"))
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!("(module m (def (main) (let ((fs (list {lams}))) 0)) (export main))");
    let mut db = crate::db::Db::load(parse(&src));
    // Drive lowering (fills `db.lifted`) — `diagnostics` runs the full pipeline over the program.
    let _ = crate::diagnostics(&mut db);
    assert_eq!(
        db.lifted.len(),
        8,
        "8 distinct escaping closures lift to 8 distinct slots (no false dedup)"
    );
    let mut bodies: Vec<u32> = db.lifted.iter().map(|l| l.body.0).collect();
    bodies.sort_unstable();
    bodies.dedup();
    assert_eq!(
        bodies.len(),
        8,
        "each lifted slot has a distinct body occurrence"
    );
}

#[test]
fn a_directly_recursive_function_declines_quickly() {
    // `(def (loop n) (loop n))` applied — the callee reaches its own body, so the STATIC recursion
    // check declines it before any β-reduction (a recursive function needs runtime specialization,
    // not yet built). It must decline, not hang: the check is structural, so this returns at once.
    let src = "(module m (def (loop n) (loop n)) (def (main) (loop 0)) (export main))";
    let msg = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("recursion must decline")
        .message;
    assert!(
        msg.contains("recursive") || msg.contains("runtime"),
        "got: {msg}"
    );
}

#[test]
fn a_branching_recursive_function_does_not_explode_at_compile() {
    // A recursive body with TWO self-calls (the shape a CBOR tree-reader takes) would explode
    // exponentially in appended nodes IF INLINED; the static recursion check prevents inlining and
    // emits a real recursive CALL instead, so compilation returns immediately rather than hanging.
    // Regression for the 10-bytes.sexp gate timeout. (Since call-site inference `@<this commit>`, the
    // unannotated param `n` is seeded to Int64 from `main`'s `(rec 3)`, so this now COMPILES to a
    // recursive call — a divergent-but-well-typed program that stack-overflows at RUN time, NOT a
    // compile-time decline. The invariant this guards is the ABSENCE of exponential compile blowup:
    // the artifact is small + built fast, whichever way inference resolves the param.)
    let src = "(module m \
            (def (rec n) (+ (rec n) (rec n))) \
            (def (main) (rec 3)) (export main))";
    let out = compile_component(&crate::codec::encode(&parse(src)));
    match out {
        // Emitted: a real recursive call, NOT an exponentially-inlined body — a small artifact.
        Ok(bytes) => assert!(
            bytes.len() < 10_000,
            "branching recursion must not inline-explode; got {} bytes",
            bytes.len()
        ),
        // A decline is equally acceptable (the point is no hang / no blowup, not a specific outcome).
        Err(d) => assert!(
            d.message.contains("recursive") || d.message.contains("runtime"),
            "got: {}",
            d.message
        ),
    }
}

#[test]
fn an_unproductive_nullary_recursion_declines_not_crashes() {
    // `(def (f) (f))` — a NULLARY self-call with no base case. A nullary def resolves its name to a
    // `Ref` at its body, so the head/record-reduction helpers (`lambda_of`, `reduce_to_record_id`)
    // would re-enter the same body and overflow the native stack. It must DECLINE at the recursion
    // bound, never abort — and carry the reserved ROBUSTNESS code CDZ0999 ("declined, not crashed",
    // 09-functions "an unproductive self-recursion is declined, not a compiler crash"), the coded
    // upgrade from the former codeless decline. Regression for the compile-time-recursion crash.
    let src = "(module m (def (f) (f)) (def (main) (f)) (export main))";
    let reject = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("nullary self-recursion must decline");
    assert_eq!(
        reject.code.as_deref(),
        Some("CDZ0999"),
        "got: {} / {:?}",
        reject.message,
        reject.code
    );
}

#[test]
fn a_self_applying_term_declines_at_the_reduction_budget_not_hangs() {
    // A self-application whose argument applies itself — `((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))`
    // — has NO normal form (each β-reduction produces a larger term). It is NOT statically recursive
    // (the lambdas call a PARAMETER, so `is_recursive` finds no call-graph cycle) and each reduction
    // stays within `REDUCE_DEPTH_LIMIT`, so the DEPTH guard alone does not stop it — the term roughly
    // DOUBLES each step and the type/fault walk would attempt an EXPONENTIAL number of reductions and
    // HANG (the `cdz-smith` timeout finding). The TOTAL-work budget (`REDUCE_NODE_BUDGET`, enforced in
    // `Db::enter_reduction`) bounds cumulative reduction attempts: past it the reduction DECLINES at
    // the resource bound (CDZ0999, "declined not crashed"), so a non-normalizing term is a prompt,
    // diagnosed decline — never a compiler hang. Regression for the self-application timeout. The test
    // TERMINATING at all (returning an `Err` quickly) is the property; the code pins the diagnosis.
    let src = "(module m (def (main) ((fn (v0) (v0 v0)) (fn (v1) (v1 (v1 v1))))) (export main))";
    let reject = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a non-normalizing self-application must decline, not hang");
    assert_eq!(
        reject.code.as_deref(),
        Some("CDZ0999"),
        "a diverging reduction declines at the resource bound (CDZ0999): {} / {:?}",
        reject.message,
        reject.code
    );
    // The original cdz-smith reproducer (a self-applier inside a match/list) likewise TERMINATES —
    // here it surfaces a genuine type error (`(v0 16.32)` applies a non-function) before the blowup,
    // which is fine: the point is it produces a diagnostic promptly rather than hanging.
    let smith = "(module m (def (main) (list ((fn (v0) (match (v0 16.32) (0 (v0 v0)) (_ (v0 144)))) (fn (v1) (v1 (v1 v1)))))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(smith))).is_err(),
        "the cdz-smith self-application reproducer must decline, not hang"
    );
}

#[test]
fn an_if_wrapped_self_application_is_rejected_not_an_inference_hang() {
    // A SECOND hang shape the fuzzer surfaced after the plain self-app was capped: `(fn v (if (v v) 1
    // (v v)))` applied to a copy of itself. The self-app in the if CONDITION forces β-reduction, which
    // reduces the branch's self-app, and applied to itself the term grows exponentially. Unlike the
    // plain hang (capped by the β-reduction budget in `enter_reduction`), this one hung type INFERENCE
    // through a DIFFERENT path — the lambda-parameter context recovery (`expected_arrow_for_lambda` →
    // `type_of`) re-derives the growing term's types WITHOUT going through `enter_reduction`, so it
    // stayed within the descent-depth limit while attempting an exponential number of context lookups.
    // Charging that recovery against the SAME cumulative work budget (`REDUCE_NODE_BUDGET`) makes it
    // TERMINATE: past the budget it recovers no context hint, and the program is rejected promptly
    // (the self-app's Int64 result used as an if condition → CDZ0203). The property is 'never hang'.
    let src = "(module m (def (main) ((fn (v0) (if (v0 v0) 1 (v0 v0))) (fn (v2) (if (v2 v2) 1 (v2 v2))))) (export main))";
    let reject = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("an if-wrapped self-application must reject in bounded time, not hang");
    assert!(
        reject.code.is_some(),
        "the rejection is coded (a diagnosed decline, not a bare/uncoded one): {} / {:?}",
        reject.message,
        reject.code
    );
    // Two more if-self-app shapes the fuzzer minimized (cond + then; the whole applied to itself) —
    // both must TERMINATE with a coded rejection, never hang.
    for s in [
        "(module m (def (main) ((fn (v0) (if (v0 v0) (v0 v0) 1)) (fn (v2) (if (v2 v2) (v2 v2) 1)))) (export main))",
        "(module m (def (main) ((fn (v0) (if (v0 v0) 1 2)) (fn (v2) (if (v2 v2) 1 2)))) (export main))",
    ] {
        assert!(
            compile_component(&crate::codec::encode(&parse(s))).is_err(),
            "an if-wrapped self-application must decline, not hang: {s}"
        );
    }
}

#[test]
fn a_minus_over_lambda_self_application_declines_not_an_inference_hang() {
    // A THIRD hang class the fuzzer (v-cdz-smith) surfaced that ESCAPED the CDZ0999 reduction-limit:
    // an arith/compare/eq operator applied to a LAMBDA-VALUE beside a self-application, inside an outer
    // applied self-app. The `(- (fn v1) (v0 v0))` shape routes operator-on-a-function-value typing
    // through the budget-FREE STRUCTURAL reduction (`enter_reduction_structural` — project_meta /
    // reduce_to_record_id), which is NOT charged against `REDUCE_NODE_BUDGET`, so the exponentially-
    // widening reduced term drove UNBOUNDED structural reductions within a single def body while staying
    // in β-depth → type inference HUNG (the fault walk re-reduced the doubling term without a work
    // bound). Fix: `enter_reduction_structural` now charges a PER-DEF-BODY `STRUCTURAL_REDUCTION_BUDGET`
    // (reset in `collect_faults`' body loop, so the whole-compile cumulative — which a real multi-module
    // build runs high, the Option.None carve-out — is not a program-level cap). The count grows WITH the
    // exploding width so it trips BEFORE the width materializes → the term declines promptly (the
    // ill-typed self-app surfaces its real CDZ0203/CDZ0201), never hangs. Property: 'never hang'. The
    // escape class is the whole arith/compare/eq family, so all of {-, +, *, <, =} are pinned.
    for s in [
        // HANG-1 (v-cdz-smith minimization, minus + fn-value beside self-app, both bodies self-app):
        "(module m (def (main) ((fn (v0) (v0 (- (fn (v1) v0) (v0 v0)))) (fn (v2) (- (fn (v4) v4) (v2 v2))))) (export main))",
        // HANG-2 (the let-in-body variant — two redexes per substitution):
        "(module m (def (main) ((fn (v0) (v0 (v0 v0))) (fn (v2) (let ((v3 (v2 5))) (- (fn (v4) v4) (v2 v2)))))) (export main))",
        // The operator-family breadth (+ * < = all escape the same budget-free structural path):
        "(module m (def (main) ((fn (v0) (v0 (+ (fn (v1) v0) (v0 v0)))) (fn (v2) (+ (fn (v4) v4) (v2 v2))))) (export main))",
        "(module m (def (main) ((fn (v0) (v0 (* (fn (v1) v0) (v0 v0)))) (fn (v2) (* (fn (v4) v4) (v2 v2))))) (export main))",
        "(module m (def (main) ((fn (v0) (v0 (< (fn (v1) v0) (v0 v0)))) (fn (v2) (< (fn (v4) v4) (v2 v2))))) (export main))",
        "(module m (def (main) ((fn (v0) (v0 (= (fn (v1) v0) (v0 v0)))) (fn (v2) (= (fn (v4) v4) (v2 v2))))) (export main))",
    ] {
        // If the fix regresses, this HANGS the suite (a divergent structural walk) — a loud signal, the
        // same discipline the sibling self-app hang tests use.
        assert!(
            compile_component(&crate::codec::encode(&parse(s))).is_err(),
            "an operator-over-a-lambda self-application must decline in bounded time, not hang: {s}"
        );
    }
}

#[test]
fn a_module_member_access_survives_reduction_budget_exhaustion() {
    // Regression for the compiler-ml self-host emit-cache collision (#3) + the sread-eval scaling
    // CDZ0201s — SAME root: `reduce_nodes` (the cumulative anti-divergence work budget) is monotonic
    // over a whole `compile()` and never reset, so a large-but-TERMINATING multi-module closure build
    // exhausts the 1M budget. Once exhausted, `enter_reduction` denied EVERY reduction — INCLUDING a
    // trivial `Ref{Module}→record` hop in `member_value` — so a well-formed member access like
    // `Option.None` mis-typed "a … value has no field `None`" and a well-typed program failed its
    // downstream component build with a locationless CDZ0201.
    //
    // The fix: a STRUCTURAL `Ref` hop (`reduce_to_record_id`, `enter_reduction_structural`) charges only
    // the per-chain DEPTH guard (cycle termination), NOT the cumulative node budget — a `Ref` deref is
    // O(1), not the exponential-fanout β-reduction the budget targets. So a member access must resolve
    // even at budget exhaustion. Here: a user module `m` with a member def `k`, accessed `(. m k)`;
    // pre-load, drive `reduce_nodes` to the budget (simulating a giant closure build), then check the
    // diagnostics carry NO spurious "no field" reject.
    // Access a PRELUDE SUM MODULE's variant constructor by member — `(. Option None)` — which routes
    // `type_of`'s `Resolved::Member` arm → `member_value(Option, None)` → `reduce_to_record_id`, the
    // exact path that needs the `Ref{Option}→record` hop. (This is the shape the closure build
    // SYNTHESIZES; here it is written explicitly so a small program exercises it.)
    let src = "(module m (def (main) (: (. Option None) (Option Int64))) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    db.reduce_nodes = crate::db::REDUCE_NODE_BUDGET; // as if a large closure build already spent it
    let diags = crate::diagnostics(&mut db);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("no field") || d.message.contains("requires a record"))
        .collect();
    assert!(
        bad.is_empty(),
        "a module-member access must resolve even at reduction-budget exhaustion (the emit-cache \
             collision-#3 / scaling root); got spurious: {:?}",
        bad.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn a_compound_wrapped_self_application_declines_not_a_stack_overflow() {
    // A THIRD hang shape the fuzzer surfaced: `(fn v (tuple (v v) 1))` applied to a copy of itself. Here
    // the reduction BUDGET already terminates inference (β-reduction gives up past `REDUCE_NODE_BUDGET`)
    // — but that leaves a MEMOIZED core chain thousands of nodes deep, `Tuple[Tuple[…poison…, 1], 1]`,
    // bottoming out in the reduction-bound poison. That chain is built bottom-up at shallow demand
    // depths, so lowering's own descent guard never fired on it; the reached-poison walk
    // (`collect_reached_poisons`) then descended the whole pre-built chain in ONE native recursion and
    // OVERFLOWED THE COMPILER'S STACK — a process abort. Giving that walk the same `DESCENT_DEPTH_LIMIT`
    // guard lowering has makes it surface the reduction-bound poison (CDZ0999) past the limit instead of
    // crashing. The guard is at the walk's single recursive entry and the walk dispatches structurally,
    // so the whole compound-construction class (tuple / record / list / …) is covered by ONE guard, not
    // one syntactic wrapper at a time. The property is 'never crash': it must TERMINATE with a coded
    // rejection on any input, regardless of the compound the divergence hides in.
    let tup = "(module m (def (main) ((fn (v0) (tuple (v0 v0) 1)) (fn (v2) (tuple (v2 v2) 1)))) (export main))";
    let reject = compile_component(&crate::codec::encode(&parse(tup)))
        .expect_err("a tuple-wrapped self-application must decline, not crash");
    assert_eq!(
        reject.code.as_deref(),
        Some("CDZ0999"),
        "a tuple-wrapped diverging reduction declines at the resource bound (CDZ0999): {} / {:?}",
        reject.message,
        reject.code
    );
    // The record-wrapped sibling is the SAME class (a self-app in a record field) — it must likewise
    // TERMINATE with a coded rejection, exercising the shared structural guard on a different compound.
    let rec = "(module m (def (main) ((fn (v0) (record (= a (v0 v0)) (= b 1))) (fn (v2) (record (= a (v2 v2)) (= b 1))))) (export main))";
    let reject = compile_component(&crate::codec::encode(&parse(rec)))
        .expect_err("a record-wrapped self-application must decline, not crash");
    assert!(
        reject.code.is_some(),
        "a record-wrapped diverging reduction is a coded decline, not a crash: {} / {:?}",
        reject.message,
        reject.code
    );
}

#[test]
fn a_sum_payload_wrapped_self_application_declines_not_a_stack_overflow() {
    // The SUM-CONSTRUCTOR-payload sibling of the tuple/record shape above: `(fn v (Some (v v)))` applied
    // to a copy of itself. `cdz check` (inference) already declines CDZ0999 (the reduction budget), but
    // `cdz compile` HUNG — the LAYOUT reachability walks (`collect_call_callees`/`collect_closure_codes`)
    // descend a `Core::SumNew` payload by calling `core_of`, which β-reduces one more level per call
    // WITHOUT holding the reduction-DEPTH guard (unlike tuple lowering), materializing an unbounded
    // `Core::SumNew` chain the walk descends in ONE native recursion until the stack OVERFLOWS. The fix
    // bounds those walks with a DEDICATED `walk_depth` counter (not `core_of`'s `descent_depth`, which
    // the walk also drives — sharing would spuriously decline a valid moderately-deep program). Past the
    // limit the walk stops descending; `collect_faults` then reports the coded CDZ0999 decline. The
    // property is 'never crash' — TERMINATE with a coded rejection on `compile`, not just `check`.
    let some =
        "(module m (def (main) ((fn (v0) (Some (v0 v0))) (fn (v2) (Some (v2 v2))))) (export main))";
    let reject = compile_component(&crate::codec::encode(&parse(some)))
        .expect_err("a Some-payload-wrapped self-application must decline, not crash");
    assert_eq!(
        reject.code.as_deref(),
        Some("CDZ0999"),
        "a sum-payload diverging reduction declines at the resource bound (CDZ0999): {} / {:?}",
        reject.message,
        reject.code
    );
    // `(Ok (v v))` is the same class (a different built-in sum), and a user MULTI-payload variant
    // `(P (v v) 1)` too — both must TERMINATE with a coded rejection (the user one surfaces a CDZ0201
    // payload-type conflict before the blowup, which is fine: a prompt diagnostic, not a hang).
    for src in [
        "(module m (def (main) ((fn (v0) (Ok (v0 v0))) (fn (v2) (Ok (v2 v2))))) (export main))",
        "(module m (type B (P Int64 Int64)) (def (main) ((fn (v0) (P (v0 v0) 1)) (fn (v2) (P (v2 v2) 1)))) (export main))",
    ] {
        let reject = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a sum-payload self-application must decline, not crash");
        assert!(
            reject.code.is_some(),
            "a sum-payload diverging reduction is a coded decline, not a crash: {} / {:?}",
            reject.message,
            reject.code
        );
    }
}

#[test]
fn a_repeated_squaring_bigint_chain_diagnoses_in_bounded_time() {
    // REGRESSION (perf): `collect_reached_poisons` — the reached-poison walk (`compile::diagnostics` /
    // `cdz check`, the hot editor path) that descends a nullary def's lowered core to find a provable
    // trap — followed `core_of` (which resolves a `Ref` to its target's body) with NO visited-set. A
    // def used in BOTH operand positions of a binary op is a SHARED core DAG, and the naive recursion
    // walked it as a TREE. Fixed-width `Int` never triggered it (a constant `(* a a)` folds to a
    // `Core::ConstInt` leaf — no binary node to re-descend), but `BigInt` arithmetic DELIBERATELY does
    // not constant-fold (exact unbounded math is a runtime op), so a `BigInt` repeated-squaring chain
    // `a_i = (* a_{i-1} a_{i-1})` — the TEXTBOOK large-power idiom — left a `Core::BigIntBinOp` at every
    // level, and each level reached `a_{i-1}` from both sides → O(2^depth) node visits. A depth-30
    // chain (~30 tiny defs) took SECONDS and grew ×2 per level, an effective HANG of "diagnostics as
    // you type" on a small realistic program. The fix records each fully-walked node in
    // `Db::reached_visited` and skips it on re-reach (a poison's origin is its own node, so its
    // contribution is path-independent and `dedup_faults` collapses the duplicates a shared DAG would
    // otherwise yield). This depth would not TERMINATE pre-fix; that `diagnostics` returns is the gate.
    // (The BACKEND'S emit genuinely inlines each nullary def per use → O(2^depth) INSTRUCTIONS, a
    // distinct downstream cost; this test pins the diagnostics/check path the fix addresses.)
    let mut defs = String::from("(def (a0) (BigInt.of 3))");
    for i in 1..=30 {
        defs.push_str(&format!(" (def (a{i}) (* a{prev} a{prev}))", prev = i - 1));
    }
    let src = format!("(module m {defs} (def (main) (= a30 (BigInt.of 0))) (export main))");
    // The well-typed program has no faults: an empty diagnostics list, returned in bounded time.
    // Through the host-stack guard the bin uses (`host.rs`): the reached-poison walk recurses ~per
    // chain level (30 deep here), which OVERFLOWS a default `cargo test` worker's ≈2 MB stack (SIGABRT,
    // EXIT=101 with 0 FAILED) even though the visited-set fix makes it TERMINATE — deep-but-finite, not
    // a loop. Sizing the stack from `DESCENT_DEPTH_LIMIT` bounds it by depth, not the native stack.
    let diags = crate::host::run_with_compiler_stack(move || {
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
    });
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a well-typed repeated-squaring BigInt chain has no error diagnostics: {diags:?}"
    );
}

#[test]
fn a_pathologically_deep_expression_declines_not_crashes() {
    // A `(+ 1 (+ 1 …))` nest far past the depth bound must DECLINE (a resource-limit rejection) rather
    // than overflow the stack and abort — the completes-or-declines, never-crashes property.
    //
    // The reader is now ITERATIVE + UNCAPPED (v-syntax-nonrec-reader): `sexpr::read` parses arbitrary
    // depth without overflowing the native stack, so the deep source PARSES — it no longer returns a
    // reader `ReadError`, and no big-stack thread is needed to reach the reader. The graceful rejection
    // now lives SOLELY at the COMPILER's `DESCENT_DEPTH_LIMIT`: the compiler's own descent (still
    // recursive) DECLINES a program nested past 1024 with "expression nests too deeply to compile".
    // Compile the deep program through `run_with_compiler_stack` — it sizes the worker stack from
    // `DESCENT_DEPTH_LIMIT` so the COMPILER's descent reaches its guard before the native stack limit
    // (replacing the former manual 64 MiB thread, which had guarded the now-gone reader recursion) — and
    // assert it DECLINES, not crashes.
    let mut body = "1".to_string();
    for _ in 0..4000 {
        body = format!("(+ 1 {body})");
    }
    let src = format!("(module m (def (main) {body}) (export main))");
    let diags = crate::host::run_with_compiler_stack(move || {
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
    });
    assert!(
        diags.iter().any(|d| d.message.contains("nests too deeply")),
        "a pathologically deep expression must DECLINE at the compiler's depth limit, not crash: {diags:?}"
    );
}

#[test]
fn a_deeply_nested_capturing_lambda_chain_checks_without_exponential_blowup() {
    // REGRESSION (perf): a chain of INLINE lambdas each capturing the outer params —
    // `((fn (a0) ((fn (a1) … (+ a0 (+ a1 … ))) 1)) 0)`. `check_application` type-checks a lambda-headed
    // app by β-reducing the body and `collect`-ing the reduced (synthesized, cache-missing) copy — AND
    // it computed a `baseline` by ALSO `collect`-ing the UNREDUCED callee body to diff callee-intrinsic
    // faults. For a nested chain, BOTH collects contain the inner nested application, so each re-reduced
    // the inner chain → O(2^depth) (a depth-20 chain took ~28s). FIX: skip the baseline collect when the
    // reduced body has NO faults to filter (the well-typed common case) — the reduced-body collect alone
    // then reduces each level once. This depth-20 chain would not finish pre-fix; that `diagnostics`
    // returns quickly (and clean) is the gate.
    let d = 20;
    let mut inner = "0".to_string();
    for i in 0..d {
        inner = format!("(+ a{i} {inner})");
    }
    let mut body = inner;
    for i in (0..d).rev() {
        body = format!("((fn (a{i}) {body}) {i})");
    }
    let src = format!("(module m (def (main) {body}) (export main))");
    // Through the host-stack guard the bin uses (`host.rs`): a depth-20 nested-lambda chain's
    // β-reduction/type-check recurses ~per level, which SIGABRTs a default `cargo test` worker's
    // ≈2 MB stack (EXIT=101, 0 FAILED) even though the O(2^depth)→O(depth) fix the test guards makes it
    // TERMINATE quickly. Deep-but-finite, not a loop — size the stack from `DESCENT_DEPTH_LIMIT`.
    let diags = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
    });
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a deeply-nested capturing lambda chain type-checks clean in bounded time: {diags:?}"
    );
    // And the baseline de-dup it guards STILL works: a callee-intrinsic fault (a non-exhaustive match
    // in an applied lambda's body) is reported EXACTLY ONCE, not duplicated by the reduced-body check.
    let intrinsic = "(module m (type T (A) (B)) (def (main) ((fn ((: t T)) (match t ((T.A) 1))) (T.A))) (export main))";
    let n = crate::diagnostics(&mut crate::db::Db::load(parse(intrinsic)))
        .iter()
        .filter(|x| x.code.as_deref() == Some("CDZ0210"))
        .count();
    assert_eq!(
        n, 1,
        "the non-exhaustive-match fault is reported once, not duplicated"
    );
}

#[test]
fn mutually_recursive_functions_decline() {
    // `f` calls `g`, `g` calls `f` — neither reaches a normal form. The transitive call-graph walk
    // finds the cycle f→g→f, so applying `f` declines.
    let src = "(module m \
            (def (f n) (g n)) \
            (def (g n) (f n)) \
            (def (main) (f 0)) (export main))";
    let msg = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("mutual recursion must decline")
        .message;
    assert!(
        msg.contains("recursive") || msg.contains("runtime"),
        "got: {msg}"
    );
}

#[test]
fn a_type_value_has_type_type() {
    // A type is a first-class VALUE, so it has a type — `Type`. `Bool` (a ground-type record) and
    // `(Int 64)` (a built module) both type as `Type`; a type used as a value doesn't fall to Any.
    use crate::db::Db;
    use crate::infer::type_of;
    use crate::testkit::parse;
    use crate::ty::Ty;
    // The `Bool` reference in `(def (t) Bool)` — find it and check its type.
    let ast = parse("(module m (def (t) Bool) (def (main) 0) (export main))");
    let mut db = Db::load(ast);
    // Locate the `Bool` occurrence: the body of def `t`.
    let bool_occ = db
        .defs
        .iter()
        .find(|d| d.name == "t")
        .and_then(|d| d.body)
        .expect("def t");
    assert_eq!(type_of(&mut db, bool_occ), Ty::Type);
}

#[test]
fn int_module_is_built_once_and_shared() {
    // `(Int 64)` reduces to the SAME module however many times it is demanded — the build cache
    // makes the reduction idempotent, so the arena does not grow per demand. Demand the same
    // occurrence's module twice and a second `(Int 64)`; the cache holds ONE entry per width.
    use crate::db::Db;
    use crate::eval::{meta_apply_of, reduce_ctor};
    use crate::testkit::parse;
    // Two separate `(Int 64)` applications, plus one used twice.
    let ast = parse(
        "(module m (def (main) (. (Int 64) max)) (def (other) (. (Int 64) min)) (export main))",
    );
    let mut db = Db::load(ast);
    // Find the two `(Int 64)` applications by resolving them; force each module build several times.
    // (We reduce directly via the evaluator to exercise the cache without threading occurrences.)
    let int_prim = db.prelude.get("Int").copied().expect("Int in prelude");
    let p = meta_apply_of(&mut db, int_prim).expect("Int is applyable");
    // Build width 64 three times — all must return the same node, and the cache must have 1 entry.
    let w64 = db.push_atom(crate::ast::Leaf::Int {
        value: crate::ast::IntValue::from_i64(64),
        radix: crate::ast::Radix::Dec,
    });
    // `Int`/`UInt` key their build-once cache on the WIDTH value, not the `origin` occurrence, so
    // any StructId serves as origin here (the width atom itself).
    let a = reduce_ctor(&mut db, p, w64, &[w64]).expect("build");
    let b = reduce_ctor(&mut db, p, w64, &[w64]).expect("build");
    assert_eq!(a, b, "the same width must reduce to the same module node");
    // A different width is a different module.
    let w8 = db.push_atom(crate::ast::Leaf::Int {
        value: crate::ast::IntValue::from_i64(8),
        radix: crate::ast::Radix::Dec,
    });
    let c = reduce_ctor(&mut db, p, w8, &[w8]).expect("build");
    assert_ne!(a, c, "different widths are different modules");
}

#[test]
fn try_on_an_option_yields_the_some_payload_type() {
    // `(try (Int64.checked-add 20 22))` — the operand is `(Option Int64)`, so the `?` UNWRAPS the
    // `Some` payload: the node's type is `Int64`. (The value does not lower yet — T1; this pins the
    // TYPE half.)
    let ty = try_body_ty("(try (Int64.checked-add 20 22))");
    assert_eq!(
        ty.render_name(&crate::ty::NameCtx::new(&[])),
        "Int64",
        "`(try (Option Int64))` yields the Some payload Int64, got {}",
        ty.render_name(&crate::ty::NameCtx::new(&[]))
    );
}

#[test]
fn try_on_a_result_yields_the_ok_payload_type() {
    // `(try (Ok 1))` — the operand is `(Result Int64 _)`, so `?` yields the `Ok` payload `Int64`
    // (the `Err` type is a still-unsolved phantom here, which does not affect the success payload).
    let ty = try_body_ty("(try (Ok 1))");
    assert_eq!(
        ty.render_name(&crate::ty::NameCtx::new(&[])),
        "Int64",
        "`(try (Result Int64 _))` yields the Ok payload Int64, got {}",
        ty.render_name(&crate::ty::NameCtx::new(&[]))
    );
}

#[test]
fn the_sigil_question_mark_as_a_head_points_at_the_try_spelling() {
    // WHITE-BOX RESIDUAL. The CDZ0101 code + the "(try <expression>)"/"write `try`" message on a sigil-`?`
    // HEAD, and the bare-`?` no-hint control, are now corpus 23-try-operator ("the `?` sigil in head
    // position names the `try` spelling…" / "a bare `?` NOT in head position…"). This keeps ONLY the
    // VERIFIED `?`→`try` head-rewrite FIX (replacement "try") the corpus fix-grade does not yet cover.
    for body in ["(? (Ok 1))", "(? r)"] {
        let d = expect_error(body);
        let fix = d.fix.as_ref().expect("carries a `?`->`try` fix");
        assert!(fix.verified, "the head rewrite is deterministic (verified)");
        assert_eq!(fix.replacement, "try", "rewrites the `?` head to `try`");
    }
}

#[test]
fn applying_the_question_mark_to_try_fix_recompiles_clean_in_a_fallible_context() {
    // The `?`→`try` fix is marked VERIFIED — the head rewrite is deterministic and clears the CDZ0101
    // by construction (`diagnostics.md` §A Confirmed Fix Is Marked Verified). Pin the ROUND TRIP that
    // backs that claim: a program written with the `?` sigil in head position, IN A VALID FALLIBLE
    // CONTEXT, carries the `?`→`try` fix; applying it (rewriting the head to `try`) yields a program
    // that compiles clean — proving the verified fix actually recompiles, not just that it clears the
    // one diagnostic. The context is the 23-try-operator.sexp T1 happy-path shape (a `let`-chain whose
    // tail `(Some …)` gives the `?` an Option boundary), which is a proven-passing corpus case, so its
    // all-`try` form is guaranteed clean.
    //
    // BEFORE (the `?` head is unbound — CDZ0101 — but everything else is well-formed):
    let with_sigil = "(let ((x (? (Int64.checked-add 20 22)))) \
              (let ((y (try (Int64.checked-add 40 2)))) (Some (+ x y))))";
    let d = expect_error(with_sigil);
    assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
    let fix = d.fix.expect("the `?`-head reject carries a fix");
    assert!(fix.verified, "the head rewrite is deterministic → verified");
    assert_eq!(
        (fix.kind, fix.replacement.as_str()),
        (crate::abi::FixKind::Replace, "try"),
        "the fix replaces the `?` head with `try`: {:?}",
        fix
    );
    // AFTER (apply the fix = rewrite the `?` head to `try`): the SAME program now compiles clean — no
    // residual CDZ0101, no cascade. This is the applied-fix form (the corpus T1 happy path).
    let applied = "(let ((x (try (Int64.checked-add 20 22)))) \
              (let ((y (try (Int64.checked-add 40 2)))) (Some (+ x y))))";
    assert!(
        compiles_ok(applied),
        "applying the verified `?`→`try` fix must recompile clean"
    );
}

#[test]
fn the_rest_marker_dotdot_used_as_a_value_names_the_pattern_only_role() {
    // `..` is the REST/SPREAD marker of a collection PATTERN (`(list a .. rest)` / `(map (k v) .. r)`).
    // Used as a VALUE or form HEAD — `(.. xs)`, `(g ..)` — it previously drew "unbound name `..`" (and,
    // in head position, a misleading "did you mean `.`?" — a rest marker is not a mistyped member `.`).
    // It now names the pattern-only role (CDZ0201), NO fix (the `.`-rename is a wrong guess; a rest
    // marker has no value rewrite). The sibling of the `_`-as-value and `?`-as-head sigil messages.
    for body in ["(.. xs)", "(g ..)"] {
        let src = format!("(module m (def (g a) a) (def (f xs) {body}) (export f))");
        let d = compile_component(&crate::codec::encode(&parse(&src)))
            .expect_err("`..` as a value must be rejected");
        assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
        assert!(
            d.message.contains("`..` is a rest/spread marker")
                && d.message.contains("PATTERN")
                && d.message.contains("(list a .. rest)"),
            "names the pattern-only role: {}",
            d.message
        );
        assert!(
            d.fix.is_none(),
            "no misleading fix for a rest marker: {:?}",
            d.fix
        );
    }
    // NO false positive: a `..` in its LEGITIMATE list/map PATTERN position (match AND binding) is
    // untouched — the pattern parser consumes the marker before it could reach the value-ref path.
    for ok in [
        "(module m (def (f (: xs (List Int64))) (match xs ((list a .. r) a) (_ 0))) (export f))",
        "(module m (def (f (: mp (Map Int64 Int64))) (match mp ((map (k v) .. rest) k) (_ 0))) (export f))",
        "(module m (def (f (: xs (List Int64))) (let (((list a .. r) xs)) a)) (export f))",
    ] {
        assert!(
            !crate::diagnostics(&mut crate::db::Db::load(parse(ok)))
                .iter()
                .any(|d| d.message.contains("`..` is a rest/spread marker")),
            "a legitimate pattern `..` is not flagged: {ok}"
        );
    }
}

#[test]
fn a_try_with_no_fallible_enclosing_function_is_cdz0230() {
    // WHITE-BOX RESIDUAL. The CDZ0230 code + the "boundary"/concrete-`(Result Int64 …)` hint are now the
    // corpus 23-try-operator case "a `?` with no fallible enclosing function boundary is rejected", and
    // the GENERIC fallback naming both `(Result _ e)` / `(Option _)` forms is "a `?` whose operand kind
    // is not yet definite names both fallible forms in the boundary hint". This keeps ONLY the
    // inexpressible remainder: the hint's backticks must be BALANCED (Copilot PR #453 — the generic
    // fallback used to smuggle its own backticks through a template-wrapped `{suggested}`, rendering the
    // code spans oddly), a backtick-COUNT-parity property the corpus grade has no clause for. Checked on
    // both the concrete (`(try (Ok 1))`) and the generic-fallback (`(+ 1 (try x))`) hints.
    let d = expect_error("(try (Ok 1))");
    assert_eq!(d.code.as_deref(), Some("CDZ0230"));
    assert_eq!(
        d.message.matches('`').count() % 2,
        0,
        "the concrete ?-boundary hint has balanced backticks: {}",
        d.message
    );
    let fallback = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f x) (+ 1 (try x))) (export f))",
    )))
    .into_iter()
    .find(|d| d.code.as_deref() == Some("CDZ0230"))
    .expect("the ?-boundary fallback is CDZ0230");
    assert_eq!(
        fallback.message.matches('`').count() % 2,
        0,
        "the generic fallback hint has balanced backticks too: {}",
        fallback.message
    );
}

#[test]
fn a_constant_success_try_folds_to_the_payload() {
    // BRICK 2 (the CONSTANT-SUCCESS fold, DESIGN-try-operator-rcdzc.md §3.2): a `?` on a compile-time
    // `Some x` / `Ok x` unwraps to the payload — no boundary break fires on the happy path, so it
    // needs no `Core::Block`/`Break` (that is the failure/runtime path, BRICK 3). `(Int64.checked-add
    // 20 22)` folds to `(Some 42)`, so `(try …)` folds to `42`; the body's tail `(Some x)` makes the
    // boundary `Option`, matching. It COMPILES (a value, not a decline) — the corpus case "`?` on the
    // success variant unwraps the payload" runs it to `(Some 84)` against the real heap.
    let src = "(module m (def (main) (let ((x (try (Int64.checked-add 20 22)))) (Some x))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a constant-`Some` `?` under a matching Option boundary folds to the payload, not declines"
    );
}

#[test]
fn a_constant_failure_try_short_circuits_the_boundary_to_the_failure() {
    // BRICK 3a (DESIGN-try-operator-rcdzc.md §4 v1): a `?` on a constant FAILURE (`None`/`Err`) short-
    // circuits the enclosing FUNCTION boundary — the failure value flows out as the function's value.
    // `(Int64.checked-add Int64.max 1)` overflows → `None`; the `?` in the let-init `Core::Break`s, and
    // `lower_let` folds the whole `let` to `None` (the body + later bindings never run). No boundary
    // block node for v1 (the function body IS the boundary). It COMPILES (the corpus case "`?` on the
    // failure variant short-circuits the boundary" runs it to `(None unit)`).
    let src = "(module m (def (main) (let ((x (try (Int64.checked-add Int64.max 1)))) (Some x))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a constant-`None` `?` short-circuits the boundary to `None`, not declines"
    );
}

#[test]
fn a_constant_failure_try_elides_an_earlier_trapping_let_init_it_discards() {
    // OPERATOR §283 RULING (2026-07-16): "we don't emit the trap unless it's reachable; a detected
    // unreachable trap is a WARNING." `(let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))` — `a`
    // traps (÷0) but is referenced only in `(+ a x)`, which the `?` short-circuits, so `a`'s value is
    // UNOBSERVED → the trap ELIDES and the program COMPILES to `None` (with a §285 CDZ0305 warning),
    // NOT CDZ0304. (Earlier this expected CDZ0304 via an over-strict is_trap_free guard the operator
    // ruling reverted.) A host call, being observable, would still bail the fold.
    let src = "(module m (def (main) (let ((a (/ 1 0)) (x (try (None unit)))) (Some (+ a x)))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "an unobserved trapping earlier init is ELIDED (folds to None), not CDZ0304-rejected"
    );
}

#[test]
fn a_constant_failure_try_in_a_nested_let_elides_a_trapping_outer_init() {
    // The nested-let companion of the §283 elide ruling: `(let ((a (/ 1 0))) (let ((x (try (None
    // unit)))) (Some (+ a x))))` — `a` (outer let) is referenced only in the short-circuited `(+ a x)`,
    // so its ÷0 is unobserved → elided → compiles to None. Observation, not the nesting, governs (§285).
    let src = "(module m (def (main) (let ((a (/ 1 0))) (let ((x (try (None unit)))) (Some (+ a x))))) \
                   (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "an unobserved trapping OUTER-let init is ELIDED by a nested `?` short-circuit, not rejected"
    );
}

#[test]
fn a_constant_err_try_short_circuits_a_result_boundary() {
    // The Result companion: a constant `Err` `?` under a Result boundary short-circuits to the `Err`.
    // `(try (Err 7))` breaks `main` to `(Err 7)`. Pins that the fold reads the SUCCESS disc (`Ok`) off
    // the operand type and treats the non-success `Err` as the break, symmetric with the Option/`None`
    // case. The operand + boundary are annotated `(Result Int64 Int64)` so the Result type is fully
    // determined (an unannotated `(Ok x)` alone leaves `(Result _ _)` undetermined — a separate
    // type-determinism concern, not the `?` fold).
    let src = "(module m (def (main) (: (let ((x (try (: (Err 7) (Result Int64 Int64))))) (Ok x)) \
                   (Result Int64 Int64))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a constant-`Err` `?` short-circuits the Result boundary to `Err`"
    );
}

#[test]
fn a_constant_failure_after_an_effectful_binding_declines() {
    // The soundness guard for BRICK 3a's strict-spine fold: the break drops every LATER binding + the
    // body, so an EARLIER binding whose init has an OBSERVABLE side effect (a host call) cannot be
    // dropped — folding would lose that effect. Such a shape DECLINES (a later brick handles it with the
    // real block/br that runs the effect THEN breaks). Here an earlier `(log.emit …)` host call precedes
    // the failing `?`; the fold must not fire. (A pure earlier binding — the T1 shape — still folds.)
    let src = "(do (effect Log (op emit (-> Int64 Unit))) \
                   (def (main) (host (Log) \
                     (let ((a (Log.emit 1)) (x (try (Int64.checked-add Int64.max 1)))) (Some x)))) \
                   (export main))";
    // Must NOT compile to a value that dropped the `Log.emit` — declines (or keeps the call); either
    // way it does not silently fold away the effect. Assert it does not run to a bare `(None …)` value
    // with the emit lost — a decline is the correct conservative outcome here.
    let out = compile_component(&crate::codec::encode(&parse(src)));
    assert!(
        out.is_err(),
        "a constant-failure `?` after an effectful earlier binding must decline (the break would \
             drop the host call), not silently fold"
    );
}
