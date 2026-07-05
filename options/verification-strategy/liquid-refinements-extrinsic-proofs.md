# Verification Strategy — Choice: liquid-refinements-extrinsic-proofs

> **The default choice for the `verification-strategy` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the logic and certificate shape of the
> optional verification layers. Rationale:
> `spec/learnings/2026-07-04-refinements-are-liquid-verification-is-extrinsic.md` and
> `spec/learnings/2026-07-04-fold-order-independence-is-a-verified-property.md`.

## Refinements are liquid types

A refinement constrains a base type with a predicate drawn from a **decidable** logic (liquid types):
linear arithmetic, equality, uninterpreted functions — the fragment an SMT solver decides. The
compiler discharges a refinement obligation by handing the verification condition to a solver and
recording the result as a **reproducibly checkable certificate**, not by trusting the solver inline.
Because the discharge is decidable and certificate-backed, a nondeterministic solver run never enters
the reproducible byte path (constitution VIII): the certificate is checked, and whether it checks is a
deterministic function of the source.

A refinement **erases to its base type** in the emitted component — it carries no runtime
representation and never changes emitted bytes. Adding or removing a refinement to a program that
already compiles is meaning-preserving.

## Machine-checked verification is extrinsic

Full machine-checked proofs are **extrinsic**: they are statements about a program's observable
*behavior*, checked against the executable semantics, not propositions-as-types embedded in the term
language. Keeping verification extrinsic is what lets types be first-class values with `Type : Type`
without the logical inconsistency that intrinsic dependent proof layers would introduce. A proof
obligation the compiler cannot discharge is a rejection (constitution VIII), never silently ignored,
and discharge must be **proof-producing** — it emits a certificate a third party rechecks.

## First load-bearing use: fold order-independence

The first property that actually exercises this layer is the target's **fold order-independence**: a
fold module must produce a byte-identical result regardless of the delivery order of the events it
folds (a CRDT-style commutative / latest-wins convergence property, strictly stronger than purity).
This is discharged by whichever rung fits: property-based testing (permutation invariance —
`property-based-testing.md`), a liquid refinement, or an extrinsic proof. It sits **off** the byte path
— the fold's emitted bytes are identical whether or not order-independence has been discharged — and
the certificate is what an activation review trusts (composes with the fold-purity certificate in
`capabilities-and-effects.md`).
