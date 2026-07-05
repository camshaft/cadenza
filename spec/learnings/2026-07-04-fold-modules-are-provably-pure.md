# A module's role fixes its effect profile: a fold is provably pure, and the compiler certifies it

*2026-07-04*

**What happened.** The target defines module **roles** — fold handlers, agent steps, tool executors,
review services — and each role carries a **different, mandatory effect profile**:
- A **fold** module MUST be *pure*: "granted no source of nondeterminism — no clock, no randomness, no
  ambient input/output, no concurrency." Determinism there is achieved "primarily by absence." Its
  effect row must be **empty** (or restricted to reading its granted projections and emitting within its
  namespace — no host nondeterminism).
- An **agent step** quarantines nondeterminism: it reaches its nondeterministic reasoning substrate only
  as a **capability-gated tool call** whose result is recorded, never as in-module nondeterminism
  ([[2026-07-04-durable-execution-is-effects-plus-determinism]]).
- A **tool executor** may hold outward effects, under its owner's identity.

So the language must let a module **declare its role** and have the compiler **prove and certify** that
the module's effect row is within the profile that role permits — most sharply, that a fold's row is
empty.

**Why this is a Cadenza obligation, not just a runtime one.** The target's runtime enforces determinism
"by absence" (it binds a fold none of the nondeterministic operations). But Cadenza is the tool that
turns source into the component, and its whole thesis is **empower the agent to produce a safe module
with zero human feedback** ([[2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program]]).
An agent authoring a fold should not discover at activation time that its fold reached a forbidden
effect — the compiler should **reject it at compile time** with the effect it reached and the route to
remove it, and should **certify the empty-row property** as a machine-readable output the activation
review can trust. Purity becomes a *statically proven, certified* property of the emitted component, not
a runtime hope.

**Why the effects model already carries it.**
- **Purity is the empty effect row over the host vocabulary**
  ([[2026-07-04-the-host-interface-is-the-effect-vocabulary]]). "This module folds purely" is exactly
  "this module's escaping effect row is empty" — a property the type system already computes.
- **Role is a constraint on that row.** Declaring role = fold imposes "row ⊆ {read granted projections,
  emit granted kinds}"; a fold reaching `Blob`/`Invoke`/any nondeterministic effect is a compile-time
  rejection. This is the attenuation subset-check ([[2026-07-04-capabilities-attenuate-a-handler-forwards-a-narrower-row]])
  applied at the module boundary.
- **The certificate is a query-oracle answer.** "Certify this module folds purely" is the compiler
  answering a query about the module's inferred effect row
  ([[2026-07-04-the-compiler-is-a-queryable-oracle]]) and emitting it as machine-readable output — the
  same surface that exposes the inferred manifest.

**Order-independence rides on top (see the companion learning).** A fold must also produce byte-identical
output regardless of delivery order — a *semantic* property beyond purity, discharged by the verification
layers ([[2026-07-04-fold-order-independence-is-a-verified-property]]). Purity is necessary but not
sufficient for order-independence; this learning covers the effect-profile (purity) half.

**Consequences to hold.**
- **A role is a first-class declaration with a checked effect bound.** The module declaration carries
  its role (the target's module-declaration contract already requires this); Cadenza checks the body's
  effect row against the role's permitted profile and rejects on violation.
- **Fold purity composes with compile-time purity.** The compile-time tier is already pure
  ([[2026-07-04-compile-time-evaluation-is-one-tier]]); a fold is the *runtime* analogue — pure at
  phase 0 too. Both are "empty row over the host vocabulary," one mechanism at two phases.
- **The certificate is reproducible.** Like every compiler output it is a deterministic function of
  source (Constitution II), so an activation review re-deriving the module recomputes the same purity
  certificate.

**The requirements it drives.** `spec/capabilities/capabilities-and-effects.md` gains a §"A Module Role
Bounds Its Effect Row": a module declares its role; the compiler MUST reject a module whose effect row
exceeds the profile its role permits (a fold reaching any nondeterministic or ungranted effect is a
compile-time rejection); and the compiler MUST emit the module's inferred effect row as a machine-
readable certificate the activation review can check. `spec/capabilities/capabilities-and-effects.md`
§"The Effect Vocabulary Is The Host Interface" is where the empty-row-is-purity definition lives.
Composes with [[2026-07-04-the-host-interface-is-the-effect-vocabulary]],
[[2026-07-04-durable-execution-is-effects-plus-determinism]], and
[[2026-07-04-fold-order-independence-is-a-verified-property]].
