# DESIGN — Closures across the host boundary as component resources

**Status:** proposed (design + increment plan; no code yet)
**Author direction:** "make it so we can pass closures to the host and they can pass
them back. we'd generate a resource that represents the signature of the closure. this
would be _incredibly_ powerful."

## The idea in one sentence

A Cadenza closure that crosses the component boundary becomes a **component-model
resource whose rep is the closure's heap-cell handle**, monomorphized per closure
signature `(-> A B)`, exposing a **`call` method** that internally does the exact
`call_indirect` the compiler already emits for an intra-program `Core::CallClosure`. The
host can hold the resource opaquely, invoke it, and hand it back into another Cadenza
export — a first-class callback.

This turns Cadenza functions into callbacks the host can store and drive: event
handlers, comparators, `map`/`filter`/`fold` bodies supplied to a host-side collection,
visitors, streaming/iterator callbacks, continuations. It is the missing half of
first-class functions — intra-program closures are done (`09-functions.sexp`, ~30 cases);
this extends them across the boundary.

## Why it fits the existing machinery (the key realizations)

1. **A closure is already an i32; a resource rep is always i32.** A runtime closure is a
   value-heap cell `(box-int(code), captures…)`, and its VALUE is the u32 cell handle
   (`lir.rs`: `Ty::Fn → Some(I32)`). A component resource's rep is hard-coded i32
   (`resource_type_item` emits `0x3f 0x7f 0x01 …`). So a closure cell drops into a
   resource rep slot with **no ABI change** — exactly as the value-heap escape wraps a
   compound's u32 handle.

2. **The `call_indirect` is ours, not the host's.** The closure's code lives in our
   funcref table; `Core::CallClosure` emits `resource.rep(self)` is unnecessary
   intra-program, but at the boundary the method body does: recover the cell handle from
   the resource (`resource.rep`), read the table slot (`arr-get(cell,0)` + `get-int`),
   and `call_indirect` over our table with the args. The closure logic never leaves
   Cadenza; **the host just holds a handle and either invokes the `call` method or hands
   the resource back.** This sidesteps the hard problem (a host implementing a Cadenza
   function type) — the host is a *custodian*, not an *implementor*.

3. **The resource-escape path is the template.** `assemble_runtime_resource` +
   `resource_inner_component` already export a resource-with-methods (`make`/`encode`)
   inside `cadenza:run/run`. A closure resource is the same shape with `call` in place of
   `encode` (and possibly additional methods). The byte-emitting item helpers in
   `envelope.rs` (`resource_type_item`, `own_item`, `borrow_item`, `canon_lift_item`, the
   nested re-export component) are directly reusable.

4. **"A resource per signature" = the monomorphization the escape already does.** The
   value-heap escape mints one resource type per concrete compound type
   (`tuple-int64-bool`, etc.). A closure resource is minted per closure SIGNATURE:
   `(-> Int64 Int64)` → resource `closure-s64-s64` with `call: (self, s64) -> s64`;
   `(-> Int64 (-> Int64 Int64))` → `call` takes two args (curried arrows flattened to the
   method's parameter list, exactly as `runtime_fn_spine`/`closure_type_index` flatten a
   full-arity application).

## The two directions

### Direction 1 — Cadenza → host (export a closure; host holds + invokes it)
The primary, tractable direction. A closure value at a boundary export position crosses
as `own<closure-sig>` (or `borrow`). The resource carries a `call` method. The host:
- holds the handle opaquely;
- invokes `call(self, args…) -> result` (dispatches through our `call_indirect`);
- can hand the resource back into another Cadenza export.

### Direction 2 — host → Cadenza (host hands a Cadenza closure back)
A Cadenza export takes a parameter of type `own<closure-sig>`/`borrow<closure-sig>`. The
host passes back a resource **we previously handed it**. Inside, the parameter's rep is
recovered (`resource.rep`) to our cell handle and applied via `Core::CallClosure`'s
`call_indirect` — the type index resolves against the same in-program `LiftedLambda` that
built it (the closure is ours; `closure_type_index` already matches by valtype shape).

**Out of scope (a genuinely-host-IMPLEMENTED function):** the host defining a brand-new
function of a Cadenza signature and Cadenza calling it. That would require a host-side
resource whose `call` is a host closure, dispatched via a component IMPORT rather than our
`call_indirect`. Valuable but strictly larger (import-side resource + a second dispatch
path); explicitly deferred. The round-trip (host as custodian of OUR closure) delivers
most of the power and is tractable now.

## The ABI shape (concrete)

**Key correction (author): the handle ALWAYS originates inside Cadenza.** The host never
fabricates a closure — `resource.new` runs IN THE GUEST, on a cell the guest built, at the
moment a closure value crosses the boundary. So there is **no host-called `make`
constructor** (that was over-copied from the value-heap escape, where `make` builds a
constant compound). A closure crosses as the **RESULT of an ordinary export** — which may
be parameterized, so the handle is genuinely computed from Cadenza running on the host's
inputs. For a def returning `(-> Int64 Int64)`:

```wit
resource closure-s64-s64 {
  call: func(x: s64) -> s64;   // a resource METHOD (self is the borrow/own receiver)
}
adder: func(k: s64) -> own<closure-s64-s64>   // an ORDINARY export; result is the closure
```

- **The export itself is the "constructor."** Its body runs Cadenza (closing over `k`),
  builds the closure cell (`Core::Closure` emit: `arr-alloc`, `box-int(code)`, the
  captures), and — because its RESULT crosses as `own<closure-sig>` — the boundary-return
  emits `resource.new(cell)` to hand the host an owned handle. The `resource.new` is the
  return-value crossing, not a magic method. A nullary export `(def (main) (fn (x) (+ x
  1)))` is just the degenerate (constant-closure) case of the same shape.
- **`call`** — the resource method. Core body: `resource.rep(self)` → cell handle; then
  the `Core::CallClosure` sequence (materialize cell, push env + args, `arr-get(cell,0)` +
  `get-int` + `i32.wrap_i64` + `call_indirect <type>`). The `<type>` is the in-program
  lifted lambda's functype — already computed by `closure_type_index`.
- No `encode` method (that was the value-escape's contract). A closure resource exposes
  `call` (+ a dtor); it is invoked, not serialized.

This differs structurally from the value-heap escape: the escape is a single NULLARY
export routed specially in `emit` (before selection) into a `make`/`encode` resource; a
closure export is an ORDINARY (possibly parameterized) export whose RESULT TYPE is
`Ty::Fn`. So the trigger is not "nullary compound export" but "an export whose result (or,
later, a param) is a closure type" — and the export body lowers normally, with the
`resource.new` spliced at the boundary-return.

### own vs borrow (the load-bearing fork)
The value-heap escape uses `own<t>` for `encode` and documents a KNOWN LEAK + an
un-root-caused wasmtime-37 trap when switched to `borrow<t>` (`envelope.rs:1052-1058`,
`resource_inner_component_borrow` scaffolded but unwired). For a **callable** closure the
host invokes repeatedly, `own` (consume-on-call) is wrong — the host must keep the handle
across calls. So this feature FORCES resolving the `borrow<t>` path that the escape
deferred. Options:
- **(a) `borrow<t>` for `call`, `own<t>` returned by `make`** — host owns the resource,
  lends it per call, drops it when done (correct lifetimes; requires root-causing the
  wasmtime-37 borrow trap first — the escape's open follow-up).
- **(b) `own<t>` per call, host re-obtains** — wrong for a stored callback; rejected.
- **(c) leak deliberately at first** (like the escape's `own` `encode`) — `call` takes
  `borrow` semantics but we model with `own`+no-drop to dodge the trap, accept the bounded
  leak, and fix with the general dtor work. Fastest to green; matches the escape's current
  posture.

Recommendation: **start with (c)** to prove the vertical end-to-end on a green gate
(mirrors how the value-heap escape shipped), then land the `borrow<t>` fix as a shared
increment that also fixes the escape's `encode` leak — one fix, two beneficiaries.

## Type-system surface

- `comp_valtype_of(Ty::Fn)` stays `None` (a closure has no scalar boundary valtype); the
  RESOURCE escape is the boundary form, exactly as compounds decline `comp_valtype_of` but
  escape via a resource. The dispatch guard in `emit` (currently matching Tuple/Record/Sum/
  List/Map/Bytes/String on a single nullary export) gains a `Ty::Fn` arm.
- A closure at a boundary PARAMETER position (Direction 2) is a new param ABI:
  `own<closure-sig>`/`borrow<closure-sig>` recovered via `resource.rep`. Parallels the
  host-call scalar-param path but with a resource handle.
- Monomorphization key = the closure's solved `Ty::Fn` (flattened arg list + result). Two
  exports of the same signature share one resource type; distinct signatures mint distinct
  resources (dedup like the leaf/type sections).

## Host side (cdz-run)

wasmtime 37 (component-model) supports resource methods, `ResourceAny`, and host-defined
resources — all present. `cdz-run` already (a) calls an exported resource's methods
(`run_resource_escape`: `make` then `encode`), and (b) binds host imports dynamically off
the component type. The new work:
- **Direction 1:** detect a closure-resource export, call `make`, call `call` with a
  test/driver argument, render the result. For the gate, a case supplies the arg(s) via
  the existing `(call …)` mechanism, driving the resource's `call` rather than a bare
  export.
- **Direction 2 / round-trip:** hold the `ResourceAny` from `make`, pass it as an argument
  into a second export call. This is pure wasmtime resource plumbing on the host side.

## Decisions (author, confirmed)

- **Direction 1 (Cadenza→host) first; round-trip (Direction 2) as a later increment;
  host-implemented functions out of scope entirely (a separate, larger design).**
- **own + no-drop first** (bounded leak, green gate — mirrors how the value-heap escape
  shipped), then the shared `borrow<t>` + dtor fix.
- **The handle always originates in Cadenza** — a closure crosses as an ordinary export's
  RESULT; no host-called `make`.
- **Interface:** a dedicated `cadenza:closure/*` (the host contract — a callable method —
  differs from the value-escape's `encode`, so a separate namespace keeps `cdz-run`'s two
  paths distinct).

## Increment plan (each = commit + triple gate, reject-don't-miscompile)

- **C-HOST-0 — codegen probe (byte-neutral). ✅ LANDED `@a7d7b903`.** `closure_call_functype`
  (the `call`-method component functype) + an oracle test pinning its byte shape.
  `#[allow(dead_code)]`, unreferenced.
- **C-HOST-1 — export a NO-CAPTURE closure, host calls it (own/no-drop).** An export whose
  RESULT is `(-> Int64 Int64)` → resource `closure-s64-s64` with a `call` method, published
  in `cadenza:closure/*`. The export body builds the cell and `resource.new`s it at the
  boundary-return; `cdz-run` calls the export, then `call(arg)` on the returned handle.
  First e2e: `(def (main) (fn (x) (+ x 1)))` exported; host calls the result with 5 → 6.
  Corpus: a new `17-host-closures.sexp` (or an extension) whose driver invokes the
  resource method.
  - **✅ ORACLE LANDED `@63d3d96b`** (`closure_host_resource` test module): a
    `ComponentBuilder` reference RUNS under wasmtime — `make()` → resource handle,
    `call(handle, 5)` = 6, fresh `make()` + `call(_, 41)` = 42. Standalone core (the
    no-capture cell IS the funcref-table slot; `make` = `resource.new(slot)`, `call` =
    `resource.rep(self)` → `call_indirect`); a `{make, call}` inner re-export component
    published as `cadenza:closure/exports`. **PROVES the whole mechanism** (wasmtime accepts
    a guest closure resource dispatching via the guest's own `call_indirect`). 🔑 CONFIRMED:
    `own<t>` CONSUMES the handle per `call` (a 2nd call on the same handle → "unknown handle
    index") — so a closure is single-use per handle until C-HOST-5's `borrow<t>`. This is
    the byte reference the compiler EMIT PATH (still TODO) hand-emits from a real export.
  - **✅ EMIT PATH + HOST + CORPUS LANDED `@20134a8f` — C-HOST-1 COMPLETE end-to-end.** The real
    compiler now emits it: `emit` dispatches a single `Ty::Fn`-result export to
    `emit_closure_resource`; `serialize::closure_resource_core_module` builds the core (make =
    `call export` + `resource.new`; call = `resource.rep` → `arr-get`/`get-int` →
    `call_indirect`); `envelope::assemble_closure_resource` + `resource_inner_component_closure`
    wrap it as `cadenza:closure/exports`. `cdz-run` recognizes the interface
    (`run_closure_resource`) and drives `make`→`call(args…)`, coercing the corpus's `(call …)`
    args to `call`'s declared types. VERIFIED e2e: `(def (main) (fn (x) (+ x 1)))` → host
    `call(5)`=6 / `call(41)`=42; `(* x 3)` on 4=12. +3 corpus cases
    (`spec/semantics/21-host-closures.sexp`, all pass the full gate) + 2 unit tests (compiler
    e2e via `ComposedRuntime::closure_make_call`; the core serializer). own<t> consumes per call.
- **✅ C-HOST-2 COMPLETE `@e234e618` — a PARAMETERIZED export returning a CAPTURING closure.**
  `(def (adder (: k Int64)) (fn (x) (+ x k)))` → `adder : (s64) -> own<closure-s64-s64>`; host
  calls `make(10)` then `call(5)` → 15. Two changes over C-HOST-1's nullary shape: (1) `make`
  FORWARDS the export's params (`closure_resource_core_module` gives `make` the export param
  valtypes + `local.get 0..N` before `call export`; `assemble_closure_resource` + inner component
  type `make` as `(export-params…)->own<t>` via a new `params_result_functype`); (2)
  `emit_closure_resource` collects the LIFTED bodies' used-ops (a capturing body reads its env via
  get-int etc. — ops only in the lifted body, which `resource_escape_build` doesn't walk) or the
  module was invalid. `cdz-run` SPLITS the flat `(call …)` arg list by `make`'s arity (first N →
  make's export params, rest → call's closure args). VERIFIED e2e: adder(10)+call(5)=15,
  adder(100)+call(7)=107. +2 corpus cases (→5 total) + 1 unit test. own<t> consumes per call.
- **✅ C-HOST-3 (multi-arg) COMPLETE `@0c45f75a` — corpus-only.** A closure of `(-> Int64 (->
  Int64 Int64))` → `call` with two args, 3-arg → three, a Bool-RESULT closure, and a
  parameterized+capturing+multi-arg combo — ALL already work (the C-HOST-1/2 flatten + arg-list
  plumbing was arity/type-agnostic; no compiler change). Witnessed as 4 corpus cases (→9 total),
  all through the full gate. ⏳ REMAINING part of "multiple signatures": a program exporting
  SEVERAL closures at once (`(export inc) (export add)`) still DECLINES — the escape dispatch
  fires only for a SINGLE export (`[e]`); two closure exports fall to the multi-export path where
  `Ty::Fn` has no boundary valtype. A MULTI-EXPORT closure envelope (N make/call pairs, or a
  resource type per signature published together) is a distinct later increment.
- **✅ C-HOST-3b (SCOPE FENCE: closures escaping effects are REJECTED) COMPLETE `@<pending>`.**
  Operator decision (2026-07-13): "one thing we should definitely reject for now is closures
  escaping effects. that's going to be super weird and I don't really want to support it. not sure
  it would even work correctly." A closure's effects are discharged by the `handle`/`(host …)`
  frame that is dynamically OPEN where the closure is BUILT; a host-held closure is invoked LATER,
  outside that frame, so the effect would have no home at the call. `emit_closure_resource` now
  walks the export body + every lifted closure body for a `Core::HostCall` (via
  `host::collect_host_imports`); if any is found it REJECTS with a new dedicated code **CDZ0406**
  (`Code::ClosureEscapesEffect`) naming the escaping op — instead of the incidental decline the
  program used to hit ("not in the host-import set", or CDZ0401 when the effect is not delegated at
  all). 🔑 A fully intra-program-HANDLED effect leaves NO `Core::HostCall` (the fold reduced it to
  plain code), so a self-contained effect inside the closure is NOT caught — only an effect that
  would genuinely escape the boundary is. +1 corpus case (21-host-closures →10, `(error CDZ0406)`
  code-matched) + 1 unit test (`a_closure_escaping_an_effect_declines_intentionally`). Distinct from
  CDZ0401 (EffectNoHome = no delegation anywhere): here the effect IS delegated, but the delegation
  cannot travel with the escaping closure.
- **✅ C-HOST-3c (RICHER CAPTURE coverage + clean multi-export decline) COMPLETE `@<pending>`.** Two
  parts, both green: (1) the C-HOST-2 make-forwarding + captured-cell machinery is capture-count- and
  body-shape-agnostic, so a closure capturing SEVERAL values, driving control flow off a captured
  Bool, binding a `let` in its body, or calling a top-level helper ALL cross the boundary + are
  invoked by the host with NO compiler change — witnessed as +5 corpus cases (21-host-closures →15).
  (2) A MULTI-EXPORT program with a closure result now DECLINES with a message NAMING the feature
  (`emit` gained a `layout.exports.len() > 1 && any Ty::Fn result` arm) instead of falling through to
  the scalar multi-export path's confusing generic "type `(-> A B)` has no component boundary
  representation". +1 unit test (`multiple_closure_exports_decline_naming_the_feature`). The multi-
  export closure ENVELOPE itself (N `make`, shared `call`/resource per signature) remains the next
  real structural increment + the round-trip prerequisite.
- **✅ MULTI-EXPORT ORACLE LANDED `@844030c5` (byte anchor, test-only).** A `ComponentBuilder` reference
  proves the multi-export shape RUNS under wasmtime before hand-emitting it — the same oracle-first
  rhythm C-HOST-1 used (`@63d3d96b`). TWO closure exports of the SAME signature `(-> Int64 Int64)`
  (`make-inc` slot 0 = `(+ x 1)`, `make-triple` slot 1 = `(* x 3)`) share ONE `call`. 🔑 THE LOAD-BEARING
  REALIZATION: the code slot is recovered from the resource rep at call time (`resource.rep` →
  `call_indirect`), so a single `call` dispatches WHICHEVER closure a handle names — N same-signature
  exports need N `make`s + 1 `call` + 1 resource type. Host drives each: `make-inc()`+`call(_,5)`=6,
  `make-triple()`+`call(_,5)`=15. Test fns: `multi_closure_core` (2 lifted bodies + 2 makes + shared
  call over a size-2 funcref table), `multi_inner_reexport_component` (2 make imports + shared call,
  re-exported vs the resource identity), `oracle_multi_closure_component`,
  `multi_export_closures_share_one_call_and_the_host_drives_each`.
- **✅ MULTI-EXPORT HAND-EMIT COMPLETE `@<pending>` (all 4 seams, end-to-end).** A program exporting
  SEVERAL closures of the SAME signature now compiles + runs: (a) `serialize` gained
  `multi_closure_resource_core_module(makes: &[ClosureMake])` — N `make-<name>` funcs + 1 shared `call`;
  the single-export `closure_resource_core_module` is now its N=1 wrapper (one code path). (b) `envelope`
  gained `assemble_multi_closure_resource` + `resource_inner_component_multi_closure` +
  `component_instantiate_multi_call_item` — N make imports/exports, 1 shared call, per-make own/functype
  types. (c) `emit` routes a same-signature multi-export set → `emit_multi_closure_resource`; a
  DIFFERENT-signature set and a closure-ALONGSIDE-non-closure export decline cleanly (later slices). (d)
  `cdz-run::run_closure_resource` picks `make-<opts.export>` (falls back to bare `make` for single-export).
  🔑 GOTCHA FIXED: the nested-component import names must be VALID KEBAB-CASE — `import-func-make-0`
  (numeric segment) fails wasmtime's extern-name check ("not in kebab case"); use `import-func-<make-name>`
  (`import-func-make-inc`, all-alpha). +3 corpus cases (21-host-closures →18p/1todo: inc, triple sharing
  the call, + a 3-export parameterized-capturing set) + a multi-export serializer unit test + an
  end-to-end `a_compiled_multi_closure_program_is_driven_by_the_host` (make-inc→6, make-triple→15 via the
  shared call). Baseline 1318, gate 1146p/0f.
- **✅ ROUND-TRIP ORACLE LANDED `@a3d80334` (byte anchor, test-only).** Proves C-HOST-4's shape RUNS
  under wasmtime before hand-emitting — same oracle-first rhythm as C-HOST-1 / multi-export. A
  `ComponentBuilder` component exports `make : () -> own<t>` AND a SEPARATE consumer `apply : (g:
  own<t>, x: s64) -> s64` whose PARAMETER is the closure resource. The test drives the round trip:
  host `make()` → a handle it holds → threaded BACK into `apply(handle, 5)` = 6 (dispatched via the
  guest's `call_indirect` from a handle that crossed OUT of one export and IN to another). 🔑 The
  dispatch (`resource.rep` → slot → `call_indirect`) is IDENTICAL to the `call` method; what is new is
  the handle originates in one export call and is consumed by another. 🔑 KEY REALIZATION for hand-emit:
  producer + consumer are in the SAME core module, so the closure's lifted lambda IS in-program —
  `closure_type_index` CAN match it by signature (given the producer created a lambda of that
  signature). Test fns: `roundtrip_core`, `roundtrip_inner_component`, `oracle_roundtrip_component`,
  `a_closure_handle_round_trips_through_a_consumer_export`.
- **✅ C-HOST-4 SERIALIZER SEAM LANDED `@53b5d747`.** `serialize::roundtrip_resource_core_module` emits N
  producer `make-<name>` funcs (as multi-export) PLUS M CONSUMER exports. A `ClosureConsume` names a
  consumer export + its selected body's core func idx + its params (`ConsumeParam::Closure|Scalar`) +
  result. 🔑 The consumer WRAPPER `resource.rep`s each closure param (boundary handle → guest cell) into a
  scratch local, then calls the consumer BODY — the exact mirror of `make` wrapping a producer body with
  `resource.new`. The consumer body is selected NORMALLY (its closure param is a plain CELL handle, applied
  via `Core::CallClosure`), so the wrapper is the ONLY boundary→cell bridge — NO change to `select.rs`'s
  `CallClosure` emit or `closure_type_index` (the closure was lifted in this module; the type index
  resolves by signature). +1 serializer unit test. Gate 1151p/0f.
- **✅✅ C-HOST-4 COMPLETE — the ROUND-TRIP END-TO-END `@a134b13a`.** The host produces a closure from one
  export and hands it BACK into another that applies it. All three remaining seams landed: (1)
  `emit_roundtrip_resource` (mod.rs) routes a producer(closure-RESULT) + consumer(closure-PARAM) export
  set — consumer bodies select NORMALLY (closure param = i32 cell), the serializer wrapper reps the
  handed-back handle; the consumer's `call_indirect` resolves against the producer's in-program lifted
  lambda by signature. Replaced the "passed AS A PARAMETER" decline; a consumer-ONLY program (no producer)
  still declines (host-fabricated closure = out of scope). (2) `envelope::assemble_roundtrip_resource` +
  `resource_inner_component_roundtrip` + `component_instantiate_roundtrip_item` — N producer `make-<name>`
  (exported under the source name in a round-trip, NOT `make-` prefixed) + M consumer funcs, each a plain
  component func whose first param is `own<t>`, sharing one resource type. (3) `cdz-run::run_roundtrip_closure`
  — a closure interface with NO `call` method is a round-trip; `(call <consumer> args…)` names the consumer,
  the driver calls the sole PRODUCER with leading args → a handle, then the consumer with handle + rest.
  +3 corpus cases (→21p/1todo: make-adder→apply-it, capture-tracking, a consumer applying the closure TWICE)
  + e2e compiler test (`a_produced_closure_round_trips_through_a_consumer_export`) + a consumer-only decline
  test. Gate 1167p/0f, baseline 1329. ⚠ scalar closure args/result still; ONE closure param per consumer.
- **✅ C-HOST-5 (the LEAK FIX) COMPLETE `@e10fef1b` — own-owns-and-drops.** A closure make+call and a
  produce→consume round trip now leave NO live heap cell. `make`/a producer allocates the closure cell;
  `call` and a round-trip consumer take the closure as `own<t>` (the canonical ABI transfers ownership
  INTO them), so each owns the cell's last reference — each RELEASES it (`heap.drop(rep)`) AFTER the
  dispatch returns (the lifted body finished BORROWING the env for its captures), balancing `make`'s
  alloc. The consumer wrapper drops each closure param ONCE, after the body — verified against a
  `twice-plus` consumer applying the closure twice (`(+ (g x) (g x))`). 🔑 This is the value-heap
  escape's own-owns-and-drops fix reused: `resource.rep` on a BORROWED self TRAPS in wasmtime 37, so
  `call`/consumers keep `own<t>` and drop the rep themselves rather than a `borrow<t>` + host-drop dtor
  (the dead end the escape root-caused, [[rcdzc-r1-resource-encode-linking-findings]]). +2 leak probes
  (`a_closure_call_leaves_no_live_objects`, `a_round_trip_leaves_no_live_objects`, #[ignore] — need the
  debug-counters runtime; live-objects==0). Gate 1174p/0f. ⚠ A genuine `borrow<t>` handle for
  REPEATED-call callbacks stays blocked upstream (wasmtime-37 borrow trap); own-per-call + self-drop is
  the sound leak-free posture today — a stored host callback is single-use per handle.
- **REMAINING (post-C-HOST-5, all optional widenings):** (1) genuine `borrow<t>` repeated-call handle
  (blocked on the wasmtime-37 borrow trap — an upstream fix or a workaround); (2) ✅ **DONE `@a7535e96` —
  WIDENED closure args/result to EVERY aliased-width scalar** (s8/u8/s16/u16/s32/u32/s64/u64, bool,
  f32/f64), via `closure_boundary_byte` = `comp_valtype_of` restricted to Int/Bool/Float (a `Tuple`'s u32
  threading handle is NOT a host boundary type → compound closure args still decline; +6 corpus, +1 decline
  test); (3) distinct-signature multi-export (N resource types); (4) ✅ **DONE `@c904362f` — a consumer with
  MULTIPLE closure params AND a closure param in ANY position** (`consumer_functype` walks a source-ordered
  `ConsumeParamAbi` list; the guard relaxed to "≥1 closure param, all same signature"; cdz-run threads a
  fresh handle per resource-typed param). ALSO FIXED a latent invalid-wasm miscompile — a scalar-THEN-closure
  consumer emitted a component whose lowered params didn't match the core body (the old functype hardcoded
  `own<t>` first); +3 corpus + a validity regression test. Consumer result byte is now the CONSUMER's own
  result (a consumer may return a different type than the closure). (5) a compound/closure-typed closure
  ARG. The core vertical + all scalar widths + multi/any-position closure params are COMPLETE.
- **✅ DISTINCT-SIGNATURE ORACLE LANDED `@355b7ee3` (byte anchor, test-only).** Proves the N-resource-type
  shape RUNS under wasmtime before hand-emitting — two closures of DIFFERENT signatures (`inc : (-> Int64
  Int64)` → resource `t0`, `isz : (-> Int64 Bool)` → resource `t1`) cross in ONE `cadenza:closure/exports`,
  each with its own `make`/`call` typed against its own resource. 🔑 The core imports resource-new/rep for
  BOTH resources (a core `resource.new` is typed to ONE resource, so `make-isz` news a `t1` through t1's
  intrinsic — the rep is a plain table slot, but the resource-TYPE distinction is real at the canon
  boundary); both lifteds still share the ONE guest funcref table. Host drives each: make-inc+call-inc(5)=6,
  make-isz+call-isz(0)=true. Test fns `distinct_sig_core`, `distinct_sig_inner_component` (2 imported +
  re-exported resources + 4 ascribed funcs), `oracle_distinct_sig_component`.
- **✅ DISTINCT-SIGNATURE SERIALIZER SEAM LANDED `@4154a745`.** `serialize::distinct_sig_resource_core_module`
  emits G signature GROUPS (`SigGroup { makes, arg_vts, ret_vt, lifted_slot }`), each its own resource type:
  per group its own `resource-new-<g>`/`resource-rep-<g>` imports, its `make-<name>`s, one shared `call-<g>`.
  🔑 the group's `call_indirect` functype index = `defined_type_base + order.len() + lifted_slot` (the
  distinct-sig core's TYPE layout differs from multi-export's — only ONE shared `(i32)->i32` rintr functype
  regardless of G — so `layout.lifted_type_index` is NOT reusable). +1 serializer unit test (2 groups, valid).
- **✅✅ DISTINCT-SIGNATURE MULTI-EXPORT COMPLETE — END-TO-END `@c9faa6e8`.** Closures of DIFFERENT
  signatures now compile + compose + the host drives each. (a) `envelope::assemble_distinct_sig_resource` +
  `resource_inner_component_distinct_sig` + `component_instantiate_distinct_sig_item` — G dtors, G resource
  types, G resource-new/rep pairs, an inner component importing/re-exporting all G resources with each fn
  ascribed to its own. (b) `emit_distinct_sig_resource` (mod.rs) groups exports by signature → `SigGroup`s +
  `SigGroupAbi`s (a group's `call_indirect` functype = the first lifted lambda matching by valtype shape);
  replaces the "DIFFERENT signatures" decline. (c) `cdz-run` distinct-sig branch: `(call <name>)` →
  `make-<name>` → the `call-g<n>` whose `self` resource type matches. 🔑🔑 TWO BUGS: (1) `import_base` off by
  2*(G-1) — added `resource_escape_build_n(intrinsics=2*G)`; (2) the KEBAB gotcha AGAIN — `call-0` numeric
  segment fails wasmtime's extern-name check → renamed `call-g<n>`. e2e: inc(5)=6/isz(0)=true, 3-export mix
  (inc+dbl share a resource, isz distinct) dbl(7)=14. +3 corpus (→33p). Gate 1202p/0f.
- **✅ ROBUSTNESS: adversarial probes + a clean transformer decline `@94b16219`.** Stress-probed the shipped
  multi-export/round-trip paths (higher fan-out, odd shapes) — found NO latent bugs; +6 corpus witnesses
  across two ticks (3 distinct sigs, 4 same-sig, consumer-applies-a-constant, multi-arg/capturing/widened
  round-trips). ONE clarity fix: a closure TRANSFORMER export (both a closure PARAM and a closure RESULT,
  e.g. `(def (twice (: g …)) (fn (x) (g (g x))))`) used to leak a confusing internal error; now declines
  cleanly naming the shape (`emit_roundtrip_resource` detects it up front). +1 decline test.
- **✅ DISTINCT-SIG ROUND-TRIP SERIALIZER SEAM LANDED `@204174bb`.** `serialize::distinct_sig_roundtrip_core_module`
  emits G signature groups, EACH a producer(s) + consumer(s): per group its own `resource-new-<g>`/
  `resource-rep-<g>`, its makes (new-<g>), its consumer wrappers (each closure param rep-<g>'d → cell, body
  called, cell dropped). `RtSigGroup { makes, consumers }`; no shared `call-<g>`. +1 serializer unit test
  (two groups, valid).
- **✅✅ DISTINCT-SIG ROUND-TRIP COMPLETE — END-TO-END `@d7e6de1b`.** Produce + consume closures of DIFFERENT
  signatures, each its own resource type. `envelope::assemble_distinct_sig_roundtrip_resource` (+ inner
  component + instantiate) publishes per group its producers + consumers; `emit_distinct_sig_roundtrip_resource`
  groups producers+consumers by signature → `RtSigGroup`/`RtSigGroupAbi` (dispatch routes here when the
  program's closure signatures aren't all the same); `cdz-run::run_roundtrip_closure` pairs each consumer
  closure param with the PRODUCER whose result resource type matches. 🔑 TWO BUGS FIXED: (1) all func
  sections must be uniformly PER-GROUP (makes then consumers) — a makes-flat-then-consumers-flat listing
  mismatched the envelope's per-group aliases; (2) the core emits 2*G rintr functypes so `defined_type_base
  = import_base` — else a SELECTED consumer body's `call_indirect(lifted_type_index)` was off by 2*G-1.
  e2e: appa(t0,I64->I64) adder(10)+appa(5)=15; appb(t1,I64->Bool) isz()+appb(0)=true/appb(5)=false. +3
  corpus (→54). ⚠ still declines: a closure TRANSFORMER, a >1-closure-param consumer.
- **✅ CAMELCASE / NON-KEBAB EXPORT NAMES COMPLETE `@f577fe72`.** A component-model extern name MUST be
  kebab-case, but a Cadenza source identifier may be camelCase or snake_case (`mkA`, `appA`, `makeAdder`).
  The single-closure escape already used FIXED boundary names (`make`/`call`), but the multi-export,
  distinct-sig, and round-trip shapes published the SOURCE name verbatim as the PUBLIC inner-component
  export name → a camelCase closure program emitted a component wasmtime rejects ("not in kebab case").
  Fix, two parts: (1) the PRIVATE per-func wiring names are now INDEX-derived (`import-func-f<n>`) instead
  of `import-func-<source-name>`, so a source name never reaches a wiring name — each inner-component
  import is paired with its instantiate arg by the same `f<n>` sequence, in the same order (a NEW helper
  `import_wire_name(f)` at every make/call/consumer wiring site across multi-export, distinct-sig,
  round-trip, and distinct-sig round-trip); (2) `export_func_ascribed_item` — the ONE path that emits a
  PUBLIC closure-interface export name — kebab-normalizes via `kebab_extern_name`, the SAME rule
  `comp_export_item` uses for a bare scalar export. Already-kebab names (`make`, `call`, `call-g0`,
  `make-adder`) are the IDENTITY, so every existing corpus case is byte-for-byte unchanged. `cdz-run`
  resolves the caller's SOURCE name through the SAME `kebab_extern_name` rule in the round-trip,
  distinct-sig, and multi-export lookups, so `(call appA …)` still finds the `app-a` export. +2 corpus
  (a camelCase round-trip `mkA`/`appA`→15; a camelCase same-sig multi-export `makeAdder`/`makeScaler`→12).
  Gate 1261p/0f. 🔑 This is the THIRD sighting of the kebab-extern-name gotcha (after `call-0`→`call-g<n>`
  and the effect-boundary names spec fixed at `@371a8d32`): ANY name minted at the component boundary must
  pass `kebab_extern_name`; PRIVATE wiring names should be index-derived so a source identifier can't leak.
- **✅ CLOSURE ALONGSIDE A NON-CLOSURE EXPORT COMPLETE `@293f175e` — the MIXED multi-export.** A program can
  now export a closure factory AND a plain (non-closure) function in ONE component (previously declined "not
  yet supported"). The closure(s) cross via the resource envelope (`make-<name>` + shared `call` under
  `cadenza:closure/exports`); each plain export is aliased off the SAME program instance and published as an
  ORDINARY top-level component func. Oracle-first: `oracle_mixed_component` +
  `a_closure_export_and_a_plain_export_coexist_and_the_host_drives_both` proved the resource-instance +
  top-level-func coexistence RUNS under wasmtime before hand-emit. Pieces: (a)
  `serialize::multi_closure_resource_core_module` gains a `plain: &[PlainExport]` param — the plain bodies
  are already defined funcs, so it just adds an EXPORT entry per plain export by its core-func index (no new
  functype/code); (b) `envelope::assemble_mixed_closure_resource` generalizes the multi-closure envelope
  (P=0 case) — each plain body is aliased AFTER `call`, lifted as a top-level comp func, exported directly
  under its kebab name (functypes/lifts laid after the call's); (c) `emit_mixed_closure_resource` (mod.rs)
  replaces the ALONGSIDE decline — partitions exports into closures (`Ty::Fn` result) + plain, requires the
  closures share ONE signature; (d) `cdz-run` routes `(call <plain>)` to the top-level bare func (via the
  kebab rule) and `(call <closure>)` to make/call (the guard is now "the named export is not a top-level
  func"). Scope: same-signature closures + aliased-scalar plain params/results. STILL DECLINES: DISTINCT
  closure signatures alongside a plain export (the distinct-sig envelope has no plain slot); a compound plain
  result (needs the memory/realloc lift shape). +6 corpus (closure+plain both driven; parameterized plain
  beside a capturing factory; two same-sig closures beside a plain export). Gate 1282p/0f.
- **✅ DISTINCT-SIGNATURE closures ALONGSIDE a non-closure export COMPLETE `@044ff65f`.** Extends the mixed
  shape to the distinct-sig case: closures of DIFFERENT signatures cross as N resource types (each its own
  `make-<name>`/`call-g<n>`) while plain exports ride alongside as top-level funcs. Pieces mirror the
  same-signature mixed envelope: (a) `serialize::distinct_sig_resource_core_module` gains `plain:
  &[PlainExport]` (adds an export entry per plain by its core-func index); (b)
  `envelope::assemble_distinct_sig_resource_mixed` (P=0 case delegated by `assemble_distinct_sig_resource`)
  — each plain body aliased AFTER the closure fns (core func `k+3g+total_fns+j`), functype at comp-type
  `1+g+2*total_fns+j`, lifted to comp func `k+total_fns+j`, exported top-level under its kebab name; (c)
  `emit_distinct_sig_resource` partitions exports into CLOSURE (grouped by signature) + PLAIN; (d)
  `emit_mixed_closure_resource`'s distinct-sig-plus-plain decline now routes to `emit_distinct_sig_resource`.
  `cdz-run` needs no change (the mixed dispatch already routes plain→bare-func, closure→make/call). +5
  corpus (distinct-sig Int64->Int64 + Int64->Bool beside a plain scalar, each closure + the plain driven;
  distinct-sig capturing closures beside a parameterized plain export). Gate 1310p/0f.
- **✅ PLAIN exports alongside a ROUND-TRIP closure program COMPLETE `@0af24de1` — a latent MISCOMPILE fix.**
  The round-trip path (producer + consumer exports) SILENTLY DROPPED a plain (non-closure) export — it
  emitted a valid component MISSING that export's name; the distinct-sig round-trip DECLINED such a program
  outright. Both now thread plain exports as ordinary top-level component funcs alongside the closure
  interface, the same plain-export composition the multi-export mixed envelopes use. Pieces: (a)
  `serialize::roundtrip_resource_core_module` + `distinct_sig_roundtrip_core_module` gain `plain:
  &[PlainExport]` (each exported by its core-func index); (b) `envelope::assemble_roundtrip_resource_mixed`
  + `assemble_distinct_sig_roundtrip_resource_mixed` (P=0 delegated by the existing entry points) alias each
  plain body off the same program instance after the closure funcs, lift it, export it top-level; (c)
  `emit_roundtrip_resource` collects plain exports (neither closure-result nor closure-param);
  `emit_distinct_sig_roundtrip_resource`'s "neither a producer nor a consumer" decline now collects the
  export as plain. cdz-run unchanged. +6 corpus (single-sig RT + plain, both driven + parameterized plain;
  distinct-sig RT + plain, both signature sides + the plain driven). Gate 1329p/0f.
- **✅ NOMINAL-over-scalar closure boundary PINNED `@80f50beb` (corpus-only).** A single-variant nominal
  (`(type UserId (Mk Int64))`) erases to its underlying scalar (type-system.md §156), so a closure whose
  arg/result is such a nominal ALREADY crosses as the scalar — `closure_boundary_byte` peels via
  `strip_nominal`, and the emitted `call` functype is `(own<t>) -> s64` (the nominal peeled, NO wrapper
  resource). Worked but was uncovered; +5 corpus (nominal result, nominal arg, capturing→nominal,
  round-trip through a nominal, nominal-over-Bool — the peel is kind-agnostic). Gate 1351p/0f.
- **✅ COMPOUND-RESULT CLOSURE ORACLE LANDED `@c7fc3f1d` (test-only byte anchor).** The first step of the
  compound-closure-boundary vertical: a closure whose `call` returns `list<u8>` (a compound rendered as the
  canonical value form) instead of a scalar. `closure_list_call_core` gives the closure core a MEMORY +
  `cabi_realloc` (a scalar `call` needs neither); `call(self, x)` recovers the code slot from the resource
  rep, dispatches the lifted closure via `call_indirect`, then writes the payload + canonical `(ptr, len)`
  return area and returns the retptr. `inner_reexport_component_list` types `call`'s result `list<u8>`;
  `oracle_closure_list_component` lifts `call` with Memory/Realloc canon options. `a_closure_returning_a_
  list_crosses_and_the_host_reads_the_bytes`: `make()` → handle, `call(handle,5)` → the host reads `[6,7]`
  (lifted `(x)->x+1`, n=6) through the canonical ABI. Validates + RUNS under wasmtime — licenses hand-emitting
  a compound-result `call`. ⚠ core section ORDER: table (sec 4) BEFORE memory (sec 5) — got "section out of
  order" until fixed. NEXT: hand-emit — serializer writes the value form into the return area (reuse the
  escape's `encode` walker for a tuple/String result; a `Bytes` result IS the payload, simplest first), the
  envelope lifts `call` with Memory/Realloc, `closure_boundary_byte`/`emit_closure_resource` route a compound
  result to the list shape instead of declining.
- **✅ BYTES-RESULT CLOSURE SERIALIZER SEAM LANDED `@68e568be`.** The production core-module serializer for a
  closure whose result is a runtime `Bytes` — `serialize::closure_bytes_resource_core_module`. Structurally
  the single-export scalar core, but `call` returns an i32 retptr (not a scalar) and the core carries a
  MEMORY + `cabi_realloc`. `call(self, args…)` recovers the cell rep, `call_indirect`s the lifted closure (it
  returns a runtime `Bytes` HANDLE), DROPs the cell (own<t> release), runs a `bytes-len`/`bytes-get` copy
  loop writing the payload to `OUT=8` + the canonical `(ptr=OUT, len=n)` return area, DROPs the Bytes handle,
  returns the retptr `0`. Reuses the value-escape's `to_bytes` copy-loop shape. Test
  `closure_bytes_resource_core_module_is_structurally_valid` (closure body builds `[x,x+1]` via
  `bytes-alloc`/`bytes-set`) — the shape `oracle_closure_list_component` proved runs. Gate 1352p/0f.
- **✅✅ BYTES-RESULT CLOSURE COMPLETE — END-TO-END `@08f5343f`.** A closure whose result is a runtime
  `Bytes` now crosses `call` as `list<u8>`, compiled by the real pipeline + run under the composed runtime.
  (a) `envelope::assemble_closure_bytes_resource` — a fork of `assemble_closure_resource` that ALSO aliases
  the program core's `memory` + `cabi_realloc` and lifts `call` with Memory/Realloc against `(self: own<t>,
  args…) -> list<u8>` (`resource_inner_component_closure_bytes` types the result; `closure_call_list_functype`
  is the functype). (b) `emit_closure_resource` — a `Bytes` result (peeling nominals, `ret_is_bytes`) routes
  to the bytes core + memory/realloc envelope instead of declining; the used-set gains `bytes-len`/
  `bytes-get`. The scalar path is unchanged; a compound ARG still declines (host→guest decode); other
  compound results (String/tuple/list) still decline (they need the escape's `encode` walker). The closure
  `call` returns a RAW `list<u8>` (the host reads the bytes directly), so the render is the byte sequence —
  `call(5)` on `(fn (n) (bin (u8 n) (u8 n+1)))` → `(5 6)`. +3 corpus (Bytes closure ×2 args + a capturing
  closure returning Bytes). Gate 1359p/0f.
- **✅ STRING-RESULT CLOSURE COMPLETE `@3934dc37`.** A `String` is a UTF-8 byte-rope handle
  representationally IDENTICAL to `Bytes` (same `bytes-*` store), so a `String` closure result crosses on
  the EXACT bytes-`call` path — one-line change: `emit_closure_resource`'s `ret_is_bytes` now accepts
  `Ty::String | Ty::Bytes` (both peeling nominals). The `call` copies the UTF-8 bytes out as `list<u8>`; the
  host gets the raw encoded bytes (not a decoded string), same render as `Bytes`. +3 corpus (constant
  `"hi"`→`(104 105)`, runtime `concat`→`(97 98 99)`, capturing). Gate 1362p/0f.
- **✅ MULTI-EXPORT byte-rope-result closures COMPLETE `@ba4c9864`.** N same-signature closures each
  returning a `Bytes`/`String` share ONE `call` that returns `list<u8>` — the multi-export shape extended to
  the compound-result `call`. `serialize::multi_closure_bytes_resource_core_module` (N makes + one shared
  bytes-`call` = memory + cabi_realloc + the copy loop) + `envelope::assemble_multi_closure_bytes_resource` +
  `resource_inner_component_multi_closure_bytes` (list-result `call`, running-type-counter layout);
  `emit_multi_closure_resource` routes a byte-rope shared result (`ret_is_bytes`) here. ⚠ found+fixed a
  `wasm_vec` miscount: the bytes alias section is `nmk+3` (N makes + call + memory + cabi_realloc), not
  `nmk+2` → "section size mismatch". +3 corpus (two same-sig Bytes closures both driven, two same-sig String
  closures). Gate 1364p/0f.
- **✅ MIXED-PATH byte-rope-result closure COMPLETE `@87a4554d`.** A `Bytes`/`String`-returning closure
  exported ALONGSIDE a plain non-closure export — the compound-`call` shape extended to the mixed multi-export.
  `emit_mixed_closure_resource` now detects `ret_is_bytes` (`Ty::Bytes | Ty::String`, peeling nominals) and
  routes to the compound serializer/envelope instead of the scalar `assemble_mixed_closure_resource`:
  `serialize::multi_closure_bytes_resource_core_module` and `envelope::assemble_multi_closure_bytes_resource`
  each gained a `plain: &[PlainExport]`/`&[PlainExportAbi]` param — the serializer exports each plain body in
  its export section; the envelope aliases (plain core func after cabi_realloc), lifts (plain lift comp func
  `k+nmk+1+j` against functype `5+2*nmk+j`) and top-level-exports each plain func alongside the closure make/
  call. The pure multi-export bytes path passes `&[]`. `cdz-run` unchanged (mixed dispatch already routes
  plain→bare-func, closure→make/call). +4 corpus (Bytes closure + plain `two` both driven; String closure +
  parameterized `dbl` both driven).
- **✅ DISTINCT-SIG byte-rope-result closure COMPLETE `@3d628337`.** Closures of DIFFERENT signatures each
  returning a `Bytes`/`String` now cross as G distinct resource types, each with its OWN `list<u8>`-returning
  `call-<g>` (memory + `cabi_realloc` shared across groups). Extends the byte-rope compound `call` from the
  single/multi/mixed shapes to the N-resource-type (distinct-sig) shape; a byte-rope group can coexist with a
  SCALAR group in one component (the scalar `call-<g>` returns by value; the byte-rope one via the copy loop).
  Pieces: (1) `serialize::SigGroup.ret_is_bytes` + `distinct_sig_resource_core_module` — when any group is
  byte-rope, add a shared memory + `cabi_realloc` functype/func/export; a byte-rope group's `call-<g>` emits
  the `bytes-len`/`bytes-get` copy-loop body writing a `(ptr,len)` return area (the group's core `call`
  functype `(i32 self, args…) -> i32` is UNCHANGED — a bytes handle IS an i32 — so only the BODY + the shared
  memory/realloc differ). (2) `envelope::SigGroupAbi.ret_is_bytes` + `assemble_distinct_sig_resource_mixed` —
  alias the shared memory/realloc (after the closure fns, before plain), lift each byte-rope call with
  Memory/Realloc against `(…) -> list<u8>` (own<t> + list<u8> + functype = 3 comp types vs a scalar call's 2).
  (3) `resource_inner_component_distinct_sig` converted to a RUNNING type counter (a byte-rope call consumes 3
  types, not 2, breaking the fixed `g + 2f` formula — in both import and export phases). (4)
  `emit_distinct_sig_resource` computes per-group `ret_is_bytes` (byte-rope skips the scalar-boundary-byte
  check; `bytes-len`/`bytes-get` added to the used-op set). The instantiate item is index-by-name so it was
  unchanged. +7 corpus (Int64/Bool/String distinct sigs; a byte-rope group coexisting with a scalar group both
  driven; byte-rope distinct-sig + a plain export). Gate 102p/1todo on 21-host-closures.
- **✅ ROUND-TRIP byte-rope-result consumer COMPLETE `@48c814d4`.** A round-trip CONSUMER (an export that
  takes a produced closure resource back and applies it) can now RETURN a `Bytes`/`String` — it crosses as
  `(own<t>, args…) -> list<u8>` (shared memory + `cabi_realloc`, lifted with Memory/Realloc), completing the
  byte-rope compound `call` across ALL closure shapes (single/multi/mixed/distinct-sig/round-trip). A byte-rope
  consumer coexists with a SCALAR consumer of the same closure and with a plain export. Pieces: (1)
  `serialize::ClosureConsume.ret_is_bytes` + `roundtrip_resource_core_module` — any byte-rope consumer adds a
  shared memory + `cabi_realloc`; the consumer wrapper copies the body's returned Bytes/String handle out as a
  `list<u8>` `(ptr,len)` return area (the `bytes-len`/`bytes-get` loop) AFTER dropping the closure cells. (2)
  `envelope::ClosureConsumeAbi.ret_is_bytes` + `assemble_roundtrip_resource_mixed` — alias the shared
  memory/realloc, lift each byte-rope consumer with Memory/Realloc against `(…) -> list<u8>` (own<t> +
  list<u8> + functype = 3 comp types); `consumer_list_functype` helper. (3)
  `resource_inner_component_roundtrip` converted to a RUNNING type counter (a byte-rope consumer consumes 3
  types, not 2, in both import + export phases). (4) `emit_roundtrip_resource` per-consumer `ret_is_bytes`
  (byte-rope skips the scalar-boundary-byte check; `bytes-len`/`bytes-get` added to the used-op set). `cdz-run`
  unchanged (a `list<u8>` result is a `Val::List` rendered as `(5 6)`). 🔑 ALSO fixed a LATENT `BinBuild`
  slot-typing MISCOMPILE: two `(g x)` closure applications across two `bin` segments aliased one wasm local at
  two widths (i32 cell vs i64 arith stash) → invalid module; each segment's value emit now floats above the
  high-water mark (the disjoint-slot discipline `emit_checked_arith` already uses). +7 corpus.
- **✅ DISTINCT-SIG ROUND-TRIP byte-rope-result consumer COMPLETE `@60e14737` — the byte-rope story is now
  CLOSED.** Closures of DIFFERENT signatures each cross as their own resource type, and a round-trip CONSUMER
  of one signature can RETURN a `Bytes`/`String` — crossing as `(own<t_g>, args…) -> list<u8>` (shared memory +
  `cabi_realloc`, lifted with Memory/Realloc). This was the LAST byte-rope gap: the compound `call`/consumer
  now works across EVERY closure shape (single/multi/mixed/distinct-sig/round-trip/distinct-sig-round-trip). A
  byte-rope consumer coexists with a scalar consumer of ANOTHER signature, and two byte-rope consumers of
  different signatures coexist. Pieces mirror the single-sig round-trip byte-rope work, per-group: (1)
  `serialize::distinct_sig_roundtrip_core_module` — any byte-rope consumer adds a shared memory +
  `cabi_realloc`; the byte-rope consumer wrapper copies the body's returned handle out as `list<u8>`. (2)
  `envelope::assemble_distinct_sig_roundtrip_resource_mixed` — alias the shared memory/realloc, lift each
  byte-rope consumer with Memory/Realloc against `(…) -> list<u8>` (own<t> + list<u8> + functype = 3 comp
  types). (3) `resource_inner_component_distinct_sig_rt` converted to a RUNNING type counter (byte-rope
  consumer = 3 types). (4) `emit_distinct_sig_roundtrip_resource` per-consumer `ret_is_bytes`. `cdz-run`
  unchanged. +4 corpus (byte-rope + scalar consumer of another sig; two byte-rope consumers of different sigs).
- **✅ COMPOUND (tuple/record) closure RESULT COMPLETE `@48adec10`.** A closure whose result is a fixed-shape
  compound (tuple/record/sum) now crosses: the `call` returns `list<u8>` carrying the canonical VALUE FORM,
  and the host decodes + pretty-prints the typed `(: value T)` document (the full structure + type, not the
  bare byte sequence a byte-rope renders). Pieces: (1) `serialize::closure_value_resource_core_module` —
  structurally the bytes core (memory + `cabi_realloc`, `list<u8>` `call`), but the `call` body WALKS the
  closure's returned compound handle to fill the value-form TEMPLATE (`lower::runtime_value_form_template` +
  `emit_hole_fill`, the value-heap escape's machinery keyed on the dispatch-result handle instead of a
  resource rep), then returns the `(ptr,len)` retarea. Reuses `assemble_closure_bytes_resource` (same
  `list<u8>` boundary); the data section (11) lays the template + comes AFTER code (10). (2)
  `emit_closure_resource` — a non-byte-rope, non-scalar result consults `runtime_value_form_template`:
  `Some(t)` → the value core, `None` → the scalar decline (a variable-length LIST/MAP/SET has no fixed
  template, still declines cleanly); imports `get-bool` for a Bool leaf's hole fill. (3)
  `cdz-run::render_closure_call_result` — a `list<u8>` `call` result is TRY-DECODED as a value form
  (`codec::decode` is total + 8-byte-schema-header-guarded), else rendered as a raw byte-rope; the header
  disambiguates the two unambiguously (no flag), applied at ALL closure-call result sites. +6 corpus (tuple,
  record, Bool leaf, nested tuple, capturing→tuple, negative int leaf), each rendering the full typed form.
  🔑 SCOPE: single-export, fixed-shape compound. Multi/mixed/distinct-sig/round-trip compound results + a
  variable-length list/map/set result are later widenings.
- **✅ COMPOUND closure RESULT on the MULTI-EXPORT path COMPLETE `@2f8ec34e`.** N same-signature closures
  each returning a fixed-shape compound (tuple/record/sum) now share ONE `call` that returns the value form
  as `list<u8>`. The shared `call` recovers each closure's code slot from the resource rep, dispatches it,
  and walks the returned compound handle into the ONE value-form template (all exports share the result type
  → one template). Pieces: (1) `serialize::multi_closure_value_resource_core_module` — combines the
  multi-bytes core (N makes + shared `list<u8>` `call` + memory/`cabi_realloc` + plain-export slots) with the
  single-export value core's value-form body (a data-section template walked from the dispatched compound
  handle via `emit_hole_fill`). (2) `emit_multi_closure_resource` — a non-byte-rope, non-scalar shared result
  consults `runtime_value_form_template`; `Some(t)` → the value core (reusing
  `assemble_multi_closure_bytes_resource`, same `list<u8>` envelope), `None` → the scalar decline; imports
  `get-bool` for a Bool leaf. `cdz-run` already try-decodes. +5 corpus (two tuple closures both driven, two
  record closures in canonical field order, three capturing closures sharing one call). Record fields render
  in CANONICAL sorted-name order (same as single-export + the escape).
- **✅ COMPOUND closure RESULT on the MIXED path COMPLETE `@dae1e7e0`.** A compound-returning closure
  exported ALONGSIDE a plain non-closure export now crosses: the closure's shared `call` returns the value
  form as `list<u8>` (walking the returned handle into the value-form template), and each plain export rides
  as an ordinary top-level component func. NO new serializer/envelope — `emit_mixed_closure_resource`
  consults `runtime_value_form_template` for the shared result: `Some(t)` → `multi_closure_value_resource_
  core_module` (which already threads plain exports) reusing `assemble_multi_closure_bytes_resource` (the same
  `list<u8>` envelope with plain slots), `None` → the byte-rope/scalar paths; imports `get-bool`. `cdz-run`
  already try-decodes. +4 corpus (tuple closure + plain `two`; record closure + parameterized plain `inc`).
- **✅ COMPOUND closure RESULT on the DISTINCT-SIG path COMPLETE `@e2c40e8b`.** Closures of DIFFERENT
  signatures each returning a fixed-shape compound cross as G distinct resource types, each with its own
  `call-g<n>` returning THAT group's canonical value form as `list<u8>` (a PER-GROUP template, since result
  types differ — e.g. `(Tuple Int64 Int64)` vs `(Tuple Bool Int64)`). A compound group, a byte-rope group,
  and a scalar group all coexist in one component. Pieces: (1) `serialize::SigGroup.ret_template` +
  `distinct_sig_resource_core_module` — each compound group's template gets its own 4-aligned data-section
  region (`byte_off`/`ret_off`); byte-rope groups write their dynamic payload PAST all compound data
  (`bytes_out_off`) so the two never collide; the shared memory + `cabi_realloc` fires whenever ANY group
  crosses as `list<u8>` (byte-rope OR compound); a compound `call-<g>` walks the returned handle into its
  template region via `emit_hole_fill`. (2) `emit_distinct_sig_resource` — per-group `ret_template`;
  `SigGroupAbi.ret_is_bytes` now means "crosses as `list<u8>`" (byte-rope OR compound), so the envelope lifts
  it with Memory/Realloc unchanged; imports `get-bool`. `cdz-run` already try-decodes. +5 corpus (two
  distinct-sig tuple closures with DIFFERENT result types; a compound + byte-rope + scalar group all driven).
- **✅ COMPOUND result on the ROUND-TRIP path COMPLETE `@dd7ebd9a` — the compound-result story is CLOSED.** A
  round-trip CONSUMER that takes a produced closure back, applies it, and RETURNS a fixed-shape compound now
  crosses: it returns `list<u8>` carrying the value form (its own template, walked from the body's returned
  handle). This completes the fixed-shape compound result across EVERY closure shape (single/multi/mixed/
  distinct-sig/round-trip). A compound consumer coexists with a scalar consumer, a byte-rope consumer of the
  same closure, and a plain export. Pieces mirror the distinct-sig work: (1) `serialize::ClosureConsume.
  ret_template` + `roundtrip_resource_core_module` — each compound consumer's template gets its own 4-aligned
  data-section region; byte-rope consumers write PAST all compound data (`bytes_out_off`); shared memory +
  `cabi_realloc` whenever any consumer crosses as `list<u8>`; a compound consumer wrapper walks the body's
  returned handle via `emit_hole_fill`. (2) `envelope::ClosureConsumeAbi.ret_is_bytes` now means "crosses as
  `list<u8>`" (byte-rope OR compound) → the round-trip envelope's Memory/Realloc lift is unchanged. (3)
  `emit_roundtrip_resource` per-consumer `ret_template`; imports `get-bool`. `cdz-run` already try-decodes.
  +7 corpus.
- **✅ COMPOUND result on the DISTINCT-SIG ROUND-TRIP path COMPLETE `@679a1f4d` — the FIXED-SHAPE
  compound-result surface is CLOSED.** Producers/consumers of DIFFERENT signatures where a consumer RETURNS a
  fixed-shape compound now cross: each such consumer returns `list<u8>` carrying the value form (its own
  per-consumer template). This was the last fixed-shape compound-result gap — the compound result now works
  across EVERY closure shape (single/multi/mixed/distinct-sig/round-trip/distinct-sig-round-trip). A compound
  consumer coexists with a scalar consumer, another compound consumer of a different sig, and a byte-rope
  consumer. Pieces: (1) `serialize::distinct_sig_roundtrip_core_module` — each compound consumer's template
  gets its own 4-aligned data-section region; byte-rope consumers write PAST all compound data
  (`bytes_out_off`); shared memory + `cabi_realloc` whenever any consumer crosses as `list<u8>`; a `flat_cons`
  counter indexes the per-consumer data placement in group order; a compound consumer wrapper walks the body's
  returned handle via `emit_hole_fill`. (2) `emit_distinct_sig_roundtrip_resource` — per-consumer
  `ret_template`; `ClosureConsumeAbi.ret_is_bytes` now = "crosses as `list<u8>`" (byte-rope OR compound). (3)
  `cdz-run` already try-decodes. +5 corpus (compound + scalar consumer of another sig; two compound consumers
  of different sigs — tuple + record; compound + byte-rope consumer of different sigs).
- **✅ VARIABLE-LENGTH collection (List/Map/Set) closure RESULT COMPLETE `@0beb35d6` (single-export).** A
  closure returning a `List`/`Map`/`Set` now crosses: the `call` returns `list<u8>` carrying the canonical
  value form, rendered at RUN TIME by the runtime `value-encode(rep, desc)` op (the recursive-sum escape's
  approach C) walking the returned collection handle against a compiler-baked shape DESCRIPTOR. Unlike a
  fixed-shape tuple/record (a static template), a collection is variable-length, so the runtime assembles the
  document. Pieces: (1) `serialize::closure_value_encode_resource_core_module` — the `call` body dispatches →
  the collection handle, drops the cell, builds the descriptor Bytes (`bytes-alloc` + literal `bytes-set`),
  calls `value-encode(rep, desc)` → the document, copies it out, releases rep/desc/doc; NO data section (the
  descriptor bytes are baked into the code). (2) `emit_closure_resource` — a non-bytes, non-scalar,
  non-fixed-template List/Map/Set result consults `lower::sum_shape_descriptor` (its List/Map/Set arm builds a
  parametric `Framed` descriptor so element/key/value types are observable — a NESTED collection crosses too);
  `Some(desc)` routes to the value-encode core reusing `assemble_closure_bytes_resource`. `cdz-run` already
  try-decodes. +6 corpus (List, Set canonical order, Map canonical key order, nested List, capturing→List,
  empty List). 🔑 SCOPE: single-export.
- **✅ VARIABLE-LENGTH collection closure RESULT on the MULTI-EXPORT path COMPLETE `@1258fdfc`.** N
  same-signature closures each returning a List/Map/Set now share ONE `call` that value-encodes the returned
  handle against the ONE shared shape descriptor (all exports share the result type). Pieces: (1)
  `serialize::multi_closure_value_encode_resource_core_module` — combines the multi-bytes core (N makes +
  shared `list<u8>` `call` + memory/`cabi_realloc` + plain slots) with the single-export value-encode body
  (build the descriptor Bytes, value-encode the dispatched collection handle, copy the doc out); no data
  section (the descriptor is baked into the shared `call`). (2) `emit_multi_closure_resource` — a
  non-bytes/scalar/fixed-template List/Map/Set shared result routes to the value-encode core reusing
  `assemble_multi_closure_bytes_resource`. `cdz-run` already try-decodes. +4 corpus (two list closures both
  driven; three Set-returning closures sharing one `call`, two driven).
- **✅ VARIABLE-LENGTH collection closure RESULT on the MIXED path COMPLETE `@c0e96474`.** A
  collection-returning closure exported ALONGSIDE a plain non-closure export now crosses: the closure's shared
  `call` value-encodes the returned collection handle (against the ONE shared descriptor), and each plain
  export rides as a top-level func. NO new serializer/envelope — `emit_mixed_closure_resource` consults
  `lower::sum_shape_descriptor` for a List/Map/Set shared result; `Some(desc)` routes to
  `multi_closure_value_encode_resource_core_module` (which already threads plain exports) reusing
  `assemble_multi_closure_bytes_resource` (the same `list<u8>` envelope with plain slots). `cdz-run` already
  try-decodes. +4 corpus (List closure + plain `two`; Map closure + parameterized plain `inc`).
- **✅ VARIABLE-LENGTH collection RESULT on the DISTINCT-SIG path COMPLETE `@27a2e90e`.** Closures of
  DIFFERENT signatures each returning a List/Map/Set now cross as G distinct resource types, each `call-g<n>`
  value-encoding the returned handle against THAT group's shape descriptor. A collection group, a compound
  group, a byte-rope group, and a scalar group all coexist with disjoint memory. Pieces: (1)
  `serialize::SigGroup.ret_descriptor` + `distinct_sig_resource_core_module` — a collection group's `call-<g>`
  builds the descriptor Bytes, value-encodes the dispatched handle, and copies the doc out PAST all
  compound-template data (`bytes_out_off`); the shared memory + `cabi_realloc` fires whenever any group
  crosses as `list<u8>`. (2) `emit_distinct_sig_resource` — per-group `ret_descriptor`; `SigGroupAbi.
  ret_is_bytes` now = "crosses as `list<u8>`" (byte-rope OR compound OR collection). `cdz-run` already
  try-decodes. +6 corpus (two distinct-sig list closures; a collection + a compound + a byte-rope + a scalar
  group all in one component, each driven).
- **✅ VARIABLE-LENGTH collection RESULT on the ROUND-TRIP path COMPLETE `@6cbe22e0` — the collection-result
  surface is CLOSED across ALL shapes.** A round-trip CONSUMER that applies a handed-back closure and RETURNS
  a List/Map/Set now crosses: it returns `list<u8>` carrying the value form, rendered by `value-encode(rep,
  desc)` against its shape descriptor. A collection consumer coexists with a scalar consumer of the same
  closure. Pieces: (1) `serialize::ClosureConsume.ret_descriptor` + `roundtrip_resource_core_module` — a
  collection consumer's wrapper builds the descriptor Bytes, value-encodes the body's returned handle, and
  copies the doc out PAST all compound-template data (`bytes_out_off`); shared memory + `cabi_realloc` fires
  whenever any consumer crosses as `list<u8>`. (2) `envelope::ClosureConsumeAbi.ret_is_bytes` now = "crosses
  as `list<u8>`" (byte-rope OR compound OR collection). (3) `emit_roundtrip_resource` per-consumer
  `ret_descriptor`. `cdz-run` already try-decodes. +5 corpus (List/Set/Map consumer results; a List consumer +
  a scalar consumer of the same closure).
- **✅ VARIABLE-LENGTH collection RESULT on the DISTINCT-SIG ROUND-TRIP path COMPLETE `@eec9d552` — the LAST
  collection sub-shape; the collection-result surface is now closed across EVERY closure shape.** A
  distinct-signature round-trip CONSUMER that applies its handed-back closure and RETURNS a List/Map/Set now
  crosses as `list<u8>` value form via `value-encode(rep, desc)` against the consumer's own shape descriptor —
  the single-sig round-trip mechanism generalized to the per-group distinct-signature core. A collection
  consumer coexists with a scalar / compound / byte-rope consumer of another signature (three result-assembly
  mechanisms across two resource types). Pieces: (1) `emit_distinct_sig_roundtrip_resource` — per-consumer
  `ret_descriptor` via `sum_shape_descriptor`; `any_collection` adds the value-encode used-ops;
  `ConsS.ret_descriptor` threaded into `serialize::ClosureConsume`; `ClosureConsumeAbi.ret_is_bytes` widened.
  (2) `distinct_sig_roundtrip_core_module` — `consumer_is_list` + the consumer wrapper's `extra_i32` (5: rep,
  desc, doc, n, i) widened; a new collection branch builds the descriptor Bytes, `value-encode`s the body's
  returned handle, copies the doc out PAST all compound-template data (`bytes_out_off`, disjoint memory). (3)
  The envelope re-export/instantiate path is UNCHANGED (it already lifts a `ret_is_bytes` consumer as
  `(…)->list<u8>`). `cdz-run` already try-decodes. +6 corpus (a List + scalar consumer of distinct sigs; two
  collection consumers — List + Map; a List + a compound consumer).
- **✅ COMPOUND closure ARGUMENT on the ROUND-TRIP path COMPLETE `@3f9ff427`.** On the round-trip path the
  consumer APPLIES its handed-back closure ITSELF, in-guest (`(g <compound>)` inside the consumer body), so
  the closure's ARGUMENT is built in the guest heap and NEVER crosses the host boundary — only the closure
  HANDLE (an `own<t>` resource, i32) and the consumer's own scalar params cross. So a closure argument (and
  result) need only be MACHINE-representable (a value-heap compound is an i32 handle in-guest), NOT
  scalar-host-boundary-representable. `emit_roundtrip_resource` widens the closure-signature arg + result
  checks from `closure_boundary_byte` (aliased scalar) to `valtype_of` (any machine value) — a pure
  fence-relaxation: the closure signature's ABI bytes were never consumed downstream (a `make` functype takes
  the export's own params, a consumer's its own; the signature only shapes the in-guest `call_indirect`). A
  `(-> (Tuple …) R)`/`(-> (Record …) R)`/`(-> (List …) R)` closure handed back and applied to a guest-built
  compound now compiles + runs. The DIRECT-CALL path (`emit_closure_resource`) is UNCHANGED — it still
  declines a compound closure arg (the HOST supplies it over the boundary → needs host→guest decode). +5
  corpus (Tuple/Record/List arg applied round-trip; compound arg + compound consumer result; the direct-call
  compound-arg decline).
- **✅ COMPOUND closure ARGUMENT on the DISTINCT-SIG ROUND-TRIP path COMPLETE `@4e8df79f`.** The same
  in-guest reasoning + the same pure fence-relaxation, applied to the per-group distinct-signature core:
  `emit_distinct_sig_roundtrip_resource` widens its per-group closure-signature arg + result checks from
  `closure_boundary_byte` to `valtype_of`. A distinct-sig program mixing `(-> (Tuple …) Int64)` and
  `(-> (Record …) Int64)` closures — each applied to a guest-built compound through its own resource type —
  now compiles + runs. +4 corpus (a compound-arg + a scalar-arg closure of distinct sigs; two compound-arg
  closures of distinct sigs — a Tuple-arg + a Record-arg). **A compound closure ARGUMENT is now supported on
  BOTH round-trip paths (single-sig + distinct-sig).**
- **✅ ARGUMENT surface widened to EVERY machine-representable type on the round trip `@79ada1f3` (corpus +
  doc; no compiler change).** The `valtype_of` relaxation reaches not just fixed-shape compounds but a SUM
  (Option/Result), a NESTED compound, a String/Bytes, AND — most notably — a CLOSURE-TYPED argument. A
  HIGHER-ORDER closure `(-> (-> A B) R)` handed back and applied to a guest-built inner closure needs NO extra
  resource machinery: the inner closure is an ordinary in-guest funcref-table value (an i32 slot,
  `valtype_of(Ty::Fn)`), applied by the outer via the usual `call_indirect`; only the OUTER handle crosses the
  host boundary. Verified sound (a captured + twice-applied inner closure → 90; two distinct inner closures
  kept distinct → 504) on BOTH round-trip paths. +7 corpus (sum/nested/String arg; higher-order single-sig +
  distinct-sig; captured-twice; two-distinct-inner) + a closure-typed-arg DIRECT-CALL decline.
- **✅ UNANNOTATED inner-closure compound param via the context arrow COMPLETE `@e44f25d1`.** An inner closure
  `(fn (p) …)` whose own param `p` is a COMPOUND used to decline "a closure's parameter type has no machine
  representation" when UNANNOTATED — `p` solved `Any` bottom-up. Root cause: `infer::expected_arrow_for_lambda`
  recovered a lambda's expected arrow only when the application head was a lambda/def (`lambda_params_of`); a
  higher-order closure PARAMETER `g : (-> (-> A B) R)` applied `(g (fn (p) …))` has a function-VALUED head that
  is a VARIABLE. New path (3b): peel `type_of(head)` by the argument index for any function-valued head and
  take that domain as the lambda's expected arrow. So an unannotated inner compound/collection param recovers
  its type from the higher-order parameter's declared arrow — matching the explicit `(: p (Tuple …))` form
  (which already worked). A pure inference widening (an ordinary scalar-arg HOF is unaffected; a
  HOF-passed-as-value CDZ0203 decline is unchanged). +2 corpus (an unannotated Tuple inner param; an
  unannotated List inner param). **Every closure ARGUMENT — annotated OR not, scalar/compound/sum/nested/
  String/collection/closure-typed — is now supported on both round-trip paths.**
- **✅ FLAT multi-argument arrow `(-> A B … R)` curries COMPLETE `@fd232a9a`.** The arrow type constructor
  handled only arity 1 (`(-> R)` = nullary `Unit -> R`) + arity 2 (`(-> P R)`); a FLAT multi-arg arrow
  `(-> A B … R)` errored "-> takes one or two type arguments" (`eval::reduce_ctor` + `type_in_env`). So a
  round-trip consumer whose closure parameter was written flat — `(: g (-> Int64 Int64 Int64))` — solved
  `Any` and declined "parameter type is ambiguous", though it WAS annotated; only the explicitly-curried
  `(-> Int64 (-> Int64 Int64))` spelling worked. Fix: both arrow arms now CURRY any arity ≥1
  right-associatively into `A -> (B -> (… -> R))`. A foundational inference win (not closure-specific — any
  flat n-ary arrow annotation) that unblocks the idiomatic multi-arg closure signature. +4 corpus (flat 2-arg
  beside the curried-spelling case; flat 3-arg; flat multi-arg with compound args; flat multi-arg with a
  compound result).
- **✅ SUM + nested-collection COMPOUND results cross via `value-encode` COMPLETE `@a3c1485b`.** A closure
  result crossed as `list<u8>` only for a byte-rope, a FIXED-shape compound (static template), or a bare
  `List`/`Map`/`Set`; a SUM (`Option`/`Result`/a user sum) or a compound CONTAINING a variable-length element
  (a tuple/record with a list/map/set inside) declined "no scalar host-boundary representation" — even though
  the single-export value escape already renders these via the runtime `value-encode` op. Two changes: (1)
  `lower::sum_shape_descriptor` gains a Tuple/Record arm (it already handled Sum/Nominal/List/Map/Set) —
  `shape_of` recurses over the elements. (2) The SIX `ret_descriptor` classifications (the `call` result for
  single/multi/mixed/distinct-sig producers + the round-trip consumer result for single + distinct-sig) now
  fall back to `sum_shape_descriptor` for ANY result once it is not a scalar / byte-rope / fixed-template,
  instead of only matching `List`/`Map`/`Set`. Safe general widening (`sum_shape_descriptor` → `None` for a
  scalar/unrenderable shape; a fixed-shape compound still takes the cheaper static-template path first). An
  unconstrained sum type arg (an `(Ok …)` whose `Err` type is never pinned) still correctly declines. +7
  corpus (a `call` returning Option / a user sum / a tuple-of-list; a round-trip consumer returning Option / a
  Result Err-pinned / a Result reaching both variants / a tuple-of-list). **The closure-RESULT matrix now
  reaches EVERY value-encodable type — scalar, byte-rope, fixed compound, collection, SUM, and
  compound-containing-collection — across every closure shape.**
- **✅ COMPOSED round-trip surface LOCKED IN `@86c9fa0c` (corpus-only).** The argument surface (every
  machine-representable type, incl. higher-order closure-typed args) and the result surface (every
  value-encodable type) COMPOSE freely, across single-sig and distinct-sig grouping — verified end-to-end with
  adversarial cases: a Map whose value is a list; an Option of a tuple; a list of tuples; a higher-order arg
  composed with a Sum result; a distinct-sig round-trip pairing a Sum-result consumer with a collection-result
  consumer. +6 corpus. **The ROUND-TRIP closure surface is SATURATED — every remaining gap is DIRECT-CALL
  host→guest transfer (a closure/compound the HOST supplies over the boundary, blocked on a nonexistent
  `value-decode` runtime op + a closure-resource-into-a-call ABI) or the `borrow<t>`/transformer frontier.**
- **✅ C-HOST-6 — a scalar closure crosses as a REPEATABLE `borrow<t>` callback handle `@28b678bd`. The
  wasmtime-37 borrow trap is DODGED.** A single-export scalar closure's `call` now takes `borrow<t>` instead
  of `own<t>`, so the host KEEPS the handle across calls (one `make`, MANY `call`s — the natural callback
  shape), versus `own<t>`'s consume-per-call ("unknown handle index" on a 2nd call). The wasmtime-37 borrow
  trap (`resource.rep` on a borrowed self traps) is dodged EXACTLY as the value-heap `encode` borrow method
  does: `lift_borrow` hands the guest the REP DIRECTLY as `call`'s `self`, so the body uses param 0 as the
  cell rep with NO `resource.rep` and does NOT self-drop — the `t-dtor` reclaims when the host drops the
  handle. Still leak-free (make allocs, dtor drops). Pieces: `serialize::
  {multi_closure_resource_core_module_with_host_borrow, closure_resource_core_module_borrow}` (a `call_borrow`
  flag branches the scalar `call` body); `envelope::{assemble_closure_resource_borrow,
  resource_inner_component_closure_borrow}` (`call` self = `borrow<t>`, `make` stays `own<t>`);
  `emit_closure_resource`'s scalar tail routes to borrow. PROVEN e2e under wasmtime 37 by
  `a_borrow_closure_handle_is_repeatable` (one `adder(10)` handle → `call(5)`=15 then `call(7)`=17 on the SAME
  handle). +1 corpus witness. This RESOLVES the design's "single biggest known hazard".
- **✅ C-HOST-6, VALUE-FORM results — the repeatable `borrow<t>` `call` extends to the byte-rope / compound /
  collection result closures `@c982ea22`.** The scalar `call` became repeatable last increment; this extends
  the SAME borrow posture to the single-export closures whose `call` returns a `list<u8>` value form — a
  byte-rope (`Bytes`/`String`), a fixed-shape compound (tuple/record/sum), and a variable-length collection
  (List/Map/Set value-encode). The host keeps ANY such handle across calls; the `t-dtor` reclaims the cell on
  drop; the transient result handle is still released each call (guest-owned scratch, separate from the cell).
  Pieces: a `call_borrow` flag on `serialize::{closure_bytes_resource_core_module,
  closure_value_resource_core_module, closure_value_encode_resource_core_module}` (via `_borrow` siblings —
  each branches the cell rep-recovery + release; `false` byte-identical to own); `envelope::
  {assemble_closure_bytes_resource_borrow, resource_inner_component_closure_bytes_borrow}` (`call` self =
  `borrow<t>` on the outer lift + the nested re-export re-typing); the three single-export value-form tails of
  `emit_closure_resource` route to borrow. PROVEN e2e by `a_borrow_compound_result_closure_handle_is_repeatable`
  (one `pair(100)` handle → two `call(5)`s → the SAME `(tuple 5 105)` value form). +1 corpus witness. **The
  SINGLE-EXPORT closure `call` is now a repeatable `borrow<t>` handle for EVERY result shape (scalar +
  value-form).**
- **✅ C-HOST-6, MULTI-EXPORT/MIXED shared `call` — the shared scalar `call` is a repeatable `borrow<t>`
  handle `@d8e0fe58`.** The ONE `call` that serves N same-signature closure exports (`make-<name>` per export)
  now takes `borrow<t>`: each make's handle survives across calls (the host keeps it, invokes the shared `call`
  repeatedly; the `t-dtor` reclaims). Pieces: `serialize::multi_closure_resource_core_module_borrow` (threads
  `call_borrow` to the shared scalar `call` body via `_with_host_borrow`); `envelope::
  {assemble_multi_closure_resource_borrow, assemble_mixed_closure_resource_borrow,
  resource_inner_component_multi_closure_borrow}` (the shared `call`'s self handle = `borrow<t>` on the outer
  lift + nested re-export; `make`s + plain exports unaffected); `emit_multi_closure_resource` +
  `emit_mixed_closure_resource` scalar tails route to borrow. PROVEN e2e by
  `a_multi_export_shared_borrow_call_is_repeatable` (one `make-inc` handle → shared `call(5)`=6 then
  `call(40)`=41). +1 corpus witness.
- **✅ C-HOST-6, MULTI-EXPORT/MIXED VALUE-FORM shared `call` — the shared-`call` borrow surface is CLOSED
  `@f047d867`.** The shared list-`call` serving N same-signature closures whose result is a `list<u8>` value
  form (byte-rope / compound / collection) now takes `borrow<t>` too, so every multi-export/mixed result shape
  (scalar + all value forms) is repeatable. Pieces: a `call_borrow` param on the three multi value-form cores
  (`multi_closure_{bytes,value,value_encode}_resource_core_module`) branching the shared `call`'s cell
  rep-recovery + release; `envelope::{assemble_multi_closure_bytes_resource_borrow,
  resource_inner_component_multi_closure_bytes_borrow}` (shared list-`call` self = `borrow<t>` on the outer
  lift + nested re-export; makes + plain unaffected); the multi + mixed value-form tails route to borrow.
  PROVEN e2e by `a_multi_export_value_form_shared_borrow_call_is_repeatable` (one `make-lo` handle → the SAME
  `(tuple 5 6)` value form on two shared calls). +1 corpus witness.
- **✅ C-HOST-6, DISTINCT-SIG per-group `call-g<n>` — the closure-`call` borrow surface is FULLY CLOSED
  `@8ac0b753`.** Each distinct-signature group's per-group `call-g<n>` (one resource type per signature) now
  takes `borrow<t_g>`, completing the borrow migration: EVERY closure `call` — single-export (scalar + all
  value forms), multi-export/mixed (scalar + all value forms), and distinct-sig per-group (all four branches)
  — is now a repeatable `borrow<t>` handle. Pieces: `serialize::distinct_sig_resource_core_module` gains a
  `call_borrow` param branching all FOUR per-group `call-g<n>` bodies (scalar / byte-rope / compound /
  collection — each skips `resource.rep-g` + the cell self-drop); `envelope::
  {assemble_distinct_sig_resource_mixed_borrow, resource_inner_component_distinct_sig_borrow}` (each group's
  `call-g<n>` self = `borrow<t_g>` on the outer lift + nested re-export; makes stay `own<t_g>`);
  `emit_distinct_sig_resource` routes to borrow. PROVEN e2e by `a_distinct_sig_call_g_is_repeatable` (one
  `make-inc` handle → `call-g(5)`=6 then `call-g(40)`=41). +1 corpus witness. **A host-held closure of ANY
  shape is now a repeatable callback — the design's own-vs-borrow fork is fully resolved on the borrow side.**
- **✅ INTRA-PROGRAM closure-param capture via an INLINE lambda arg — the β-reduction over-pinning gap is
  CLOSED `@e16bde5d`.** A returned lambda that captures a def's CLOSURE-typed parameter declined ("parameter
  reference has no local slot") when the def was applied to an INLINE lambda argument (`(def (mk (: g (-> A
  B))) (fn (x) (g x)))` given `(fn (y) (+ y 1))`). `eval::apply_lambda` pinned the whole arg subtree via
  `resolve_subtree`, freezing the arg lambda's OWN params too; when that lambda was later β-reduced inside the
  lifted returned body, its own-param refs (`y`) were shared as slot-less `Core::Param`. Fix: pin a lambda
  argument by its FREE variables only (new `syntactic_lambda` + `pin_free_vars` excluding the arg lambda's own
  params) — leaving own params unpinned to substitute on the later application. A def-ref or a let-bound
  lambda already worked; this brings the INLINE form to parity, so a closure-arg factory is now uniform across
  all three argument spellings (inline / top-level-def / let-bound), each → 6. +corpus (09-functions: the
  former "…is declined" anchor is now a working witness; stale narrative rewritten). This was the last
  intra-program reduction-engine blocker under the closures work — the boundary ABI gaps below remain, but the
  guest-side closure mechanics are complete.
- **✅ ORACLE `@54396f93` — a FIXED-SHAPE SCALAR tuple closure ARG crosses the DIRECT-CALL boundary by NATIVE
  component-tuple FLATTENING (attacks the "needs a nonexistent `value-decode` op / out of scope" claim).** The
  recorded rejection conflated two cases: a VARIABLE-LENGTH collection arg genuinely needs runtime decode, but
  a FIXED-SHAPE SCALAR tuple/record does NOT. It crosses as a native component `tuple<s64,s64>` type; the
  canonical ABI FLATTENS a small tuple (≤16 scalar fields) into scalar core params, so the guest `call`
  receives the fields as plain core i64s (no memory, no realloc, no runtime op) and rebuilds the tuple cell
  in-guest with the ORDINARY `arr-alloc`/`box-int`/`arr-set` ops. ComponentBuilder oracle
  (`closure_tuple_arg_call_core` + `inner_reexport_component_tuple_arg` + `oracle_closure_tuple_arg_component`,
  test `a_fixed_shape_tuple_closure_arg_crosses_by_native_flattening`) that VALIDATES + RUNS under wasmtime:
  `make()` → handle, `call(handle, (3,4))` supplied as a `Val::Tuple` → 7. So the direct-call compound-ARG
  decline is an IMPLEMENTATION GAP, not an ABI wall. REMAINING emit vertical (each an independently-landable
  brick, per the closure-capture cycle's lesson): (B) a serializer whose `call` rebuilds the tuple cell from
  the flattened field params + dispatches; (C) `assemble_closure_resource` variant emitting the `tuple`
  defined type in the `call` functype + inner re-export; (D) `emit_closure_resource` routes a fixed-shape
  scalar compound arg to it; (E) cdz-run supplies the arg as a `Val::Tuple`/`Val::Record`. Then the
  compound-arg direct-call todo flips to PASS.
- **✅✅ DIRECT-CALL FIXED-SHAPE SCALAR COMPOUND ARG COMPLETE — end-to-end** (bricks B `@22cb28ec`, C–E
  `@2807d76a`, corpus `@7ed43cae`). All four bricks landed + integrated: (B) `serialize::TupleArgRebuild` +
  the shared `call` serializer rebuilds the flattened-field tuple cell (`arr-alloc N` + per field box/arr-set,
  FBIP array threaded on the stack) then dispatches, dropping the rebuilt cell after (an owned per-call
  temporary); (C) `envelope::assemble_closure_resource_borrow_tuple` + `resource_inner_component_closure_
  borrow_tuple` mint a native `tuple<field-bytes…>` defined type in the `call` functype (+ `tuple_defined_
  type` / `closure_call_tuple_arg_functype` helpers); (D) `emit_closure_resource`'s `fixed_shape_scalar_tuple_
  arg` classifier feeds the flattened field valtypes as core call params + routes to (B)/(C); (E) cdz-run's
  `coerce_one` parses a `Type::Tuple` param from a `(tuple f0 f1 …)` literal into `Val::Tuple`. Corpus: a
  Tuple arg → 7, a RECORD arg (sorted-key order) → 7, a NARROW-int-field tuple (i32→i64 extend) → 123, a
  CAPTURING closure + tuple arg → 17 — all run end-to-end under wasmtime. SCOPE: single-export, EXACTLY ONE
  fixed-shape scalar tuple/record arg, scalar result, no build-time host effect. Still declines (clean): a
  compound arg with a VARIABLE-LENGTH field (needs `value-decode`), a compound arg ALONGSIDE other args,
  multi-export/round-trip variants of the compound arg, a closure-typed arg.
- **✅✅ DIRECT-CALL FIXED-SHAPE COMPOUND ARG on the MULTI-EXPORT path** (`@4d2f5582`, baseline `@f42ccc38`).
  N same-signature closures sharing one `call` whose single arg is a fixed-shape scalar tuple/record. The
  shared core serializer was ALREADY tuple-capable (brick B threaded `TupleArgRebuild` through it), so this
  was envelope + routing: `envelope::assemble_mixed_closure_resource_borrow_tuple` +
  `resource_inner_component_multi_closure_borrow_tuple` mint the `tuple<…>` type in the SHARED `call` functype
  (outer lift + nested re-export, both import + export sides), shifting the call-functype index + the
  re-exported resource index R past it; `emit_multi_closure_resource` detects the tuple arg + routes. `None` is
  byte-identical (54 closure tests + gate --check 0-regress). ⚠ INDEX TRAPS FIXED en route: the self-handle
  `own<resource>` must reference the RESOURCE index (0 import / R export), NOT the handle's own type slot (a
  `own<self>` self-reference → "type index out of bounds"); and `R = 2N+1+import_call_types` (NOT +2 — the
  re-exported `t` is the next index after all import types). Corpus: mk-sum/mk-diff share one `call`,
  `make-diff()` → handle, `call(handle, (10,3))` → 7 e2e under wasmtime. gate 1727p/0f.
- **✅✅ DIRECT-CALL FIXED-SHAPE COMPOUND ARG on the MIXED (closure+plain) path** (`@28dbfb5e`, baseline
  `@2b87d0e7`). A tuple-arg closure factory crossing via make+shared `call` ALONGSIDE a plain (non-closure)
  top-level export. Pure ROUTING — the shared core serializer (`multi_closure_resource_core_module_with_host_
  borrow` takes both `plain` + `tuple_arg`) AND the mixed envelope (`assemble_mixed_closure_resource_borrow_
  tuple` takes both `plain` + `tuple_arg_bytes`, built last tick) were already capable; `emit_mixed_closure_
  resource` detects the tuple arg, feeds flattened field vts, routes the scalar tail to the tuple serializer +
  mixed-tuple envelope with plain exports alongside. Corpus: `mk` (tuple-arg closure) + `twice` (plain) in one
  component — closure `call(handle,(3,4))`→7 AND plain `twice(21)`→42, both e2e. gate 1734p/0f.
- **✅✅ DIRECT-CALL FIXED-SHAPE COMPOUND ARG on the DISTINCT-SIG path** (`@5ddf1f28`, baseline `@63c0e7ac`).
  Closures of DIFFERENT signatures each taking a fixed-shape scalar tuple/record arg → G resource types, each
  make + `call-g<n>` taking a native `tuple<…>` rebuilt from the flattened fields. `SigGroup.tuple_arg` +
  `SigGroupAbi.tuple_arg_bytes`: the scalar `call-g` body rebuilds the cell (per-group), both per-group
  `call-g<n>` functype sites (outer lift + inner re-export, import & export phases) mint the tuple type
  advancing the running type counter by 3 (like a byte-rope group's list<u8>), `n_tuple` in the outer item
  count. 🪤 the representative-lifted-slot match is on the lambda's OWN param shape (`match_vts` = `[I32]`, the
  tuple-cell handle) NOT the flattened `arg_vts` — else "no matching lifted lambda". A tuple arg + a byte-rope/
  compound/collection result declines cleanly. Corpus: mk-sum(→Int64) + mk-eq(→Bool) both taking a Tuple;
  `call(sum,(3,4))`→7 AND `call(eq,(5,5))`→true e2e. gate 1741p/0f.
- **🐛→✅ MISCOMPILE FIXED: a compound ARG + a compound/byte-rope/collection RESULT now DECLINES** (`@68ac2bd8`,
  baseline `@e7b00e12`). A latent miscompile across single/multi/mixed direct-call paths (distinct-sig already
  guarded): a fixed-shape compound ARG detection set `arg_vts` to the flattened fields, but the list-returning
  RESULT cores (`closure_bytes_/value_/value_encode_resource_core_module`) inline their own `call` bodies +
  do NOT thread the `TupleArgRebuild`, and their envelopes take the scalar `arg_bytes` (empty for a tuple arg)
  — so the two combined emitted a scalar-arg envelope over a flattened-field core (wasmtime "lowered param
  types [I32] do not match [I32,I64,I64]" / "failed to parse"). Guards in `emit_closure_resource`/
  `emit_multi_closure_resource`/`emit_mixed_closure_resource` now decline cleanly; a scalar-result compound
  arg still emits. +1 corpus todo anchor + `a_compound_arg_with_a_compound_result_declines_not_miscompiles`.
  🔑 the FIX for this = thread `TupleArgRebuild` through the 3 list-result serializers + their envelopes (the
  next real widening); this tick made it an honest decline first (correctness over the miscompile).
- **✅ TUPLE ARG + BYTE-ROPE RESULT (single-export)** (`@5e97fafe`, baseline `@c4299d05`). The FIRST of the
  compound-arg + list-result widenings: a closure taking a fixed-shape scalar tuple/record arg AND returning
  Bytes/String now compiles + runs. Factored `serialize::emit_tuple_rebuild` + `emit_tuple_rebuilt_drop` free
  helpers (shared by every `call` body); threaded `tuple_arg` into `closure_bytes_resource_core_module_borrow`
  (rebuild the cell before dispatch, drop after the copy). Envelope: `closure_call_list_tuple_arg_functype` +
  `assemble_closure_bytes_resource_borrow_tuple` + `resource_inner_component_closure_bytes_borrow_tuple` mint
  the `tuple<…>` type before the `list<u8>` result type (running type counter, both import + export sides).
  `emit_closure_resource`'s guard relaxed to allow byte-rope + tuple arg (still declines compound/collection
  result). Corpus: `(fn (p) (bin (u8 (.p 0)) (u8 (.p 1))))`, `call(handle,(5,6))`→(5 6) e2e.
- **✅ TUPLE ARG + FIXED-SHAPE COMPOUND RESULT (single-export)** (`@db6110b9`, baseline `@6a59dc64`). The
  SECOND widening: a closure taking a fixed-shape scalar tuple/record arg AND returning a fixed-shape compound
  (tuple/record/sum) now compiles + runs. Threaded `tuple_arg` into `closure_value_resource_core_module_borrow`
  (reusing the shared `emit_tuple_rebuild`/`_drop` helpers) + routed the compound-result branch to the shared
  list<u8> tuple envelope. 🪤 the i64 `scratch` local sits AFTER all i32 locals → its index = `cell + n_i32`
  (a tuple arg adds a 3rd i32, shifting scratch by 1); getting it wrong gave a wasmparser "expected i64, found
  i32". Corpus: tuple-arg → `(tuple 13 7)`, record-arg → `(record (diff 7) (sum 13))`; a tuple-arg + List
  result decline anchor.
- **✅✅ TUPLE ARG + COLLECTION RESULT (single-export) — the single-export tuple-arg result matrix is CLOSED**
  (`@8a1a539b`, baseline `@3fca5268`). The final single-export widening: a closure taking a fixed-shape scalar
  tuple/record arg AND returning a variable-length collection (List/Map/Set) now compiles + runs. Threaded
  `tuple_arg` into `closure_value_encode_resource_core_module_borrow` (reusing `emit_tuple_rebuild`/`_drop`; a
  7th i32 local for the rebuilt arg cell) + routed the value-encode branch to the shared list<u8> tuple
  envelope. The `emit_closure_resource` result-shape decline guard is GONE — a single-export tuple arg composes
  with EVERY result shape (scalar / byte-rope / fixed-compound / collection). Corpus: tuple-arg → `(list 10 3)`,
  Map → `(map (1 100) (2 200))`. 🎯 SINGLE-EXPORT: a fixed-shape scalar compound arg × every result shape DONE.
- **✅ TUPLE ARG × LIST-RESULT on the MULTI-EXPORT path** (`@8f1a08f1`, baseline `@72cb720e`). Extended the
  tuple-arg × list-result composition to N same-sig closures sharing one list-returning `call`. Threaded
  `tuple_arg` into all three multi list-result cores (`multi_closure_bytes_/value_/value_encode_resource_core_
  module`, shared helpers) + `assemble_multi_closure_bytes_resource_borrow_tuple` +
  `resource_inner_component_multi_closure_bytes_borrow_tuple` (running type counter mints the `tuple<…>` before
  `list<u8>`). The `emit_multi_closure_resource` list-result decline guard is GONE. Corpus: mk-rev → (list 3
  10), mk-sum → (tuple 13 7).
- **✅ TUPLE ARG × LIST-RESULT on the MIXED path** (`@a57eba46`, baseline `@7cf64e35`). The trivial follow-on:
  a list-returning tuple-arg closure ALONGSIDE a plain export reuses the SAME multi list-result cores + the
  shared multi list<u8> tuple envelope. Pure ROUTING — `emit_mixed_closure_resource`'s list-result guard GONE,
  its 3 branches thread `tuple_arg` + route to the tuple envelope with the plain exports riding alongside.
  Corpus: a List-returning tuple-arg closure `mk` + plain `twice` → closure (list 10 3) + plain twice(21)=42.
- **✅✅ TUPLE ARG × LIST-RESULT on the DISTINCT-SIG path — the tuple-arg × RESULT × EXPORT matrix is CLOSED**
  (`@ed3e160e`, baseline `@fbe5f259`). The last list-result gap: closures of DIFFERENT signatures each taking a
  fixed-shape scalar tuple arg AND returning a list<u8>-crossing result, each per-group `call-g<n>` rebuilding
  its own flattened tuple arg. Threaded `gr.tuple_arg` into all three per-group list-result `call-g` branches
  (collection/value-form/byte-rope, shared helpers); the per-group envelope functype sites (outer + inner
  import/export) emit a 4-type block (handle + tuple + list<u8> + `(self,tuple)->list<u8>`) for a both-group,
  `n_bytes + n_tuple` counting the extra types. `emit_distinct_sig_resource`'s guard GONE + the used-ops set
  gains the rebuild ops (arr-alloc/arr-set + per-field box ops the groups reference — else `import_index` panic
  for a group whose body builds no cell). Corpus: mk-a (Tuple Int64 Int64 → List) + mk-b (Tuple Int64 Bool →
  List) of distinct sigs → (list 10 3) / (list 7). 🎯 A FIXED-SHAPE SCALAR compound closure ARG now composes
  with EVERY result shape (scalar/byte-rope/fixed-compound/collection) across ALL FOUR export shapes
  (single/multi/mixed/distinct-sig). The direct-call tuple-arg surface is complete.
- **✅ A fixed-shape scalar tuple ARG can sit AMONG scalar args** (`@e934430d`, baseline `@54ee91ff`; single-
  export, scalar result). Generalized the arg model from "the tuple is the SOLE arg" to "exactly one tuple at
  ANY position among aliased-width scalars". `TupleArgRebuild` gained `base_param` (the core-param index the
  flattened fields start at = `1 + prefix-scalar-count`); `emit_tuple_rebuild` reads `base_param + i`; the
  shared scalar `call` body pushes prefix scalars, the rebuilt tuple, suffix scalars, in the closure's arg
  order. Envelope: `closure_call_tuple_arg_functype_interleaved` (self + prefix bytes + tuple + suffix bytes);
  `assemble_closure_resource_borrow_tuple` + the inner re-export thread `tuple_prefix_bytes`/`tuple_suffix_
  bytes`. `single_compound_among_scalars` detects it. 🪤 the envelope's `tuple<…>` type takes the tuple's OWN
  field bytes (NOT the full flattened list) — else a param-count mismatch. Corpus: scalar-then-tuple → 113,
  tuple-then-scalar → 113, scalar-tuple-scalar → 114. Still declines: an among-scalars tuple with a LIST result
  (list-result cores don't yet interleave — a follow-on); multi/mixed/distinct-sig among-scalars.
- **✅ tuple-among-scalars on the MULTI-EXPORT path (scalar result)** (`@a7f5470f`, baseline `@b5c84cd3`). N
  same-sig closures each taking a tuple among scalars share one interleaving `call`. `emit_multi_closure_
  resource` uses `single_compound_among_scalars`; `assemble_mixed_closure_resource_borrow_tuple` +
  `resource_inner_component_multi_closure_borrow_tuple` thread `tuple_prefix_bytes`/`tuple_suffix_bytes` into
  the shared `call` functype (via `closure_call_tuple_arg_functype_interleaved`). Corpus: mk-a → 113, mk-b →
  87. Mixed keeps sole-tuple only; among-scalars + list result still declines.
- **✅ tuple-among-scalars composes with EVERY result shape on the SINGLE-export path** (`@d2c1737f`, baseline
  `@aa899608`). Removed the single-export among-scalars + list-result decline: a tuple among scalars now works
  with a byte-rope, a fixed-shape compound value-form, and a collection value-encode result. New shared
  `serialize::emit_closure_call_args(tuple_arg, tuple_local, arity, imp)` pushes prefix scalars → rebuilt tuple
  → suffix scalars (or the raw scalars when None) — replacing the 6 sole-tuple `if let…else` blocks in the
  three list-result cores + the scalar multi core + the 3 distinct-sig cores with ONE call (byte-identical for
  a sole tuple / None). `assemble_closure_bytes_resource_borrow_tuple` +
  `resource_inner_component_closure_bytes_borrow_tuple` thread prefix/suffix into the list<u8> `call` functype
  via `closure_call_list_tuple_arg_functype_interleaved`. Corpus: among-scalars × List/Bytes/compound results
  + tuple-before-scalar × List, all e2e. Still declines: among-scalars + list result on the MULTI/MIXED/
  DISTINCT-SIG paths (their emit fns don't yet interleave — the helpers now exist, a mechanical follow-on).
- **✅ tuple-among-scalars composes with EVERY result shape on the MULTI-EXPORT path** (`@d2c1737f`→landed
  next). The three multi list-result cores already interleave via the shared `emit_closure_call_args`; the
  only wiring was the envelope functype — `assemble_multi_closure_bytes_resource_borrow_tuple` +
  `resource_inner_component_multi_closure_bytes_borrow_tuple` gained `tuple_prefix_bytes`/`tuple_suffix_bytes`
  feeding `closure_call_list_tuple_arg_functype_interleaved`. The multi among-scalars decline is GONE. Corpus:
  multi-export among-scalars × List/Bytes/compound, e2e. The MIXED path still detects a SOLE tuple only.
- **✅ tuple-among-scalars composes with EVERY result shape on the MIXED path** (landed after the multi path).
  `emit_mixed_closure_resource` switched from the 3-tuple sole-tuple detection to the 5-tuple
  `single_compound_among_scalars` (like multi-export); `arg_vts` = the full flattened core param list; the
  scalar tail + the 3 list-result routings pass the tuple's prefix/suffix to
  `assemble_mixed_closure_resource_borrow_tuple` (already threads them). NO serializer/envelope change. Corpus:
  scalar-then-tuple closure beside a plain export × scalar/List/compound, e2e. ⚠ a semantic rebase conflict: a
  sibling's `emit_mixed_closure_resource` edit left a 3-tuple `if let Some((_,_,rebuild))` at the used-ops
  scan — git didn't flag it textually; caught by the build.
- **✅ tuple-among-scalars composes with EVERY result shape on the DISTINCT-SIG path — the LAST direct-call
  arg-position gap CLOSED** (landed after the mixed path). `emit_distinct_sig_resource` detects each group's
  arg via `single_compound_among_scalars`; `GroupInfo.tuple_arg` carries prefix/suffix; `arg_vts` = the full
  flattened core params; `match_vts` maps each arg to its lambda-param valtype (a tuple → one i32 cell, a
  scalar → itself). The last inline sole-tuple push (the distinct-sig SCALAR-result `call-g` body) now uses the
  shared `emit_closure_call_args`, so ALL per-group `call-g` bodies interleave. `SigGroupAbi` gained
  `tuple_prefix_bytes`/`tuple_suffix_bytes`; the 4 per-group functype sites use the interleaved forms; the
  now-unused non-interleaved wrappers were removed. 🪤 the shared-helper edit dropped the env `local.get` before
  the args — caught by wasm-tools validate (`func 17 failed`), re-added. Corpus: two DISTINCT-sig scalar-then-
  tuple closures (Int64-pair + Int64/Bool) × scalar/List results, e2e. **A fixed-shape scalar tuple ARG among
  scalar args is now supported on ALL FOUR export shapes (single/multi/mixed/distinct-sig) for EVERY result
  shape (scalar/byte-rope/fixed-compound/collection) — the direct-call tuple-among-scalars surface is CLOSED.**
- **✅ a RECORD closure argument is now DRIVABLE end-to-end** (cdz-run `dd9e6530`). A record arg already
  COMPILED (it erases to a component `tuple<…>` in canonical SORTED-NAME order via `tuple_field_abi`/
  `Core::Record`'s `BTreeMap`), but the cdz-run corpus driver could only supply a positional `(tuple …)` — a
  record value `(record (x 10) (y 3))` failed to coerce. Fixed in `coerce_one`'s `Type::Tuple` arm: when every
  parsed field is a `(name value)` group, sort the fields by name (matching the boundary tuple's sorted-key
  layout) + unwrap each to its value. Proven sound by an OUT-OF-SORTED-ORDER record (`(z, a)` → boundary
  `(a, z)` → `r.z - r.a` = 97, not a positional coincidence). A TEST-HARNESS fix, not a compiler change.
  Corpus: record arg — sole/among-scalars/out-of-order/narrow-Bool-field/List-result/multi-export + a 3-field
  tuple + a 2-prefix/1-suffix interleaving.
- **✅ tuple-arg field-type variety witnessed** (`ef863bb3`, corpus-only). The flatten/rebuild is field-type
  agnostic: FLOAT fields (→4.0), MIXED widths (Int32+Int64→42), a Bool among ints (→110, box-bool), and a
  single-variant NOMINAL over a tuple (erases to the tuple, §156 kind-agnostic peel — supply the bare tuple).
- **✅ direct-call tuple-arg × RESULT matrix filled: String + Sum results** (`1ef864ed`, corpus-only). A
  sole-tuple-arg closure composes with a String result (byte-rope core, → bytes) AND a Sum/Option result
  (value-encode walker, → `(Some 5)`) — the matrix now covers scalar / byte-rope (Bytes+String) / fixed-
  compound / collection (List+Map) / sum. (A Char tuple field declines project-wide: `valtype_of(Ty::Char)=None`
  — no runtime Char slot yet, cross-cutting, not closure-specific.)
- **✅ NESTED fixed-shape compound ARG — vertical COMPLETE (single-export scalar-result)** (3/3 bricks). Brick 1
  (`274398d9`, oracle) proved `tuple<s64, tuple<s64,s64>>` flattens RECURSIVELY to 3 leaf core params (no
  `value-decode`). Brick 2 (`9361f77f`, byte-neutral) made the CORE rebuild recursive (`FieldRebuild` tree +
  `emit_cell_rebuild` threading a leaf cursor). Brick 3 (`c8d193b7`) added the envelope's recursive type mint:
  `TupleFieldShape` + `mint_tuple_type_nested` (inner tuples first, referenced by sleb128 type-index) +
  `nested_tuple_type_count`; `assemble_closure_resource_borrow_tuple` + its inner component gained a
  `tuple_shape` param; `mod.rs::nested_fixed_shape_tuple_arg` recurses. e2e: tuple-in-tuple/record-in-record/
  tuple-in-record/3-deep/nested-Bool-leaf, all → correct under wasmtime. SCOPE: single-export, scalar result.
- **✅ NESTED compound ARG × list<u8>-crossing RESULTS (single-export)** (`39da0d6a`). Widened the nested arg
  to EVERY result shape on the single-export path: byte-rope, fixed-shape compound value-form, and
  variable-length collection. `assemble_closure_bytes_resource_borrow_tuple` + its inner component gained the
  same `tuple_shape` param + recursive mint; the 3 list-result routings thread a shared `list_rebuild`/
  `list_shape` (falling back from flat `tuple_arg` to `nested_tuple`). e2e: nested tuple/record arg ×
  List/Bytes/compound. The single-export nested-compound-arg surface is now COMPLETE across all result shapes.
- **✅ NESTED compound ARG on the MULTI-EXPORT path** (`3f217430`, scalar + list<u8> results). N same-sig
  closures each taking a sole nested tuple/record arg share one `call`. `emit_multi_closure_resource` binds
  `nested_tuple`; the multi/mixed envelopes + their inner components gained the same `tuple_shape` param (the
  `tuple_shift` / running type counters absorb `nested_tuple_type_count` extra types). e2e: mk-a→113, mk-b→87.
- **✅ NESTED compound ARG on the MIXED path** (`374078bf`, scalar + list results). A nested-tuple-arg closure
  exported ALONGSIDE a plain export. PURE ROUTING in `emit_mixed_closure_resource`.
- **✅✅ NESTED compound ARG on the DISTINCT-SIG path — the nested-arg matrix is CLOSED** (`f1e2077f`). Closures
  of DIFFERENT signatures each taking a sole nested tuple/record arg cross as G distinct resource types, each
  per-group `call-g<n>` rebuilding its nested cell + minting its inner `tuple<…>` types by index.
  `emit_distinct_sig_resource` gained a per-group `group_nested` fallback + `GroupInfo.nested_shape`;
  `SigGroupAbi.tuple_shape`; the 4 per-group mint sites mint via `mint_tuple_type_nested`; `n_tuple` sums
  `nested_tuple_type_count`. e2e: two DIFFERENT nested sigs → 113, 110 (Bool leaf at depth), × List.
  **A nested fixed-shape compound ARG now crosses on ALL FOUR export shapes (single/multi/mixed/distinct-sig)
  for every result shape.**
- **✅ NESTED compound ARG AMONG scalar args (single-export)** (`d818f772`, scalar + list results). Composes
  the nesting + interleaving features: a nested tuple/record at any position among aliased-width scalars. New
  `nested_compound_among_scalars` classifier (mirrors `single_compound_among_scalars` via
  `nested_fixed_shape_tuple_arg`); `NestedCompoundArgBoundary` gained prefix/suffix; the recursive rebuild's
  `base_param` shifts past the prefix + the interleaved envelope functype already surrounds the minted types.
  Also witnessed a record-with-a-tuple-field + a triply-nested record (already worked). e2e: prefix→1113,
  prefix+suffix→1114, × List.
- **✅ NESTED compound ARG AMONG scalars on the MULTI-EXPORT + MIXED paths** (`8b6fc532`, scalar + list results).
  Extracted a shared `nested_sole_or_among_scalars` classifier all 3 emit paths' `nested_tuple` binding calls;
  the multi/mixed `tpre`/`tsuf` + nested scalar branches thread the nested prefix/suffix. No new serializer/
  envelope. e2e: multi scalar-then-nested (mk-a→1113, mk-b→887), mixed × List.
- **✅✅ NESTED compound ARG AMONG scalars on the DISTINCT-SIG path — the NESTED-ARG MATRIX IS FULLY CLOSED**
  (`63065513`). `emit_distinct_sig_resource`'s per-group `group_nested` uses the shared
  `nested_sole_or_among_scalars`; `arg_vts` = full flattened, `match_vts` = per-arg (compound → i32 cell), the
  per-group `tuple_arg` carries the nested prefix/suffix so `SigGroupAbi` interleaves the `call-g` functype. No
  new envelope/serializer. e2e: two DIFFERENT-sig scalar-then-nested → 1113, 1100 (Bool leaf), × List. **A nested
  fixed-shape compound ARG now crosses on ALL FOUR export shapes (single/multi/mixed/distinct-sig), SOLE or
  AMONG scalars, for every result shape.**
- **✅ DEEPER direct-call compound RESULT shapes witnessed** (`dcedfc29`, corpus-only). The value-form template
  + value-encode walker descend arbitrarily + compose with the arg rebuild — a nested record result, a
  Tuple-arg × nested-tuple result, a tuple-with-a-List (compound-with-collection), a nested-arg × nested-result,
  a Sum-of-tuple, and a List-of-tuples all cross + decode on the direct-call path (all ALREADY worked; corpus
  lagged). The direct-call RESULT surface is as deep as the arg surface.
- **REMAINING (all optional, none blocking) — the DIRECT-CALL arg frontier, all HOST→GUEST transfer:** these
  are GENUINE declines (confirmed by probing, distinct from the record-DRIVER test-harness gap). (1) **N
  compound args** (two tuple args) — `single_compound_among_scalars` rejects >1 tuple; `TupleArgRebuild` + ~65
  envelope sites + 16 `tuple_defined_type` mint sites assume EXACTLY ONE tuple; a `Vec<TupleArgRebuild>`
  generalization is a large multi-tick vertical. (2) a **SUM
  (Option) direct-call arg** (needs host→guest decode of
  the discriminant+payload). (4) a **VARIABLE-LENGTH collection arg** (needs a `value-decode` runtime op that
  does not exist). (⚠ the ROUND-TRIP path where the CONSUMER builds the arg in-guest ALREADY works for all of
  these — no direct-call round-trip gap.) A closure-typed closure ARG on the direct-call path (a closure-
  resource passed INTO a call); a closure TRANSFORMER (`own<t>` both directions — cleanly declined). **The entire byte-rope
  (`Bytes`/`String`) result surface, the entire fixed-shape compound (tuple/record/sum) result surface, AND
  the variable-length collection (List/Map/Set) result surface are ALL DONE across EVERY closure shape —
  single-export + multi-export + mixed + distinct-sig + round-trip + distinct-sig-round-trip; the complete
  closure-RESULT matrix is closed. Every MACHINE-REPRESENTABLE closure ARGUMENT (scalar, compound, sum,
  nested, String, collection, closure-typed — annotated or not, flat OR curried multi-arg arrow) is now
  supported on BOTH round-trip paths (built in-guest). The remaining gaps are all DIRECT-CALL host→guest
  transfer + the `borrow<t>`/transformer frontier.**

## Risks / open questions

1. **borrow<t> trap (C-HOST-5)** — the single biggest known hazard, inherited from the
   escape's deferred follow-up. Start with own/no-drop (C-HOST-1..4) to keep the gate
   green, then resolve.
2. **Interface naming** — DECIDED: a dedicated `cadenza:closure/*` interface (host
   contract is a callable method, distinct from the value-escape's `encode`).
3. **Arg/result boundary types** — the DIRECT-CALL path restricts `call` args + result to the
   aliased scalar widths (same restriction as host-call `abi_val_type`), because the host supplies
   the argument over the boundary. RESOLVED for the ROUND-TRIP path (`@3f9ff427` single-sig,
   `@4e8df79f` distinct-sig, `@79ada1f3` widened to every machine type): there the closure is applied
   in-guest, so ANY machine-representable arg — a compound, sum, nested, String, OR a closure-typed
   (higher-order) arg — is built guest-side and need only be machine-representable (an i32 handle /
   funcref slot). A compound/closure-typed arg on the direct-call path (host→guest decode) + an inner
   closure whose own param is a compound (a lifted-lambda-param fence) remain later increments.
4. **Lifetime / RC** — who drops the closure cell and its captures? Tied to own vs borrow;
   the general Perceus drop work (`a_runtime_closure_leaks_exactly_one_cell_known_gap`)
   and the resource dtor converge here.
5. **Gate expressibility** — the gate's `(call main …)` drives a bare export; driving a
   resource METHOD needs a small `cdz-run` + corpus extension (a `(call-closure …)` shape,
   or reuse `(call …)` against the resource). Non-trivial but bounded.

## What this is NOT (scope fences)

- Not a host-IMPLEMENTED Cadenza function (host defines the body). Deferred — needs an
  import-side resource + a second dispatch path.
- Not closures with compound/closure args crossing (first cut = scalar args/result).
- Not a change to intra-program closures (complete and unaffected).
