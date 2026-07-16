# Vertical charter: machine-checked verification (HOL-Light-style, baked into the language)

**Operator directive (2026-07-16, verbatim intent):** "We need to get a vertical thinking about
machine-checked verification. I really like the idea of baking something like HOL-Light into the
language."

## Your mandate
You own a NEW standing vertical: bring **machine-checked verification** into Cadenza, in the spirit of
**HOL-Light**. This is DESIGN-FIRST and exploratory — the operator has named a direction and a north
star (HOL-Light-in-the-language), not a spec. Your first job is to think, scope, and propose — not to
rush code.

## Why HOL-Light is the named model (understand it before designing)
HOL-Light is a theorem prover whose entire trustworthiness rests on a tiny **LCF-style kernel**: a
small set of primitive inference rules + axioms, with an abstract `thm` (theorem) type whose ONLY
constructors are those trusted rules. Everything else — tactics, decision procedures, big libraries — is
ordinary code that can only produce a `thm` by going through the kernel, so a bug in a tactic can never
produce an unsound theorem. The kernel is a few hundred lines; you trust it, and everything built on top
inherits soundness. That LCF discipline (an abstract proof type gated behind a minimal trusted kernel)
is very likely the crux of "baking HOL-Light into the language."

## The core design questions (work these; route genuine forks to the concierge → operator)
1. **What does "baked into the language" MEAN here?** Candidates to weigh:
   - (a) An LCF-style kernel as a Cadenza LIBRARY — an opaque `Thm` type whose constructors are the
     primitive rules, exploiting Cadenza's opaque/abstract types so user code can't forge a `Thm`. This
     is the most natural first fit (leans on features that already exist — opaque types, sums, the
     purity story) and needs the least compiler change. Likely your Increment 0.
   - (b) Verification of CADENZA PROGRAMS themselves — prove properties of actual Cadenza functions
     (pre/postconditions, refinement types, a `@verify`/`@theorem` annotation the compiler checks). Much
     bigger; needs a semantics-to-logic bridge. A later increment / possibly its own design.
   - (c) A reflective/metaprogramming tie — proofs as first-class terms via the quote/`Ast` machinery
     v-metaprogramming built, so tactics can manipulate proof terms. Worth exploring for synergy.
   Decide which is Increment 0 and sequence the rest. My read: START with (a), the LCF kernel as a
   library, because it's the trust foundation everything else needs and it stress-tests the opaque-type
   soundness guarantees (a real language-design win either way).
2. **Soundness boundary:** an LCF kernel's whole value is that `Thm` is UNFORGEABLE. Can Cadenza's
   opaque-type / module-abstraction story actually guarantee no user code fabricates a `Thm` outside the
   kernel module? If there's ANY hole (reflection, decode, `Ast` eval, unsafe host boundary, equality
   tricks), the kernel is worthless. This is the make-or-break — coordinate with v-inference (types),
   v-metaprogramming (quote/eval can it forge?), v-runtime (can a raw heap value be cast to Thm?),
   v-memory-safety. Pin adversarial "try to forge a Thm" cases with the breaker.
3. **The logic:** HOL is classical higher-order logic with a specific term/type structure (simply-typed
   λ-calculus + a few axioms: `REFL`, `BETA`, `ASSUME`, `EQ_MP`, `TRANS`, `MK_COMB`, `ABS`,
   `DEDUCT_ANTISYM_RULE`, + INST/INST_TYPE, + the axioms of infinity/choice/extensionality). Which
   fragment do you target first? A minimal propositional/equational core to prove the mechanism, then
   grow toward full HOL.
4. **Surface + ergonomics:** how does a user WRITE a proof/theorem in Cadenza — a `Thm`-producing
   library API, a tactic combinator language, a dedicated annotation, an embedded-DSL via
   tagged-templates (the metaprogramming mechanism)? Design the ergonomics after the kernel works.

## How to work (per the vertical contract + the operator's design-first framing)
- **Increment 0 = a DESIGN doc**, not code. Write `design/verification-hol-kernel.md` (or similar):
  what "baked in" means, the Increment 0 choice + rationale, the soundness-boundary analysis, the HOL
  fragment targeted first, and the open forks. Route the genuine forks to the concierge (→ operator) —
  this is heavily design-gated and the operator wants to be in the loop on the shape.
- Then increment: kernel skeleton → primitive rules → the unforgeability pins (with breaker) → a first
  proved theorem → grow the logic → ergonomics. Gate every code increment normally (corpus/rcdzc/check).
- COORDINATE early: the soundness boundary touches v-inference / v-metaprogramming / v-runtime /
  v-memory-safety — send them notes when your design leans on their guarantees. Feed v-guide a doc
  suggestion once there's a user-visible surface (this is a flagship "look what Cadenza can do" story,
  like CAD).
- This is a REAL stress test of the language (opaque-type soundness, purity, abstraction) — REPORT/FIX
  language gaps you find, don't work around them, same as the compiler-ml port ethos.

## Not urgent, do it right
The operator framed this as "get a vertical THINKING about" it — so depth over speed. A crisp Increment-0
design doc that nails the soundness-boundary question is worth more than rushed kernel code. Stand as a
strong owner: each tick, advance the design or an increment; if idle, deepen the soundness analysis or
survey the HOL-Light kernel for the next primitive to port.
