# Compiling effect handlers: classify first, and the tail-resumptive common case is plain code

*2026-07-06*

**What happened.** Intra-program effects (the `handle` / perform / `resume` layer of
[[2026-07-05-effects-are-declared-with-one-surface-the-declaration-is-the-grant]]) were the single
largest gap between the language and a compiler authored in it — the `DECLINE`-frequency signal from
[[2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps]] put effects at 10, well ahead
of the numeric model, and the seed realizes none of the surface (it still parses only the legacy
`(import (host …))`, so even the two host-replay corpus cases do not parse). `algebraic-one-shot.md` pins
*what* the effects mean, but left *how to transform them into wasm* open. A SOTA research pass closed it:
ten lanes (Koka evidence-passing, Effekt lexical capabilities, OCaml 5 one-shot, selective CPS,
defunctionalized continuations-as-data, the wasm stack-switching proposal, Asyncify, the component-model
async ABI, Temporal/Unison durable replay, and compile-time handler classification), each load-bearing
claim adversarially fact-checked against primary sources, then four candidate lowering architectures
evaluated against Cadenza's exact constraints.

The research converged, unanimously, on a **classification-first** lowering, and the decisive finding is
that **Cadenza's constraints make the shipping surface require zero continuation machinery.** A
compile-time pass sorts each handler arm by how it uses `resume` — **tail-resumptive** / **abortive** /
**general one-shot** — and lowers each to a minimal, stock-wasm shape:

- **Tail-resumptive** (`op ↦ λx.λk. k e`, `k ∉ fv(e)`): perform becomes a direct call to (or inline of) the
  statically-known handler arm, and a tail `(resume e)` becomes just the value `e`. **Every current corpus
  arm — `Choose.pick`, `Get.get`, `Fresh.next`, `Diag.emit`, `Unify/Scope.resolve` — and all three of the
  self-hosting compiler's own effects (`Fresh`, `Diag`, `Unify`) are tail-resumptive.** So the fast path is
  the whole of the initial implementation.
- **Abortive** (never resumes): `block`/`br` out to the handler, like an exception. No corpus case yet.
- **General one-shot** (resume not in tail, or a captured continuation): a defunctionalized frame chain on
  the **existing** value-heap runtime (`sum-new(site-disc, arr-of-locals)` + a `br_table` `apply`). No
  corpus case reaches it; until built, a non-tail `resume` is a clean decline (reject-don't-miscompile).

**Rejected: native stack-switching (`cont.*` / WasmFX)** on three independent, verified grounds — it is not
in any Wasmtime stability tier (WIP x86_64 Cranelift only), it was >4× slower than Asyncify in the only
published numbers, and its continuation is an **opaque native stack that cannot cross the component boundary
or be re-derived as data** (proposal issue #128), which breaks `component-abi.md` §A Durable Continuation Is
Canonical Data. Also rejected: whole-program Asyncify, and selective-CPS-as-primary. Everything adopted
ships in stock WebAssembly today.

**Why.** Cadenza already made the three decisions that collapse the hard cases:

- **Statically-determined handler resolution over a monomorphized closed row** means the handler discharging
  any performed operation is a compile-time constant — no runtime handler search, and Koka's runtime
  *evidence vector* collapses to a **direct reference to the arm node** (one step past *Evidently*'s
  "constant offset for a non-polymorphic context"). Resolution is *dynamic in extent* (a perform can be
  discharged by a caller's handler, not one lexically enclosing the performing definition — the corpus
  cross-function cases pin this) but *statically resolved* by monomorphizing the handler context per call
  site — the compile-time realization of Koka evidence-passing / Effekt capabilities in a raw-wasm emitter.
  (An earlier draft called this "lexical"; the cross-function cases show it is dynamic-in-extent,
  statically determined — the spec heading was corrected to match.)
- **One-shot (affine) continuations** mean the reified-continuation case never copies, and multi-shot is a
  rare, explicit, per-build opt-in (a cost, not a soundness break — the "multi-shot breaks RC" worry is
  overstated).
- **The separate value-heap runtime** means the one case that *does* need a reified continuation reifies it
  as ordinary `sum`/`arr` values in the **frozen WIT prefix** — so even the fallback is **envelope-neutral**,
  needing no new runtime op and no ABI arm. Given that every WIT/envelope touch costs a frozen-envelope
  re-derivation ([[2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope]]), this is the biggest
  architectural win.

The two effect mechanisms have **opposite state ownership** and must never share a continuation object: the
host-bound boundary continuation is canonical **data** `(component, input, log)` and migratable
([[2026-07-05-host-calls-suspend-as-replay-from-the-hosts-log]]); a reified intra-program continuation is a
chain of **non-durable opaque heap handles**, valid within one run and re-derived by replay. A compile-time
invariant (checked with `CDZ0402`/`CDZ0403`) forbids the one configuration that would confuse them: **a
reified general-one-shot continuation must not span a host suspension point.** Because tail-resumptive and
abortive handlers leave *nothing reified on the wasm stack*, a host-bound effect performed inside an
intra-program handler suspends and replays with no serialized intra-program state — deterministic
re-execution re-establishes every lexical handler context for free. This is why the host boundary was sound
all along, and it validates Cadenza against the shipping durable-execution systems (Temporal, Azure Durable
Functions, Restate, Unison) that use the same triple: Cadenza is *more* sound because
`determinism-and-fuel.md` §No Nondeterministic Instruction Is Emitted forbids the non-determinism leak those
systems can only detect at runtime.

The research pass also **corrected a stale assumption in its own framing**: the task treated fuel accounting
as a live compiler obligation, but constitution Amendment 0.7.0 (this same day) retired Core Principle V and
`determinism-and-fuel.md` §Resource Accounting — bounding a run is now host-owned runtime policy
([[2026-07-06-fuel-is-host-owned-runtime-policy-not-a-compiler-emitted-measure]]). So no lowering threads a
compiler-emitted counter, which makes the tail-resumptive path trivially deterministic. The adversarial
fact-checking earned its keep by catching this against a primary source rather than propagating the stale
premise.

**The requirement it drove.** The initial lowering changed no RFC-2119 requirement — it was the
*operational lowering* the existing behavior admits. A follow-on pass (2026-07-06, same day) then found the
behavior was under- and mis-specified in two ways and corrected `capabilities-and-effects.md` itself: (1)
the handler-resolution requirement said "resolved lexically at compile time", which the newly-added
cross-function corpus cases proved wrong — resolution is *dynamic in extent* (a callee's perform is
discharged by a caller's handler) yet statically determined by monomorphization; the heading and sentence
were rewritten to say so; and (2) a new requirement, *A Handler Threads State Across The Operations It
Discharges*, was added — every `handle` seeds an initial state, each arm binds the current state, and
`(resume value next-state)` folds the state across the operations, the handle evaluating to its body's
value (read-out is an ordinary operation, no return clause). The stateless handler is the degenerate
unit-state case (zero-cost, `Kind::Unit` emits no bytes), which collapses the former TailPure/TailState
lowering split into one state-passing transform. The durable output is the concrete design
[`options/effects-model/lowering-to-wasm.md`](../../options/effects-model/lowering-to-wasm.md)
(companion to `algebraic-one-shot.md`): the three-class classifier, the per-class emitted-wasm shapes, the
host-composition invariant, the zero-ABI-impact result, the seed integration points, and a staged plan whose
Stage 1 (Tier-1 tail-resumptive) turns green all five intra-program corpus cases and clears the #1
self-hosting blocker. It composes with [[2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic]]
(the backend the lowering emits into), [[2026-07-04-capabilities-attenuate-a-handler-forwards-a-narrower-row]]
(the row-subset check the same classifier pass performs), and [[2026-07-04-durable-execution-is-effects-plus-determinism]]
(the durable-replay soundness this lowering keeps intact).
