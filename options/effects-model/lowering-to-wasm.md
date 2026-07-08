# Effects Model — Lowering `handle` / perform / `resume` to WebAssembly

> **Companion to [`algebraic-one-shot.md`](./algebraic-one-shot.md).** That document pins *what*
> Cadenza's effects mean — a host-delegated effect is a plain imported-function call the host resolves
> (resumption strategy is host policy); intra-program effects are
> algebraic operations discharged by lexically scoped handlers with one-shot continuations. This
> document pins *how* the intra-program layer is compiled to WebAssembly by a compiler that emits raw
> component bytes (the seed, `implementation/seed/crates/cdz-compiler/src/codegen.rs`; and, later, the
> Cadenza-authored compiler). The guarantees are not replaceable — this fixes the operational lowering,
> the observable behavior is `algebraic-one-shot.md`'s.
>
> Grounded in a 2026-07-06 SOTA research pass (Koka evidence-passing, Effekt lexical capabilities,
> OCaml 5 one-shot, selective CPS, defunctionalized continuations-as-data, the wasm stack-switching
> proposal, Asyncify, the component-model async ABI, Temporal/Unison durable replay, and compile-time
> handler classification), each load-bearing claim adversarially fact-checked against primary sources.
> See [`spec/learnings/2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code.md`](../../spec/learnings/2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code.md).

## The decision, in one paragraph

Lower intra-program effects by a **classification-first** strategy: a compile-time pass sorts each
handler arm by how it uses `resume` into **tail-resumptive**, **abortive**, or **general one-shot**, and
lowers each class to a different, minimal wasm shape. Because Cadenza resolves the discharging handler
**statically at compile time** — the nearest handler enclosing the perform in *dynamic extent* (the call
chain), determined statically by monomorphizing the handler context (`capabilities-and-effects.md` §Handler
Resolution Is Dynamic In Extent And Statically Determined) over a **closed effect row** (`type-system.md`
§The Effect Row Is A Row Over The Same Machinery) — the handler discharging any performed operation is a
compile-time constant, so there is **no runtime handler search, no runtime evidence vector**. And because
every operation the corpus performs and every effect the self-hosting compiler carries (`Fresh`, `Diag`,
`Unify`) is **tail-resumptive**, the entire shipping surface lowers with **zero continuation machinery**:
`perform` becomes a direct call to (or an inline of) the statically-known handler arm, and a tail
`(resume value next-state)` becomes just the value `value` (the accumulator threads through `next-state`).
The one class that genuinely needs a reified continuation — **general non-tail one-shot** — is a bounded
fallback (a defunctionalized frame on the existing value-heap runtime) whose trigger is precise and which
no current program reaches; until built, a non-tail `resume` is a clean decline (reject-don't-miscompile).

**Everything here ships in stock WebAssembly today** — direct calls, `block`/`br`/`br_table`, locals, and
the frozen value-heap imports are all core wasm. No arm of this design depends on a wasm proposal.

## Why not the alternatives (rejected, with grounds)

- **Native stack-switching (`cont.*` / WasmFX / "typed continuations").** Rejected as the mechanism on
  three independent, verified grounds. (1) *Not shipping:* it is work-in-progress on x86_64 Cranelift only
  and sits in **no** Wasmtime stability tier (the older "Tier 3" characterization is stale). (2) *Slow:* the
  only published numbers put it **>4× slower than Asyncify**, and it lacks the tail-resumptive inlining that
  makes the common case cheap. (3) *Contract-breaking:* a `cont` is an **opaque native stack** that cannot
  cross the component boundary (proposal issue #128) and cannot be reconstructed by a host that re-derives a
  run from `(input, responses)` (capabilities-and-effects.md §A Run Is A Deterministic Function Of Its Input
  And Responses) — the decisive disqualifier for intra-program use. Admissible only
  as a far-future, purely-local, never-migrated optional fast path once it reaches a Wasmtime tier.
- **Whole-program Asyncify (Binaryen post-pass).** Rejected: one-shot-only, large code-size multiplier, a
  whole-module rewrite that fights a compiler emitting exact bytes, and its rewind re-enters already-run code
  (a metering hazard). Its *idea* — a resumable state machine with locals spilled to linear memory — is
  borrowed, but **emitted directly by the compiler for only the effectful region** in the Tier-3 fallback,
  not run as a global pass.
- **Selective CPS as the primary strategy.** Rejected as primary: correct and elegant (and the theoretical
  backbone of tail-resumptive lowering), but transforming even effect-typed functions into
  continuation-passing form is heavier than the classification-first path needs, since Cadenza's static
  resolution collapses the interesting cases to direct code. CPS reasoning survives *inside* the Tier-3
  fallback (a defunctionalized one-shot continuation is a first-order CPS).

## The two mechanisms are separate and never share a continuation object

Cadenza routes one kind of declared effect two ways, and the two routings have **opposite state
ownership**. The routing is a property of the enclosing router — a lexical `(handle …)` or the
entrypoint's `(host …)` delegation — **not** of the effect's declaration (which is a routing-agnostic
contract; `capabilities-and-effects.md` §Host-Binding Is A Routing Decision Made At The Entrypoint):

- **Delegated to the host** (an operation whose nearest enclosing router is an entrypoint `(host …)`
  delegation) — discharged at the component boundary as a **plain imported-function call that returns its
  response**. The entry is a plain function `input -> output` (`component-abi.md` v4 §The Entry Is A Plain
  Function — no `Suspended` arm, no injected trap arm). How the host resolves the call — inline, suspend a
  fiber and resume in place, or tear down and re-derive from recorded responses — is host policy the bytes
  don't encode; the language guarantees only determinism in `(input, ordered responses)`. **Decided;
  unchanged by this document** (this doc is about the intra-program layer).
- **Handled in-program** (an operation whose nearest enclosing router is a lexical `(handle …)`) —
  discharged by that handler. Never enters the manifest; never crosses the component boundary. **This
  document.** Because routing is lexical, a `handle` nearer the perform than the entrypoint's delegation
  interposes — the same operation the entrypoint would have delegated is serviced in-program instead
  (mock/count/cache), and only what no handler catches reaches the boundary.

**A reified intra-program continuation must never be confused with durable state.** An intra-program
continuation (only ever built in the Tier-3 fallback) is a chain of **non-durable opaque heap handles**
(`component-abi.md` §A Runtime Handle Is Meaningful Only Within The Single Run) — valid within one run and
reconstructed by recomputation if a host re-derives the run, **not** reconstructible from `(input,
responses)` on its own. This is exactly why native stack-switching is rejected for intra-program use: its
continuation is a native stack a re-derivation strategy cannot reconstruct as data.

## Handler classification

A single type-directed pass — the same pass that computes `CDZ0401`/`CDZ0403` — classifies **each handler
arm** by how its body uses `resume`. It is decidable because the discharging handler is statically
determined and an effect's operation set is a closed declared set. For an arm
`(E.op (p₁ … pₙ) <state> arm-body)` — the state binder follows the operation parameters — treating `resume`
as bound only within this arm (never crossing into a nested `(fn …)` or nested `(handle …)`):

- **TAIL-RESUMPTIVE** — `resume` occurs **exactly once** and in **tail position** of every control path of
  `arm-body`, and the continuation is not otherwise referenced. The canonical form (from *Effect Handlers,
  Evidently*, Xie & Leijen) is `op ↦ λx.λs.λk. k value next-state` with `k ∉ fv(value, next-state)`. Every
  handler threads state (`(handle <init> (arms…) body)` seeds it; each arm binds the current state; `resume`
  carries `value` and `next-state`), so a tail arm is lowered by one **state-passing transform** —
  `capabilities-and-effects.md` §A Handler Threads State Across The Operations It Discharges. Two shapes of
  the same transform:
  - **UNIT-STATE (the degenerate, zero-cost case)** — the handler's state kind is `Unit` (seed `unit`, arm
    threads `s` unchanged) and `value` is a pure expression over the arm parameters. Because `Kind::Unit`
    emits no bytes, the transform threads a zero-width value and the emitted code is byte-identical to a
    stateless inline. This is the whole current unit-state corpus (`Choose`/`Get`/`Scale`/`Unify`/`Scope`).
  - **STATEFUL fold** — a non-unit accumulator threaded through the handled region: `Fresh` (a counter,
    `resume s (+ s 1)`), `Diag` (a growing list, `resume unit (List.push s code)` with a `collect` read-out
    `resume s s`), the compiler's `Unify` store. Same transform, non-trivial state. A read-out is an
    ordinary operation whose arm resumes the current state; there is no separate return clause.
  - **FORWARDING (effectful-tail)** — `resume` occurs exactly once in tail position, but the resumed
    `value` expression **itself performs an operation** (`k ∉ fv` still holds — the continuation is not
    captured). The canonical case is an *interposing* handler whose arm re-performs the operation it is
    discharging (`(ask.ask () s (do (Count.tick) (resume (ask.ask) s)))`): it observes/counts/mocks, then
    forwards to the next-outer handler. Still Tier 1, still no continuation object — `resume value s → value`
    (state `s` threaded) exactly as for the unit/stateful cases, the only difference being that `value` is
    emitted under the arm's **definition-site** handler stack (the under-frame), so its nested perform
    resolves to the *parent* handler (up to the host boundary) rather than re-entering this arm. Do **not**
    let "`value` is effectful" push the classifier to GENERAL ONE-SHOT — the tail+affine test is unchanged
    and this is the mechanism that makes host-effect interposition (mock/count/cache) free.
- **ABORTIVE** — `resume` does not occur in `arm-body`. The handler discards its continuation
  (exception / early-exit shape).
- **GENERAL ONE-SHOT** — `resume` occurs, but not in tail position, or the continuation is captured (stored,
  returned, or applied beyond the resumed value). Needs a reified continuation (Tier 3). A second `resume`
  under the affine default (`capabilities-and-effects.md` §A Continuation Is One-Shot By Default) is a
  **compile-time rejection**, not a class.

**The classifier is conservative:** anything not *provably* tail-resumptive or abortive is GENERAL
ONE-SHOT. Mis-classing a non-tail arm as tail would silently drop post-resume work — a miscompile. The check
is purely syntactic and exact; the tier is the least upper bound over *all* control paths, so a runtime
branch inside an arm never changes it (the reify-or-not decision is made at the perform site, upstream of
the arm body).

### Every current corpus arm is tail-resumptive

Verified against `spec/semantics/14-effects-and-handlers.sexp`:

| Case | Arm | `resume` form | Class |
|---|---|---|---|
| `Choose.pick` | `(pick () s (resume 5 s))` | tail, constant value, unit state | unit-state |
| `Get.get` | `(get () s (resume 41 s))` | tail, constant value, unit state | unit-state |
| `Scale.by` | `(by (n) s (resume (* n 2) s))` | tail, pure over `n`, unit state | unit-state |
| `Fresh.next` | `(next (u) s (resume s (+ s 1)))` | tail, reads & advances state | stateful |
| `Diag.emit` | `(emit (code) s (resume unit (List.push s code)))` | tail, accumulates state | stateful |
| `Diag.collect` | `(collect (u) s (resume s s))` | tail, reads state out | stateful |
| `Unify.resolve` | `(resolve (x) s (resume (+ x 1) s))` | tail, pure over `x`, unit state | unit-state |
| `Scope.resolve` | `(resolve (x) s (resume x s))` | tail, identity, unit state | unit-state |

The self-hosting driver's own effects — `Fresh` (fresh-name supply, a folded counter), `Diag`
(diagnostics, a folded list read out by `collect`), `Unify` (a state store) — are all tail-resumptive and
fold state through the one transform. **The entire shipping surface is tail-resumptive**, so the fast path
*is* the whole initial implementation; the unit-state cases ride the zero-cost degenerate form.

## Lowering, per class

### The statically-resolved handler is compile-time knowledge, not a runtime vector

Nearest-enclosing resolution (the handler active on the call chain) plus a monomorphized closed row
collapse Koka's runtime evidence vector to a compile-time stack of handler frames on the compiler's
function context. Resolution is *dynamic in extent* — a perform can be discharged by a caller's handler,
not one lexically enclosing the performing definition — but *statically determined*: within a function the
stack is walked directly, and across function boundaries the effect is an implicit evidence parameter and
each effectful function is monomorphized once per handler context it is called under (see §Effect-context
monomorphization). There is no `(marker, handler)` pair, no array index, no `find_ev`.
`(handle <init> (arms…) body)` pushes a frame of classified arms (seeded with the initial state), emits
`body`, pops. A perform `(E.op args)` resolves `E.op` against that stack top-down; the result is a single
concrete arm and its class — a compile-time constant. Cadenza goes one step past *Evidently*'s "constant
offset for a non-polymorphic context": the offset is not an index but a **direct reference to the arm
node**.

Each arm carries its **definition-site** capture: the lexical environment *and the handler-stack depth at
the handle site*. See the under-frame landmine below.

### Tier 1 — tail-resumptive → direct call + `resume`-unwrap + state threading

`perform (E.op a₁ … aₙ)` resolving to a tail arm `(p₁…pₙ) s body`:

1. Bind each `pᵢ ↦ aᵢ` and `s ↦ current-state` — the **exact aliased-local machinery the seed already uses
   for a lambda argument**.
2. Rewrite the arm body: every tail `(resume value next-state)` becomes `value`, and `next-state` is
   threaded forward as the state seen by the rest of the handled region. (`resume` is not a call; it is the
   "return this value to the perform site, carry this state forward" marker, and unwrapping it is the whole
   trick.) When the state kind is `Unit`, threading is a no-op (`Kind::Unit` emits no bytes) — the unit-state
   fast path, byte-identical to a stateless inline.
3. Emit the rewritten body under the arm's definition-site environment **and the handler stack truncated to
   the arm's definition-site depth** (the *under*-frame — see landmine). The result Kind is the operation's
   declared result type through the existing type→Kind path.

Worked shapes:

- `Get.get` → `(() s (resume 41 s))`, state Unit; `(Get.get)` emits `41`; `(+ (Get.get) 1)` const-folds →
  emitted wasm `i64.const 42`. Byte-identical to a stateless handler.
- `Fresh.next` → `((u) s (resume s (+ s 1)))`, state `Int64` seeded `0`; the three performs in the body see
  `0,1,2` (each yields `s`, threads `s+1`), and the `do` yields `i64.const 2` — the state transform doing
  real work.
- `Diag.emit`/`Diag.collect` → `((code) s (resume unit (List.push s code)))` and `((u) s (resume s s))`,
  state a list seeded empty; two emits thread `(list 201 210)` forward, then `collect` reads it out as an
  ordinary operation; the body yields that list. No return clause.

**No continuation object, no stack, no evidence vector.** Two emit strategies chosen by arm size:
**inline** the rewritten body at the perform site (best for small arms — every corpus case), or emit the arm
once as an **ordinary wasm function and `call` it** (bounds code size when an arm is performed at many sites;
the return value is the resume value). Both are stock wasm.

**The state transform is the Tier-1 lowering, not a separate tier.** Because every `handle` threads state,
the state-passing transform over the handled region *is* Tier 1: the `(value, next-state)` pair threads
left-to-right through the continuation, each performed operation reads the current state (the arm's `s`
binder) and `resume value next-state` delivers `value` while carrying `next-state` forward. `get`/`collect`
read the state, `set`/`Fresh.next` update it. The heap stays immutable — the state value is a scalar or an
opaque `Kind::Heap` handle threaded by value. The **unit-state** case (seed `unit`, thread `s` unchanged)
is the degenerate instance: `Kind::Unit` emits no bytes, so it is byte-identical to a stateless inline.
This collapses what an earlier draft split into "pure tail" and a distinct "state-threading" sub-tier —
they are one transform with a zero-cost degenerate case; the non-unit accumulator (the compiler's `Fresh`
counter, `Diag` list, `Unify` store) is where it does real work.

### Tier 2 — abortive → branch to the handler continuation

An arm that never resumes lowers like an exception. Emit the handled `body` inside a wasm `block` whose
result type is the handle's result Kind; lower each perform of an abortive op to `br` to that block's end
carrying the arm's value. No capture, no continuation. (An alternative lowering onto the finished wasm
exception-handling proposal exists, but the `block`/`br` shape needs no proposal and is preferred.) No
corpus case exercises this today; build opportunistically when an exception-shaped effect appears.

### Tier 3 — general one-shot → defunctionalized frame on the value heap (the fallback)

**Trigger:** the classifier assigns an arm to GENERAL ONE-SHOT (a `resume` not in tail position, or a
captured continuation). No corpus case and no self-hosting-driver effect reaches this today; until built it
is a clean decline.

Reify the delimited region between the perform site and its lexically-known handler as a **first-order frame
chain on the existing value-heap runtime** (defunctionalization — the industrial state-machine technique,
and a truer raw-wasm exemplar than Koka's C-via-emscripten path):

- A frame is `sum-new(site-disc, arr-of-captured-locals)` — `site-disc` a compiler-assigned discriminant per
  suspension point; the payload array holds the live locals there. A continuation spanning several suspension
  points is a linked list of such frames.
- A non-tail `perform` builds the frame and calls the statically-known handler arm passing `(args, k)`, `k`
  being the frame handle.
- `resume k v` = `apply(k, v)`, where `apply` is a single compiler-emitted wasm function that reads
  `sum-disc(k)`, `br_table`s to the code for that site, restores captured locals from `sum-payload(k)`, and
  resumes yielding `v`.
- **One-shot** means the frame chain is consumed exactly once — no clone, RC-reclaimed by the runtime's
  existing `drop`. **Multi-shot** (a rare build-level opt-in) copies the frame chain per resume
  (O(frame-depth)); a cost, **not** a soundness break — the "multi-shot breaks RC" claim is overstated.

**Cost:** a frame allocation + `br_table` dispatch per non-tail perform, and a resumption-flag check after
effectful calls on the reified path (cheap, well-predicted). All stock wasm, fully deterministic.

## Composition with the host boundary

**Scenario:** a host-delegated effect (e.g. `ask.ask`, routed to the boundary by the entrypoint's `host`
delegation) is performed *inside* an intra-program handler.

For tiers 1 / 1′ / 2, the intra-program handler left **nothing reified on the wasm stack** (a tail arm was
inlined as a value/direct-call; an abortive arm as a branch). The host call is an ordinary
imported-function call that returns its response inline and the run continues — the entry is a plain
function, no `Suspended` arm. The emitted bytes are identical regardless of how the host resolves the call.
The one guarantee the compiler must uphold is that a host which **re-derives** the run (re-invokes the
entry with the same input, feeding recorded responses in order) reproduces identical behavior: it does, for
free, because **deterministic re-execution re-establishes every lexical intra-program handler context**
(they are static — reconstructed by recomputation) and **nothing about the intra-program continuation is
serialized.** The corpus's `(host-responses …)` fixture is exactly that ordered response sequence; the two
host cases (`ask.ask → 100`; `(+ (ask.ask) (ask.ask)) → 7`) assert determinism in it.

**The compile-time invariant** (checked alongside `CDZ0401`/`CDZ0403`):

> A host-delegated operation may be performed under intra-program handlers **only when every enclosing
> intra-program handler up to the perform is tail-resumptive, state-threading, or abortive** (tiers 1 / 1′ /
> 2). A **reified (Tier-3) intra-program continuation must not span a host call** — a host may re-derive
> across the call, and a chain of non-durable heap handles is not reconstructible from `(input, responses)`.

This falls out of the same classifier, so it is statically enforceable, and today's corpus satisfies it
automatically. It reconciles the two continuation notions: intra-program continuations are ephemeral and
recomputed; a host that re-derives a run reconstructs them by recomputation; the invariant forbids the one
configuration a re-derivation could not reconstruct (an ephemeral continuation spanning a host call).

### Soundness against the durable-replay state of the art (a host re-derivation strategy)

When a host chooses the **re-derivation** strategy (drop the run, re-invoke with recorded responses), it is
running exactly Temporal / Azure Durable Functions / Restate / Unison's triple `(code-version, input,
event-history)`. This strategy is host policy, not a language mandate — but a program that a host *might*
re-derive must not defeat it, so the compiler upholds three pitfalls the durable systems prove exist (all
trivially satisfied by tiers 1–3; a host that only ever answers inline or fiber-suspends is unaffected):

1. **Non-determinism leak.** Any value not a function of `(input, responses)` corrupts re-derivation.
   Cadenza forbids it at emission (`determinism-and-fuel.md` §No Nondeterministic Instruction Is Emitted).
2. **Observation re-fires on re-derivation.** An intra-program `Diag.emit` before a host call re-runs on
   every re-derivation — harmless, because it is *re-derived within the run*, not re-emitted to the host.
   *Obligation:* keep replay-idempotent observation effects as intra-program handlers (they are). **This is
   the load-bearing rule for an interposing handler** (§forwarding): a handler that intercepts a host effect
   to count/log/cache it, then forwards via `(resume (host.op …))`, re-executes its arm on every
   re-derivation because the forwarded host call is where the host may re-derive and everything before it
   recomputes. So the interceptor's *own* side effect must be replay-idempotent — an intra-program
   `State`/`Count` effect (re-derived within the run: the count is correct) — **not** a host counter
   (`metric.incr` would over-count once per re-derivation). `capabilities-and-effects.md` §A Handler May
   Interpose On An Effect An Entrypoint Would Delegate makes this an obligation on the interposing arm. A
   durable host-side count must be idempotent-by-key, a host contract not a language one.
3. **Non-durable handles across a host call** (the load-bearing pitfall). A Tier-3 continuation is a chain
   of non-durable heap handles; it must not span a host call a strategy might re-derive across. Enforced by
   the invariant above.

## ABI / runtime impact

**Tiers 1, 1′, 2 — the intra-program layer — need ZERO ABI/WIT change.** Handler arms are ordinary user
functions or inlined bodies; State threads as extra core params/returns; everything lives in the program's
own call stack + scalars + handles. Intra-program effects do not appear in the manifest and never cross a
boundary. (The host boundary itself is governed by `component-abi.md` v4 — the entry is a plain function,
no `Suspended` arm — and a delegated effect adds imported interfaces to the *program's* world per
`host-interface-binding.md`; both are the host-delegation concern, orthogonal to this intra-program layer.)

**Tier 3: envelope-neutral.** A continuation frame is an ordinary `sum-new(disc, payload)` over an `arr` of
captured locals — all in the **frozen prefix** of the runtime WIT (indices 6–12); `apply` is a
compiler-emitted in-program function; existing `drop`/`reset` reclaim frames. **No new WIT op is required.**
If a dedicated frame representation is ever wanted, it is an **append-only** WIT addition at a new frozen
index (like `bytes-compact` at 36), costing one envelope re-derivation — never a reshuffle. This is the
single biggest architectural win, given that every WIT/envelope touch costs a frozen-envelope re-derivation
(`spec/learnings/2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope.md`).

## Mapping onto the seed compiler

Concrete integration points in `codegen.rs` (a raw-wasm emitter over a `Kind` lattice; lambdas
compile-time-inlined; no runtime closures or funcref tables today):

- **Kind lattice.** No new `Kind` for tiers 1 / 1′ / 2. An operation's result kind is its declared result
  type through the existing type→Kind path; the resume value's kind is the operation's result kind; the
  handle expression's kind is its body's kind. Tier-3 frames are `Kind::Heap`. A `Kind::Cont` refinement of
  `Heap` is optional and only aids Tier-3 type-checking — defer it.
- **Effect-declaration parsing.** Add a parser for `(effect Name (op op-name (-> T… R)) …)` building
  `effect → {op → op-type}` — a routing-agnostic contract, **no `(host)` marker** and no host-bound set at
  the declaration. **Retire `parse_host_imports`**: host-binding is no longer read from a declaration but
  from an entrypoint `(host …)` delegation (below), so the legacy `(import (host …))` surface and the
  declaration-time host set both go away.
- **Form recognition.** Add `handle` **and `host`** to the name-headed dispatch (alongside `do`/`let`/
  `match`) → a `gen_handle` and a `gen_host`. Add `resume`, `handle`, and `host` to the form-keyword set so
  a bare `resume` does not misfire the `CDZ0401` ungranted-effect path. `gen_host` is `gen_handle`'s
  boundary twin: it pushes a delegation frame naming the delegated effects, emits its body, and lowers each
  delegated operation as the existing host-import boundary call; it is admitted **only at an entrypoint**
  (a non-entrypoint `host` form is rejected).
- **Perform interception.** A perform `(E.op args)` reads (via the reader's dotted-name sugar) as a list
  headed by `(. E op)`, which already lands in the `.`-headed arm of the list dispatcher. Add: if `E` is a
  declared effect and `op` one of its operations, route to a `gen_perform` **before** the
  constructor/lambda/dotted-apply checks.
- **Function context.** Add a router stack of classified `HandlerFrame`s and `HostFrame`s. `gen_handle` and
  `gen_host` push, emit body, pop. `gen_perform` resolves top-down: a `HandlerFrame` match inlines (Tier 1,
  reusing the aliased-local + emit path), branches (Tier 2), or reifies-or-declines (Tier 3); a `HostFrame`
  match emits the boundary call. An op that reaches the entrypoint top with **no** matching frame is
  `CDZ0401` (reached-but-no-handler-and-no-delegation); a `host` frame naming an effect no reachable perform
  matches is `CDZ0404` (latent authority); an arm naming an undeclared op is `CDZ0403`.
- **Value-heap runtime component.** Unchanged for tiers 1 / 1′ / 2. Tier-3 frames are ordinary heap values;
  the program threads opaque handles and never dereferences them.
- **Fuel.** None to emit. `determinism-and-fuel.md` §Resource Accounting was retired by constitution
  Amendment 0.7.0; bounding a run is the host's job
  (`spec/learnings/2026-07-06-fuel-is-host-owned-runtime-policy-not-a-compiler-emitted-measure.md`). The
  tail-resumptive path is trivially compatible; do not add a compiler-emitted counter.
- **Diagnostics.** `CDZ0401` (an effect reached the entrypoint top with no handler and no delegation — the
  merged ungranted-effect check, subsuming the former `CDZ0402`), `CDZ0403` (arm names an undeclared op),
  and `CDZ0404` (a delegation names an unreached effect — latent authority) are registered in
  `options/diagnostics-schema/`; wire them in the classifier pass.

## Determinism

Every lowering emits only ordinary deterministic wasm — direct calls, inlined bodies, `br`, `br_table`,
params/returns, and the frozen value-heap ops. No instruction with an unspecified result, no uninitialized
read, no unshared-without-capability thread op (`determinism-and-fuel.md` §No Nondeterministic Instruction Is
Emitted). This is also what makes host-boundary replay reconstruct intra-program handler state exactly,
closing the loop with the host mechanism. A strict advantage over native stack-switching, which buries
control transfer in an engine intrinsic the compiler cannot reason about.

## Staged implementation plan

Earliest-value-first. Each stage names the corpus cases it turns green.

- **Stage 0 — declaration + routing surface + rejections (no runtime lowering).** Parse the
  routing-agnostic `(effect …)`; retire `parse_host_imports`; add `resume`/`handle`/`host` keywords;
  recognize the entrypoint `(host …)` delegation; classify arms; implement `CDZ0401` (merged ungranted
  effect), `CDZ0403`, and `CDZ0404`.
  *Green:* the rejection cases (`CDZ0403`, and the merged-`CDZ0401` no-home case) and the pure/empty-row
  case (`+ 20 22 → 42`). Also stops effect constructs misfiring the syntactic dispatch.
- **Stage 1 — Tier-1 tail-resumptive with state threading (the core).** Lower perform to the
  `resume`-unwrapped, arg-and-state-aliased arm body via the existing inline path; model the *under*-frame;
  thread state (unit-state = zero-cost no-op via the `Kind::Unit` fast path). Cross-function resolution
  comes for free by inlining the (non-recursive) callee into the handled region (§Effect-context
  monomorphization).
  *Green (unit-state):* `Choose.pick → 6`, `Get.get → 42`, `Scale.by → 42`, `Unify/Scope.resolve → 5`.
  *Green (stateful fold):* `Fresh.next ×3 → 2`, `Diag emit/collect → (list 201 210)`.
  *Green (cross-function):* callee-performs-caller-handles → 42, resolve-through-intermediate → 105,
  nearer-shadows → 10, same-fn-two-handlers → 32, deep-chain → 10, cross-fn Fresh counter → (tuple 0 1).
  **Milestone (the #1 self-hosting unblock):** the compiler's own effect algebra — `Fresh` (a real folded
  counter), `Diag` (a real accumulated list), `Unify` (a store) — runs on stock wasm, across function
  boundaries. EFFECTS was the top self-host blocker; this clears it.
- **Stage 1b — direct-call emit for reused/large arms (correctness-neutral).** Emit an arm as a real wasm
  function + `call` when it is performed at multiple sites or is large (returns `(value, next-state)`, or a
  bare `value` when the next-state is Unit; bounds code size).
- **Stage 2 — host-boundary composition.** Wire the entrypoint `(host …)` delegation through the
  host-import lowering — a delegated operation becomes a plain imported-function call (entry stays a plain
  function, no `Suspended` arm) and its interface enters the manifest; add the compile-time invariant.
  *Green:* `ask.ask → 100`; `(+ (ask.ask) (ask.ask)) → 7`; `Scale.by (ask.ask) → 42`; the
  interpose-and-forward case → 7. **Milestone:** in-program handling and host delegation compose, provably,
  with no serialized intra-program state, and a `handle` interposes a delegated effect.
- **Stage 3 — effect-context specialization (only when inlining is insufficient).** When a *recursive*
  effectful function or a same-value-under-two-handlers-at-runtime case appears, emit the function once per
  handler context as a monomorphization (not inlining). Not needed by the current corpus (Stage 1's
  inlining subsumes it); the moment before Tier 3.
- **Stage 4 — Tier-2 abortive (opportunistic).** `block`/`br`; build when an exception-shaped effect appears.
- **Stage 5 — Tier-3 defunctionalized general one-shot (only when triggered).** Reify frames on the frozen
  value heap; emit `apply` as a `br_table` dispatcher; reject multi-shot unless a build's declared default
  enables it (then copy the frame chain).

## Risks and open questions

- **Under-frame miscompile (verified "surprisingly subtle").** A tail arm that itself performs must resolve
  nested performs against its *definition-site* handler stack, not the perform site. *Mitigation:* carry the
  definition-site environment and handler-stack depth on every arm; emit arm bodies with the stack truncated
  to that depth; never do naive textual substitution. Test with a nested same-effect handler before shipping
  Stage 1.
- **Non-conservative classifier → silent miscompile.** *Mitigation:* the tail check is syntactic and exact;
  anything uncertain is GENERAL ONE-SHOT or declines.
- **Corpus/seed surface mismatch (forces a decision).** The corpus declares routing-agnostic `(effect …)`
  and delegates at the entrypoint with `(host …)`; the seed parses the legacy `(import (host …))`.
  *Decision:* migrate to the declaration + entrypoint-delegation surface in Stage 0, retiring
  `parse_host_imports`.
- **Code-size blowup from inlining at many sites.** *Mitigation:* Stage-1b emit-as-function + call.
- **State-threading is real machinery.** *Decision:* keep it a distinct sub-tier.
- **Multi-shot under the affine default** is a use-after-consume. *Mitigation:* statically reject (stronger
  than OCaml 5's dynamic check); admit only where a build's declared default enables it.
- **Deep Tier-3 resumption chains grow the wasm stack.** *Mitigation:* the host bounds it (a clean trap is a
  defined halt); the eventual optimization is emitting `return_call` (Wasm 3.0 tail-call), not on the
  critical path.
- **Open:** introduce `Kind::Cont` for Tier-3 (defer — `Kind::Heap` suffices until a non-tail resume
  appears); surface for the multi-shot opt-in (a build-level declared default, decided when Tier 3 is built);
  interaction with the opt-in effect-row typing layer (orthogonal and meaning-preserving — no action now).
