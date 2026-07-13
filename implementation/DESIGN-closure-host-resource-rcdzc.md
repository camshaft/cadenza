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
  (blocked on the wasmtime-37 borrow trap — an upstream fix or a workaround); (2) widen closure
  args/result beyond aliased scalar widths; (3) distinct-signature multi-export (N resource types); (4) a
  consumer with MORE than one closure param; (5) a compound/closure-typed closure ARG. The core vertical
  (Direction 1 + the round-trip + leak-free) is COMPLETE.

## Risks / open questions

1. **borrow<t> trap (C-HOST-5)** — the single biggest known hazard, inherited from the
   escape's deferred follow-up. Start with own/no-drop (C-HOST-1..4) to keep the gate
   green, then resolve.
2. **Interface naming** — DECIDED: a dedicated `cadenza:closure/*` interface (host
   contract is a callable method, distinct from the value-escape's `encode`).
3. **Arg/result boundary types** — first cut restricts `call` args + result to the aliased
   scalar widths (same restriction as host-call `abi_val_type`); a closure whose arg is
   itself a compound/closure is a later increment (recursion into the resource machinery).
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
